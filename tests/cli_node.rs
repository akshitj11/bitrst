//! Node process integration test with graceful shutdown.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn spawn_test_node() -> Child {
    Command::new(env!("CARGO_BIN_EXE_bitrst"))
        .args([
            "node",
            "--listen",
            "127.0.0.1:0",
            "--network",
            "testnet",
            "--no-connect-seeds",
            "--max-inbound",
            "2",
            "--max-outbound",
            "0",
            "--network-time",
            "1231006505",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn node")
}

#[cfg(unix)]
fn signal_graceful_shutdown(child: &Child) {
    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("send SIGTERM");
    assert!(status.success(), "kill -TERM failed");
}

#[test]
#[cfg(unix)]
fn node_binds_localhost_and_exits_gracefully_on_sigterm() {
    let mut child = spawn_test_node();
    let deadline = Instant::now() + Duration::from_secs(10);

    let mut saw_listen = false;
    while Instant::now() < deadline {
        if let Some(stderr) = child.stderr.as_mut() {
            let mut buf = [0u8; 256];
            if let Ok(n) = stderr.read(&mut buf) {
                let text = String::from_utf8_lossy(&buf[..n]);
                if text.contains("listening on") {
                    saw_listen = true;
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(saw_listen, "node did not report listening address");

    signal_graceful_shutdown(&child);
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status,
            None if Instant::now() >= deadline => panic!("node did not exit after SIGTERM"),
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    assert!(status.success(), "expected graceful exit, got {status:?}");
}

#[test]
#[cfg(not(unix))]
fn node_starts_on_supported_platforms() {
    let mut child = spawn_test_node();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut saw_listen = false;
    while Instant::now() < deadline {
        if let Some(stderr) = child.stderr.as_mut() {
            let mut buf = [0u8; 256];
            if let Ok(n) = stderr.read(&mut buf) {
                let text = String::from_utf8_lossy(&buf[..n]);
                if text.contains("listening on") {
                    saw_listen = true;
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(saw_listen, "node did not report listening address");
    let _ = child.kill();
    let _ = child.wait();
}
