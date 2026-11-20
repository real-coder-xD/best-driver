// src/browser.rs — Rust 2021: Chromium process launcher + pure-Tokio loopback DevTools discovery (zero reqwest/TLS)
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use serde::Deserialize;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::error::{DriverError, DriverResult};

#[derive(Deserialize, Debug, Clone)]
struct VersionInfo {
    #[serde(rename = "webSocketDebuggerUrl")]
    web_socket_debugger_url: Option<String>,
}

/// Pure-Tokio loopback HTTP GET `/json/version` — parses Content-Length directly (never hangs on HTTP/1.1 keep-alive).
async fn fetch_browser_ws_url(port: u16) -> Option<String> {
    let fut = async {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.ok()?;
        let req = format!(
            "GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            port
        );
        stream.write_all(req.as_bytes()).await.ok()?;

        let mut buf = Vec::with_capacity(2048);
        let mut tmp = [0u8; 1024];
        loop {
            let n = stream.read(&mut tmp).await.ok()?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(hdr_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&buf[..hdr_end]);
                let content_len = headers.lines().find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    if k.trim().eq_ignore_ascii_case("content-length") {
                        v.trim().parse::<usize>().ok()
                    } else {
                        None
                    }
                });
                let body_bytes = &buf[hdr_end + 4..];
                if let Some(cl) = content_len {
                    if body_bytes.len() >= cl {
                        let body = std::str::from_utf8(&body_bytes[..cl]).ok()?;
                        let info: VersionInfo = serde_json::from_str(body.trim()).ok()?;
                        return info.web_socket_debugger_url;
                    }
                } else if let Ok(body) = std::str::from_utf8(body_bytes) {
                    if let (Some(s), Some(e)) = (body.find('{'), body.rfind('}')) {
                        if let Ok(info) = serde_json::from_str::<VersionInfo>(&body[s..=e]) {
                            return info.web_socket_debugger_url;
                        }
                    }
                }
            }
        }
        let text = String::from_utf8_lossy(&buf);
        let body = text.split("\r\n\r\n").nth(1)?;
        let start = body.find('{')?;
        let end = body.rfind('}')?;
        let info: VersionInfo = serde_json::from_str(&body[start..=end]).ok()?;
        info.web_socket_debugger_url
    };
    tokio::time::timeout(Duration::from_secs(2), fut).await.ok().flatten()
}

/// Builds `Emulation.setUserAgentOverride` params when Chrome reports `HeadlessChrome/`, aligning UA + Sec-CH-UA.
pub fn build_headless_ua_override(raw_ua: &str) -> Option<Value> {
    if !raw_ua.contains("HeadlessChrome/") {
        return None;
    }
    let clean_ua = raw_ua.replace("HeadlessChrome/", "Chrome/");
    let full_ver = clean_ua
        .split("Chrome/")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .unwrap_or("130.0.0.0")
        .to_string();
    let major = full_ver.split('.').next().unwrap_or("130").to_string();
    let (platform, platform_ver, arch) = if clean_ua.contains("Windows") {
        ("Windows", "10.0.0", "x86")
    } else if clean_ua.contains("Mac OS X") {
        ("macOS", "14.0.0", "arm")
    } else {
        ("Linux", "6.5.0", "x86")
    };
    Some(json!({
        "userAgent": clean_ua,
        "userAgentMetadata": {
            "brands": [
                { "brand": "Not(A:Brand", "version": "99" },
                { "brand": "Google Chrome", "version": major },
                { "brand": "Chromium", "version": major }
            ],
            "fullVersionList": [
                { "brand": "Not(A:Brand", "version": "99.0.0.0" },
                { "brand": "Google Chrome", "version": full_ver },
                { "brand": "Chromium", "version": full_ver }
            ],
            "fullVersion": full_ver,
            "platform": platform,
            "platformVersion": platform_ver,
            "architecture": arch,
            "model": "",
            "mobile": false,
            "bitness": "64",
            "wow64": false
        }
    }))
}

pub struct BrowserProcess {
    pub child: Child,
    pub port: u16,
    pub ws_url: String,
    // Held for RAII: the temp profile is removed when the process struct drops
    _temp_dir: Option<TempDir>,
    pub user_data_dir: PathBuf,
}

