#![cfg(unix)]

use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind free port");
    listener.local_addr().expect("free port addr").port()
}

fn wait_for_cdp(port: u16) {
    for _ in 0..50 {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = stream.write_all(
                b"GET /json/list HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            );
            let mut body = String::new();
            if stream.read_to_string(&mut body).is_ok() && body.contains("200 OK") {
                return;
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("Chrome CDP endpoint did not become ready on port {port}");
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
#[ignore = "launches a real Chrome with remote debugging; run manually for attached-CDP regressions"]
fn attached_cdp_uses_existing_page_instead_of_opening_blank_page() {
    let chrome = match gsd_browser_common::chrome::find_chrome(None) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("skipping attached CDP test; Chrome not available: {err}");
            return;
        }
    };

    let home = unique_temp_home("attached-cdp");
    let profile = home.join("chrome-profile");
    fs::create_dir_all(&profile).expect("create chrome profile");
    let port = free_port();
    let page_url = "data:text/html,<title>Attached CDP Page</title><button>ready</button>";

    let mut chrome_child = Command::new(chrome)
        .arg(format!("--remote-debugging-port={port}"))
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--disable-background-networking")
        .arg("--disable-breakpad")
        .arg("--disable-component-update")
        .arg("--disable-default-apps")
        .arg("--disable-domain-reliability")
        .arg("--disable-extensions")
        .arg("--disable-features=MediaRouter,OptimizationHints")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(page_url)
        .spawn()
        .expect("launch Chrome with remote debugging");

    let session = format!("attached-cdp-{}", std::process::id());
    let result = std::panic::catch_unwind(|| {
        wait_for_cdp(port);

        let base_args = [
            "--session",
            &session,
            "--cdp-url",
            &format!("http://127.0.0.1:{port}"),
            "--json",
        ];

        let list_output = Command::new(env!("CARGO_BIN_EXE_gsd-browser"))
            .env("HOME", &home)
            .args(base_args)
            .arg("list-pages")
            .output()
            .expect("run list-pages");
        assert!(
            list_output.status.success(),
            "list-pages failed: stdout={} stderr={}",
            String::from_utf8_lossy(&list_output.stdout),
            String::from_utf8_lossy(&list_output.stderr)
        );
        let list_result: Value =
            serde_json::from_slice(&list_output.stdout).expect("parse list-pages JSON");
        assert_eq!(list_result["count"], 1);
        assert_eq!(list_result["pages"][0]["title"], "Attached CDP Page");
        assert!(
            list_result["pages"][0]["url"]
                .as_str()
                .unwrap_or_default()
                .starts_with("data:text/html"),
            "expected attached data URL, got {}",
            list_result["pages"][0]["url"]
        );

        let eval_output = Command::new(env!("CARGO_BIN_EXE_gsd-browser"))
            .env("HOME", &home)
            .args(base_args)
            .args(["eval", "document.title"])
            .output()
            .expect("run eval");
        assert!(
            eval_output.status.success(),
            "eval failed: stdout={} stderr={}",
            String::from_utf8_lossy(&eval_output.stdout),
            String::from_utf8_lossy(&eval_output.stderr)
        );
        let eval_result: Value =
            serde_json::from_slice(&eval_output.stdout).expect("parse eval JSON");
        assert_eq!(eval_result["result"], "\"Attached CDP Page\"");
    });

    let _ = Command::new(env!("CARGO_BIN_EXE_gsd-browser"))
        .env("HOME", &home)
        .args(["--session", &session, "daemon", "stop"])
        .output();
    stop_child(&mut chrome_child);
    let _ = fs::remove_dir_all(&home);

    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}
