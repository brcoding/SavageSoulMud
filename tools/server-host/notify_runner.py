#!/usr/bin/env python3
"""Stable Upstart child which follows GRIM copyover's sd_notify MAINPID changes.

Linux/Python stdlib only. Run as the dedicated game UID, never as a root proxy.
"""
import argparse
import ctypes
import fcntl
import logging
import os
from pathlib import Path
import select
import signal
import socket
import stat
import struct
import subprocess
import sys
import tempfile
import time

LOG = logging.getLogger("grim-notify-runner")
MAX_NOTIFY = 4096


def subreaper():
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(36, 1, 0, 0, 0) != 0:  # PR_SET_CHILD_SUBREAPER, Linux >=3.4
        raise OSError(ctypes.get_errno(), "PR_SET_CHILD_SUBREAPER failed")


def parent_pid(pid):
    try:
        text = Path(f"/proc/{pid}/stat").read_text()
        return int(text[text.rindex(")") + 2:].split()[1])
    except (OSError, ValueError, IndexError):
        return None


def descendant(pid, ancestor):
    seen = set()
    for _ in range(64):
        if pid == ancestor:
            return True
        if pid is None or pid <= 1 or pid in seen:
            break
        seen.add(pid)
        pid = parent_pid(pid)
    return False


def expected_process(pid, expected_exe, uid):
    try:
        if Path(f"/proc/{pid}").stat().st_uid != uid:
            return False
        actual = os.readlink(f"/proc/{pid}/exe")
        # Atomic binary replacement leaves the predecessor's old inode marked
        # deleted; its path still identifies the same supervised application.
        actual = actual.removesuffix(" (deleted)")
        return os.path.realpath(actual) == str(expected_exe)
    except OSError:
        return False


def private_directory(path):
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = path.lstat()
    if (not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid()
            or info.st_mode & 0o077 or path.resolve() != path):
        raise ValueError("runner state must be a private, owned, canonical directory")


