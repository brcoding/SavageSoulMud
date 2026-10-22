//! Real-network copyover (hot restart) test.
//!
//! Unlike the message-level harness in `scenarios.rs`, copyover cannot be
//! exercised in-process: it forks + execs the compiled binary and passes live
//! socket fds over a unix socket (SCM_RIGHTS). So this test spawns the real
//! `copyover_fixture` binary, drives it as a telnet client would, triggers a
//! copyover with `SIGUSR2`, and asserts the *same* TCP connection survives and
//! resumes the character in the room it walked to — no re-login. Nothing more.
//!
//! Unix-only: copyover relies on POSIX signals + fd passing.
#![cfg(unix)]

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A password that satisfies validation.
const PW: &str = "secretpw";

/// Read from `stream` until `needle` appears in the accumulated output or
/// `timeout` elapses. Returns everything read (lossy UTF-8, telnet IAC bytes and
/// all). Panics on timeout, dumping what was seen — that is the failure signal.
fn expect(stream: &mut TcpStream, needle: &str, timeout: Duration) -> String {
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let deadline = Instant::now() + timeout;
    let mut acc = String::new();
    let mut buf = [0u8; 4096];
    while Instant::now() < deadline {
        match stream.read(&mut buf) {
            Ok(0) => break, // peer closed
            Ok(n) => {
                acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                if acc.contains(needle) {
                    return acc;
                }
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => panic!("read error waiting for {needle:?}: {e}"),
        }
    }
    panic!("timed out waiting for {needle:?}; saw:\n{acc}");
}

fn send(stream: &mut TcpStream, line: &str) {
    stream.write_all(line.as_bytes()).unwrap();
    stream.write_all(b"\n").unwrap();
    stream.flush().unwrap();
}

/// Connect with retries while the fixture's listener comes up.
fn connect(port: u16) -> TcpStream {
    let addr = ("127.0.0.1", port)
        .to_socket_addrs()
        .unwrap()
        .next()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match TcpStream::connect_timeout(&addr, Duration::from_millis(300)) {
            Ok(s) => return s,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => panic!("fixture never accepted connections: {e}"),
        }
    }
}

