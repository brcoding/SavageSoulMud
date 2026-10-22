import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest

RUNNER = Path(__file__).with_name("notify_runner.py")
FIXTURE = r'''
import os, pathlib, signal, socket, subprocess, sys, time
state = pathlib.Path(sys.argv[1]); stage = sys.argv[2]
def notify(pid):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
    s.connect(os.environ["NOTIFY_SOCKET"])
    s.send(("MAINPID=%d\nREADY=1" % pid).encode()); s.close()
notify(os.getpid())
if stage == "crash":
    sys.exit(7)
if stage == "initial":
    (state/"initial.pid").write_text(str(os.getpid()))
    child = subprocess.Popen([sys.executable, __file__, str(state), "successor"])
    for _ in range(100):
        if (state/"successor.pid").exists(): break
        time.sleep(.01)
    notify(child.pid)
    sys.exit(0)
def stop(sig, frame):
    (state/"stopped").write_text(str(os.getpid()))
    sys.exit(0)
signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)
(state/"successor.pid").write_text(str(os.getpid()))
while True: time.sleep(.1)
'''


def wait_for(check, timeout=8):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if value := check():
            return value
        time.sleep(.025)
    raise AssertionError("fixture deadline exceeded")


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux subreaper and SCM_CREDENTIALS test")
class RunnerTests(unittest.TestCase):
    def launch(self, directory, stage):
        state = Path(directory)
        fixture = state / "fixture.py"
        fixture.write_text(FIXTURE)
        state.chmod(0o700)
        process = subprocess.Popen([sys.executable, str(RUNNER), "--state", str(state),
                                    "--ready-timeout", "5", "--stop-timeout", "3", "--",
                                    sys.executable, str(fixture), str(state), stage],
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.addCleanup(lambda: process.poll() is None and process.kill())
        return state, process

    def test_copyover_adoption_spoof_rejection_and_signal_forward(self):
        with tempfile.TemporaryDirectory() as directory:
            state, process = self.launch(directory, "initial")
            def successor_is_current():
                try:
                    current = (state / "mud.pid").read_text().strip()
                    return current if current == (state / "successor.pid").read_text().strip() else None
                except OSError:
                    return None
            successor = int(wait_for(successor_is_current))
            initial = int((state / "initial.pid").read_text())
            self.assertNotEqual(initial, successor)
            wait_for(lambda: not Path(f"/proc/{initial}").exists())
            self.assertIsNone(process.poll(), "predecessor clean exit must not exit runner")
            spoof = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
            spoof.connect(str(state / "notify.sock"))
            spoof.send(f"MAINPID={os.getpid()}\nREADY=1".encode())
            spoof.send(b"x" * 8192)
            spoof.close()
            time.sleep(.15)
            self.assertEqual(int((state / "mud.pid").read_text()), successor)
            process.send_signal(signal.SIGTERM)
            stdout, stderr = process.communicate(timeout=8)
            self.assertEqual(process.returncode, 0, stderr)
            self.assertEqual(int((state / "stopped").read_text()), successor)
            self.assertFalse((state / "mud.pid").exists())
            self.assertFalse(Path(f"/proc/{successor}").exists())

    def test_crash_exit_code_reaches_upstart(self):
        with tempfile.TemporaryDirectory() as directory:
            _state, process = self.launch(directory, "crash")
            _stdout, stderr = process.communicate(timeout=8)
            self.assertEqual(process.returncode, 7, stderr)


if __name__ == "__main__":
    unittest.main()