def write_pid(path, pid):
    fd, temporary = tempfile.mkstemp(prefix=".mud-pid-", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            stream.write(f"{pid}\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


class Runner:
    def __init__(self, state, pid_file, command, expected_exe, stop_timeout=45, ready_timeout=90):
        self.state = state
        self.pid_file = pid_file
        self.command = command
        self.expected_exe = expected_exe
        self.stop_timeout = stop_timeout
        self.ready_timeout = ready_timeout
        self.current = None
        self.group = None
        self.stopping = None
        self.ready = False
        self.received_signal = None

    def signal(self, signum, _frame):
        if signum in (signal.SIGTERM, signal.SIGINT):
            if self.stopping is None:
                self.stopping = time.monotonic()
            self.received_signal = signum
        if self.current:
            try:
                os.kill(self.current, signum)
            except ProcessLookupError:
                pass

    def notification(self, data, credentials):
        sender, uid, _gid = credentials
        if uid != os.getuid() or not descendant(sender, os.getpid()):
            return
        # A predecessor may already be a zombie when its queued packet arrives.
        # It is still unreaped/current, so its credential PID cannot be reused.
        if sender != self.current and not expected_process(sender, self.expected_exe, uid):
            return
        try:
            fields = dict(line.split("=", 1) for line in data.decode("utf-8").splitlines() if "=" in line)
            target = int(fields.get("MAINPID", self.current or 0))
        except (UnicodeError, ValueError):
            return
        if (target <= 1 or not descendant(target, os.getpid())
                or not expected_process(target, self.expected_exe, uid)):
            return
        if target != self.current:
            self.current = target
            write_pid(self.pid_file, target)
            LOG.info("Copyover MAINPID is now %s", target)
            if self.stopping is not None:
                self.signal(self.received_signal or signal.SIGTERM, None)
        if fields.get("READY") == "1":
            if not self.ready:
                LOG.info("MUD ready, PID %s", target)
            self.ready = True

    def drain(self, notify):
        # Bound a flood's CPU/memory and still reap/forward stop signals promptly.
        for _ in range(64):
            try:
                data, ancillary, flags, _addr = notify.recvmsg(MAX_NOTIFY, socket.CMSG_SPACE(12))
            except BlockingIOError:
                return
            if flags & (socket.MSG_TRUNC | socket.MSG_CTRUNC):
                continue
            credentials = next((struct.unpack("3i", value[:12]) for level, kind, value in ancillary
                                if level == socket.SOL_SOCKET and kind == socket.SCM_CREDENTIALS and len(value) >= 12), None)
            if credentials:
                self.notification(data, credentials)

    def run(self):
        subreaper()
        private_directory(self.state)
        if self.pid_file.parent != self.state:
            raise ValueError("PID file must live directly in the private runner state directory")
        lock = open(self.state / "runner.lock", "a")
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        os.chmod(self.state / "runner.lock", 0o600)
        notify_path = self.state / "notify.sock"
        if len(os.fsencode(notify_path)) >= 108:
            raise ValueError("notify socket path exceeds Linux Unix socket limit")
        if notify_path.exists() or notify_path.is_symlink():
            notify_path.unlink()
        notify = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
        notify.setsockopt(socket.SOL_SOCKET, socket.SO_PASSCRED, 1)
        notify.bind(str(notify_path))
        os.chmod(notify_path, 0o600)
        notify.setblocking(False)
        env = os.environ.copy()
        env["NOTIFY_SOCKET"] = str(notify_path)
        # Never carry accidental predecessor state into a fresh Upstart launch.
        env.pop("GRIM_COPYOVER_SOCK", None)
        initial = subprocess.Popen(self.command, env=env, start_new_session=True)
        self.current = self.group = initial.pid
        try:
            write_pid(self.pid_file, self.current)
        except BaseException:
            # Failed startup bookkeeping must not leave a game running when
            # Upstart retries the wrapper (for example after a full disk).
            os.killpg(self.group, signal.SIGKILL)
            initial.wait()
            notify.close()
            notify_path.unlink(missing_ok=True)
            raise
        LOG.info("Started MUD PID %s", self.current)
        for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGUSR2):
            signal.signal(sig, self.signal)
        started = time.monotonic()
        code = 1
        try:
            while True:
                select.select([notify], [], [], 0.1)
                # Drain MAINPID before reaping: a successful predecessor sends
                # its update before exit, and must not look like a service stop.
                self.drain(notify)
                exited = {}
                while True:
                    try:
                        info = os.waitid(os.P_ALL, 0, os.WEXITED | os.WNOHANG | os.WNOWAIT)
                    except ChildProcessError:
                        break
                    if info is None:
                        break
                    # Keep the zombie identifiable until its final queued
                    # MAINPID notification has been authenticated. Exit may
                    # have raced the first drain above.
                    self.drain(notify)
                    pid, status = os.waitpid(info.si_pid, 0)
                    exited[pid] = os.waitstatus_to_exitcode(status)
                    if pid == initial.pid:
                        initial.returncode = exited[pid]
                if self.current in exited:
                    code = exited[self.current]
                    code = code if code >= 0 else 128 - code
                    if self.stopping is not None:
                        code = 0
                    break
                if not self.ready and time.monotonic() - started > self.ready_timeout:
                    LOG.error("MUD did not announce readiness before startup deadline")
                    break
                if self.stopping is not None and time.monotonic() - self.stopping > self.stop_timeout:
                    LOG.error("MUD exceeded stop deadline")
                    code = 0
                    break
        finally:
            # Successors share the original process group even after adoption.
            # Do not leave a failed/half-started successor holding a listener.
            try:
                os.killpg(self.group, signal.SIGTERM)
            except ProcessLookupError:
                pass
            end = time.monotonic() + 2
            while time.monotonic() < end:
                try:
                    pid, _ = os.waitpid(-1, os.WNOHANG)
                    if pid == 0:
                        time.sleep(0.05)
                except ChildProcessError:
                    break
            try:
                os.killpg(self.group, signal.SIGKILL)
            except ProcessLookupError:
                pass
            notify.close()
            notify_path.unlink(missing_ok=True)
            self.pid_file.unlink(missing_ok=True)
            lock.close()
        return code


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", type=Path, required=True)
    parser.add_argument("--pid-file", type=Path)
    parser.add_argument("--expected-exe", type=Path)
    parser.add_argument("--stop-timeout", type=float, default=45)
    parser.add_argument("--ready-timeout", type=float, default=90)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("a foreground game executable is required after --")
    state = args.state.absolute()
    pid_file = args.pid_file.absolute() if args.pid_file else state / "mud.pid"
    expected = (args.expected_exe or Path(command[0])).resolve(strict=True)
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
    try:
        return Runner(state, pid_file, command, expected, max(1, args.stop_timeout), max(1, args.ready_timeout)).run()
    except (OSError, ValueError):
        LOG.exception("Notify runner could not supervise the game")
        return 1


if __name__ == "__main__":
    sys.exit(main())