impl BrowserProcess {
    pub fn find_chrome(custom_path: Option<&str>) -> DriverResult<PathBuf> {
        if let Some(p) = custom_path {
            let path = PathBuf::from(p);
            if path.exists() {
                return Ok(path);
            }
            return Err(DriverError::Launch(format!(
                "Đường dẫn Chrome chỉ định không tồn tại: {}",
                p
            )));
        }

        let candidates = [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ];

        for path in &candidates {
            let p = PathBuf::from(path);
            if p.exists() {
                return Ok(p);
            }
        }

        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let user_chrome = PathBuf::from(local_app_data)
                .join(r"Google\Chrome\Application\chrome.exe");
            if user_chrome.exists() {
                return Ok(user_chrome);
            }
        }

        which::which("chrome")
            .or_else(|_| which::which("chromium"))
            .or_else(|_| which::which("google-chrome"))
            .or_else(|_| which::which("msedge"))
            .map_err(|_| {
                DriverError::Launch("Không tìm thấy trình duyệt Chromium trên hệ thống".to_string())
            })
    }

    pub fn get_free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .unwrap_or(9222)
    }

    pub async fn launch(
        headless: bool,
        user_data_dir: Option<String>,
        browser_path: Option<String>,
        extra_args: Option<Vec<String>>,
    ) -> DriverResult<Self> {
        let binary = Self::find_chrome(browser_path.as_deref())?;
        let port = Self::get_free_port();

        let (temp_dir, data_dir) = match user_data_dir {
            Some(d) => (None, PathBuf::from(d)),
            None => {
                let tmp = tempfile::Builder::new()
                    .prefix("best_driver_profile_")
                    .tempdir()
                    .map_err(|e| DriverError::Io(format!("Không thể tạo temp profile: {}", e)))?;
                let path = tmp.path().to_path_buf();
                (Some(tmp), path)
            }
        };

        let mut args = vec![
            format!("--remote-debugging-port={}", port),
            format!("--user-data-dir={}", data_dir.display()),
            "--no-first-run".to_string(),
            "--no-default-browser-check".to_string(),
            "--test-type".to_string(),
            "--disable-infobars".to_string(),
            "--silent-debugger-extension-api".to_string(),
            "--disable-blink-features=AutomationControlled".to_string(),
            "--disable-features=IsolateOrigins,site-per-process".to_string(),
            "--disable-ipc-flooding-protection".to_string(),
            "--password-store=basic".to_string(),
            "--use-mock-keychain".to_string(),
            "--disable-popup-blocking".to_string(),
            "--disable-prompt-on-repost".to_string(),
            "--disable-background-networking".to_string(),
            "--disable-component-update".to_string(),
            "--disable-domain-reliability".to_string(),
            "--disable-sync".to_string(),
            "--window-size=1280,800".to_string(),
        ];

        if headless {
            args.push("--headless=new".to_string());
        }

        if let Some(extras) = extra_args {
            args.extend(extras);
        }

        args.push("about:blank".to_string());

        let child = Command::new(&binary)
            .args(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| DriverError::Launch(format!("Lỗi khi khởi chạy process trình duyệt: {}", e)))?;

        let mut browser_ws_url = String::new();
        for _ in 0..80 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Some(ws) = fetch_browser_ws_url(port).await {
                browser_ws_url = ws;
                break;
            }
        }

        if browser_ws_url.is_empty() {
            let mut proc = child;
            let _ = proc.kill();
            let _ = proc.wait();
            return Err(DriverError::Timeout(
                "Timeout khi kết nối remote debugging port của Chrome".to_string(),
            ));
        }

        Ok(Self {
            child,
            port,
            ws_url: browser_ws_url,
            _temp_dir: temp_dir,
            user_data_dir: data_dir,
        })
    }

    /// Checks whether the Chromium child process is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Waits briefly for Chromium to exit cleanly (flushing profile SQLite/cookies), then force-kills and reaps.
    pub fn wait_graceful_or_kill(&mut self, max_wait: Duration) {
        let deadline = std::time::Instant::now() + max_wait;
        while std::time::Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => break,
            }
        }
        self.kill();
    }

    /// Kill and reap so Chrome releases the profile directory before it is deleted.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for BrowserProcess {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_ua_override_strips_headless_and_sets_metadata() {
        let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/157.0.0.0 Safari/537.36";
        let ov = build_headless_ua_override(ua).expect("should override headless UA");
        assert_eq!(
            ov["userAgent"].as_str().unwrap(),
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Safari/537.36"
        );
        assert_eq!(ov["userAgentMetadata"]["platform"].as_str().unwrap(), "Windows");
        assert_eq!(ov["userAgentMetadata"]["fullVersion"].as_str().unwrap(), "157.0.0.0");
    }

    #[test]
    fn headful_ua_is_not_overridden() {
        let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Safari/537.36";
        assert!(build_headless_ua_override(ua).is_none());
    }
}