/// Wait until the pidfile holds a PID different from `prev` (the successor of a
/// copyover has taken over and recorded itself), then return it. Mirrors how the
/// real deploy learns the current server via systemd's MainPID.
fn wait_for_new_pid(pidfile: &std::path::Path, prev: u32) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if let Ok(s) = std::fs::read_to_string(pidfile) {
            if let Ok(pid) = s.trim().parse::<u32>() {
                if pid != prev {
                    return pid;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("pidfile never advanced past {prev}");
}

/// `SIGKILL` the whole process group (fixture + any copyover successor it forked)
/// so no server outlives the test.
fn kill_group(pgid: u32) {
    let _ = Command::new("kill")
        .arg("-KILL")
        .arg(format!("-{pgid}"))
        .status();
}

#[allow(clippy::too_many_lines)]
// reason: real-process/socket integration test; one linear scenario
// `cfg(coverage)` is set by cargo-llvm-cov, not Cargo: declare the use
// expected so `-D warnings` stays green under `make coverage`.
#[allow(unexpected_cfgs)]
#[test]
fn copyover_keeps_player_connected_and_resumes_last_room() {
    // This spawns real server processes and drives them over real sockets.
    // Under `cargo llvm-cov` the parallel run starves the fork+exec handoff,
    // so skip when built with `cfg(coverage)` — CI runs this file separately,
    // non-instrumented. Coverage of the copyover code itself is not lost
    // meaningfully (a SIGKILLed successor never flushes its profile anyway;
    // the fd framing is unit-tested).
    if cfg!(coverage) {
        eprintln!("skipping copyover IT under llvm-cov; see CI integration step");
        return;
    }
    // Unique port + isolated data dir per run.
    let port: u16 = 40000 + (std::process::id() % 10000) as u16;
    let dir = std::env::temp_dir().join(format!("grim-copyover-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Run from a *copy* of the binary so we can replace it before the copyover,
    // reproducing a real deploy: the swap unlinks the running image, so the
    // successor's exec path must survive the `(deleted)` marker Linux reports.
    let bin = dir.join("grim-server");
    std::fs::copy(env!("CARGO_BIN_EXE_copyover_fixture"), &bin).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let pidfile = dir.join("server.pid");
    let mut child: Child = Command::new(&bin)
        .env("GRIM_TEST_PORT", port.to_string())
        .env("GRIM_TEST_DATA", &dir)
        .env("GRIM_TEST_PIDFILE", &pidfile)
        // Area blueprints load from the filesystem — point at the repo's
        // committed data/areas. The successor inherits this env across copyover.
        .env(
            "GRIM_AREAS_DIR",
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/areas"),
        )
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Own process group so we can reap the fixture *and* its successor.
        .process_group(0)
        .spawn()
        .expect("spawn copyover_fixture");
    let pgid = child.id();

    // Guard so a panic mid-test still tears the server down.
    struct Cleanup(u32);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            kill_group(self.0);
        }
    }
    let _cleanup = Cleanup(pgid);

    let mut stream = connect(port);

    // ── Create an account + character, enter the world (in the tavern) ──
    expect(
        &mut stream,
        "Character name or account email",
        Duration::from_secs(20),
    );
    send(&mut stream, "alice@example.com");
    expect(&mut stream, "create an account", Duration::from_secs(20));
    send(&mut stream, "y");
    expect(&mut stream, "Choose a password", Duration::from_secs(20));
    send(&mut stream, PW);
    // Account created → character menu. Create a character.
    expect(&mut stream, "character", Duration::from_secs(20));
    send(&mut stream, "c");
    expect(
        &mut stream,
        "name for your new character",
        Duration::from_secs(20),
    );
    send(&mut stream, "Alice");
    // Name accepted → pick gender, race, class (by name/slug).
    expect(&mut stream, "Choose a gender", Duration::from_secs(20));
    send(&mut stream, "neutral");
    expect(&mut stream, "Choose a race", Duration::from_secs(20));
    send(&mut stream, "human");
    expect(&mut stream, "Choose a class", Duration::from_secs(20));
    send(&mut stream, "warrior");
    // MOTD → press enter to enter the world → land in the starting tavern.
    std::thread::sleep(Duration::from_millis(300));
    send(&mut stream, "");
    expect(&mut stream, "Exits:", Duration::from_secs(20));

    // ── Walk north into the Town Square (in-game; respect command cooldown) ──
    std::thread::sleep(Duration::from_millis(1500));
    send(&mut stream, "north");
    expect(&mut stream, "Town Square", Duration::from_secs(20));

    // ── Swap the binary on disk (as a deploy would), then trigger copyover ──
    // Overwrite via a temp file + rename so the running image's inode is unlinked
    // — this is what makes `current_exe()` report a `(deleted)` path.
    let staged = dir.join("grim-server.new");
    std::fs::copy(env!("CARGO_BIN_EXE_copyover_fixture"), &staged).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::rename(&staged, &bin).unwrap();

    let status = Command::new("kill")
        .arg("-USR2")
        .arg(pgid.to_string())
        .status()
        .expect("send SIGUSR2");
    assert!(status.success(), "kill -USR2 failed");

    // The predecessor exits on a successful handoff; reap it so it isn't a
    // zombie. Bound the wait — an unbounded `child.wait()` hangs the runner
    // forever with no output if the handoff stalls (seen under llvm-cov
    // starvation on CI).
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait().expect("reap predecessor") {
            Some(_) => break,
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            None => panic!("predecessor never exited after SIGUSR2"),
        }
    }

    // ── Same socket, no re-login: greeted by the reload and back in the Square ──
    expect(&mut stream, "world was reloaded", Duration::from_secs(60));

    std::thread::sleep(Duration::from_millis(1500));
    send(&mut stream, "look");
    let after = expect(&mut stream, "Town Square", Duration::from_secs(20));
    assert!(
        !after.contains("Rusted Anvil"),
        "should have resumed in the Town Square (walked-to room), not the tavern"
    );

    // ── Second, chained copyover (successor → successor) ──
    // The original process is gone; signal the whole process group so whichever
    // process is currently serving receives it.
    let staged2 = dir.join("grim-server.new2");
    std::fs::copy(env!("CARGO_BIN_EXE_copyover_fixture"), &staged2).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged2, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::rename(&staged2, &bin).unwrap();
    // Signal the *current* server directly (the 1st copyover's successor, which
    // recorded its PID), not the process group — group signals don't reliably
    // reach a reparented successor on CI, and the real deploy signals MainPID.
    let server_pid = wait_for_new_pid(&pidfile, pgid);
    let status = Command::new("kill")
        .arg("-USR2")
        .arg(server_pid.to_string())
        .status()
        .expect("send SIGUSR2 to current server");
    assert!(status.success(), "second kill -USR2 failed");
    expect(&mut stream, "world was reloaded", Duration::from_secs(60));
    std::thread::sleep(Duration::from_millis(1500));
    send(&mut stream, "look");
    expect(&mut stream, "Town Square", Duration::from_secs(20));

    // ── `quit` after a copyover must still close the connection ──
    std::thread::sleep(Duration::from_millis(1500));
    send(&mut stream, "quit");
    // Server-side close → our reads return 0 (EOF) within a moment.
    stream
        .set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut closed = false;
    let mut buf = [0u8; 1024];
    while Instant::now() < deadline {
        match stream.read(&mut buf) {
            Ok(0) => {
                closed = true;
                break;
            }
            Ok(_) => {}
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => {
                closed = true;
                break;
            }
        }
    }
    assert!(
        closed,
        "`quit` after a (chained) copyover must disconnect the socket — a \
         non-CLOEXEC fd inherited across the fork+exec previously kept it open"
    );

    // _cleanup drops here, killing the successor.
    let _ = std::fs::remove_dir_all(&dir);
}
