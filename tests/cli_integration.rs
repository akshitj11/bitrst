//! Process-level CLI integration tests.

use std::process::Command;

#[test]
fn tip_prints_hash_from_binary() {
    let output = Command::new(env!("CARGO_BIN_EXE_bitrst"))
        .args(["tip", "--network-time", "1231006505"])
        .output()
        .expect("run bitrst tip");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let hash_line = stdout.lines().next().expect("hash line");
    assert_eq!(hash_line.len(), 64);
    assert!(stdout.contains("ephemeral in-memory chain"));
}

#[test]
fn tip_rejects_zero_network_time_from_binary() {
    let output = Command::new(env!("CARGO_BIN_EXE_bitrst"))
        .args(["tip", "--network-time", "0"])
        .output()
        .expect("run bitrst tip");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8");
    assert!(stderr.contains("network time must be greater than zero"));
}

#[test]
fn wallet_new_does_not_leak_secret_by_default() {
    let output = Command::new(env!("CARGO_BIN_EXE_bitrst"))
        .args(["wallet", "new"])
        .output()
        .expect("run wallet new");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("address:"));
    assert!(!stdout.contains("private_key:"));
}

#[test]
fn mine_mines_single_block() {
    let output = Command::new(env!("CARGO_BIN_EXE_bitrst"))
        .args([
            "mine",
            "--count",
            "1",
            "--network-time",
            "1231006505",
            "--time",
            "1231007000",
        ])
        .output()
        .expect("run mine");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("mined block height=1"));
}
