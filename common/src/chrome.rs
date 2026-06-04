use std::path::PathBuf;

/// Discover Chrome/Chromium binary path.
///
/// Priority:
/// 1. Explicit override (--browser-path)
/// 2. Platform-specific standard paths
/// 3. PATH search via `which`
pub fn find_chrome(override_path: Option<&str>) -> Result<PathBuf, String> {
    // 1. Explicit override
    if let Some(path) = override_path {
        let p = PathBuf::from(path);
        if p.exists() {
            return Ok(p);
        }
        return Err(format!(
            "Chrome not found at specified path: {}. Verify the path exists.",
            path
        ));
    }

    // 2. Platform-specific standard paths
    let platform_paths: Vec<PathBuf> = if cfg!(target_os = "macos") {
        vec![PathBuf::from(
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        )]
    } else if cfg!(target_os = "linux") {
        [
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium-browser",
            "/usr/bin/chromium",
            "/snap/bin/chromium",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    } else if cfg!(target_os = "windows") {
        let mut paths = Vec::new();
        for env_name in ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"] {
            if let Some(base) = std::env::var_os(env_name) {
                let base = PathBuf::from(base);
                paths.push(base.join(r"Google\Chrome\Application\chrome.exe"));
                paths.push(base.join(r"Microsoft\Edge\Application\msedge.exe"));
            }
        }
        paths
    } else {
        Vec::new()
    };

    for path in platform_paths {
        if path.exists() {
            return Ok(path);
        }
    }

    // 3. PATH search
    let candidates: &[&str] = if cfg!(target_os = "windows") {
        &["chrome", "chrome.exe", "msedge", "msedge.exe", "chromium"]
    } else {
        &[
            "google-chrome",
            "google-chrome-stable",
            "chromium-browser",
            "chromium",
        ]
    };
    for name in candidates {
        if let Ok(path) = which::which(name) {
            return Ok(path);
        }
    }

    Err(
        "Chrome not found. Install Google Chrome or pass --browser-path /path/to/chrome"
            .to_string(),
    )
}
