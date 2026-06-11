#![cfg(unix)]

use gsd_browser_common::session::{SessionHealthStatus, SessionManifest};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_home(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    PathBuf::from(format!(
        "/tmp/gb-{test_name}-{}-{nanos}",
        std::process::id()
    ))
}

fn session_dir(home: &Path, session: &str) -> PathBuf {
    home.join(".gsd-browser").join("sessions").join(session)
}

/// Create a session dir holding an unhealthy manifest with no live daemon.
fn write_unhealthy_session(home: &Path, session: &str) -> PathBuf {
    let dir = session_dir(home, session);
    fs::create_dir_all(&dir).expect("create session dir");

    let mut manifest = SessionManifest::default();
    manifest.session_name = Some(session.to_string());
    manifest.health = SessionHealthStatus::Unhealthy;
    manifest.health_reason = "broken browser connection".to_string();
    manifest.socket_path = dir.join("daemon.sock").to_string_lossy().to_string();
    fs::write(
        dir.join("session.json"),
        serde_json::to_string_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");
    dir
}

/// Run a browser command against the session with a bogus browser path, so
/// any attempted daemon start fails fast instead of launching Chrome.
fn run_back_command(home: &Path, session: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_gsd-browser"))
        .env("HOME", home)
        .args([
            "--session",
            session,
            "--browser-path",
            "/definitely/missing/chrome",
            "back",
        ])
        .output()
        .expect("run gsd-browser")
}

#[test]
fn daemon_health_reports_unhealthy_for_stale_pid_and_dead_socket() {
    let home = unique_temp_home("stale-health");
    let session = "stale-health";
    let dir = session_dir(&home, session);
    fs::create_dir_all(&dir).expect("create session dir");

    fs::write(dir.join("daemon.pid"), "999999").expect("write stale pid");
    fs::write(dir.join("daemon.sock"), "").expect("write dead socket placeholder");

    let output = Command::new(env!("CARGO_BIN_EXE_gsd-browser"))
        .env("HOME", &home)
        .args(["--session", session, "--json", "daemon", "health"])
        .output()
        .expect("run daemon health");

    assert!(
        output.status.success(),
        "daemon health failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let result: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse daemon health JSON");
    assert_eq!(result["session"]["status"], "unhealthy");
    assert!(
        result["session"]["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("socket exists without a live daemon PID"),
        "unexpected reason: {}",
        result["session"]["reason"]
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn named_session_self_heals_unhealthy_manifest_when_daemon_pid_is_dead() {
    let home = unique_temp_home("stale-self-heal");
    let session = "stale-self-heal";
    write_unhealthy_session(&home, session);

    let output = run_back_command(&home, session);

    // No live daemon exists, so the stale unhealthy manifest must be
    // self-healed and the start attempted (which then fails only because the
    // browser path is bogus).
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cleaned up stale state, continuing start"),
        "stderr did not report self-heal of stale session: {stderr}"
    );
    assert!(
        !stderr.contains("Refusing to replace it automatically"),
        "stale session with dead daemon should not be refused: {stderr}"
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn named_session_refuses_implicit_replacement_when_daemon_pid_is_alive() {
    let home = unique_temp_home("live-refusal");
    let session = "live-refusal";
    let dir = write_unhealthy_session(&home, session);
    // Point the PID file at this test process so the daemon looks alive.
    fs::write(dir.join("daemon.pid"), std::process::id().to_string()).expect("write live pid");

    let output = run_back_command(&home, session);

    assert!(
        !output.status.success(),
        "command unexpectedly succeeded: stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Refusing to replace a live browser session"),
        "stderr did not contain live-session refusal: {stderr}"
    );
    assert!(
        !stderr.contains("cleaned up stale state"),
        "client must not self-heal while the daemon PID is alive: {stderr}"
    );

    let _ = fs::remove_dir_all(&home);
}
