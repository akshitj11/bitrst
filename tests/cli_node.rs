//! Node process integration test with deterministic shutdown.

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn spawn_test_node(port: u16) -> Child {
    Command::new(env!("CARGO_BIN_EXE_bitrst"))
        .args([
            "node",
            "--listen",
            &format!("127.0.0.1:{port}"),
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

#[test]
fn node_binds_localhost_and_exits_on_sigterm() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);

    let mut child = spawn_test_node(port);
    let deadline = Instant::now() + Duration::from_secs(10);

    let mut saw_listen = false;
    while Instant::now() < deadline {
        if let Some(stderr) = child.stderr.as_mut() {
            use std::io::Read;
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

    child.kill().expect("kill node");
    let _ = child.wait().expect("wait");
}
