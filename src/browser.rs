// src/browser.rs — Rust 2021: Chromium process launcher + pure-Tokio loopback DevTools discovery (zero reqwest/TLS)
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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

/// Pure-Tokio HTTP GET `/json/version` — parses Content-Length directly (never hangs on HTTP/1.1 keep-alive).
async fn fetch_browser_ws_url_host(host: &str, port: u16) -> Option<String> {
    let host = host.to_string();
    let fut = async move {
        let mut stream = TcpStream::connect((host.as_str(), port)).await.ok()?;
        let req = format!(
            "GET /json/version HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
            host, port
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
    tokio::time::timeout(Duration::from_secs(3), fut).await.ok().flatten()
}

async fn fetch_browser_ws_url(port: u16) -> Option<String> {
    fetch_browser_ws_url_host("127.0.0.1", port).await
}

/// Parses a port number, `host:port`, or `http://host:port` endpoint into `(host, port)`.
pub fn parse_cdp_host_port(endpoint: &str) -> Option<(String, u16)> {
    let s = endpoint.trim();
    if let Ok(p) = s.parse::<u16>() {
        return Some(("127.0.0.1".to_string(), p));
    }
    let stripped = s
        .strip_prefix("http://")
        .or_else(|| s.strip_prefix("https://"))
        .unwrap_or(s);
    let authority = stripped.split('/').next().unwrap_or("");
    let (host, port_str) = authority.rsplit_once(':')?;
    let port = port_str.parse::<u16>().ok()?;
    let host = host.trim_matches(|c| c == '[' || c == ']');
    if host.is_empty() {
        return None;
    }
    Some((host.to_string(), port))
}

/// Resolves a CDP endpoint (`ws://...`, port `"9222"`, `"127.0.0.1:9222"`, or `"http://127.0.0.1:9222"`)
/// into `(browser_ws_url, port)`.
pub async fn resolve_cdp_ws_url(endpoint: &str) -> DriverResult<(String, u16)> {
    let trimmed = endpoint.trim();
    if trimmed.starts_with("ws://") || trimmed.starts_with("wss://") {
        let authority = trimmed
            .split("://")
            .nth(1)
            .unwrap_or("")
            .split('/')
            .next()
            .unwrap_or("");
        let port = authority
            .rsplit_once(':')
            .and_then(|(_, p)| p.parse::<u16>().ok())
            .unwrap_or(0);
        return Ok((trimmed.to_string(), port));
    }
    let (host, port) = parse_cdp_host_port(trimmed).ok_or_else(|| {
        DriverError::InvalidInput(format!(
            "Endpoint CDP không hợp lệ '{}'; hỗ trợ port (9222), http://host:port, hoặc ws://...",
            endpoint
        ))
    })?;
    let ws_url = fetch_browser_ws_url_host(&host, port).await.ok_or_else(|| {
        DriverError::Connection(format!(
            "Không lấy được /json/version từ Chromium tại {}:{}",
            host, port
        ))
    })?;
    Ok((ws_url, port))
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
    let grease = if major.parse::<u32>().map(|v| v % 2 == 0).unwrap_or(false) {
        "24"
    } else {
        "99"
    };
    let grease_full = format!("{grease}.0.0.0");
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
                { "brand": "Google Chrome", "version": major },
                { "brand": "Chromium", "version": major },
                { "brand": "Not)A;Brand", "version": grease }
            ],
            "fullVersionList": [
                { "brand": "Google Chrome", "version": full_ver },
                { "brand": "Chromium", "version": full_ver },
                { "brand": "Not)A;Brand", "version": grease_full }
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

/// Derives the matching `http_tls` BoringSSL impersonation preset (e.g. `chrome_157`, `chrome_149_macos`, `edge_148`)
/// from the live Chromium `User-Agent` and platform string.
pub fn derive_tls_preset(ua: &str, platform: &str) -> String {
    let lower_plat = platform.to_lowercase();
    let os_suffix = if lower_plat.contains("mac") || ua.contains("Mac OS X") {
        "_macos"
    } else if lower_plat.contains("android") || ua.contains("Android") {
        "_android"
    } else if lower_plat.contains("linux") || ua.contains("Linux") {
        "_linux"
    } else {
        ""
    };

    if let Some(after_edge) = ua.split("Edg/").nth(1) {
        let major = after_edge
            .split(['.', ' '])
            .next()
            .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or("148");
        return format!("edge_{major}{os_suffix}");
    }

    let major = ua
        .split("Chrome/")
        .nth(1)
        .and_then(|s| s.split(['.', ' ']).next())
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or("149");
    format!("chrome_{major}{os_suffix}")
}

/// Merges caller `extra_args` into base Chromium flags, combining multiple `--disable-features=` flags
/// into one comma-separated list so Chromium never silently drops earlier disabled features.
fn merge_browser_args(mut base: Vec<String>, extras: Option<Vec<String>>) -> Vec<String> {
    let Some(extras) = extras else {
        return base;
    };
    let mut extra_disabled: Vec<String> = Vec::new();
    let mut passthrough: Vec<String> = Vec::new();
    for arg in extras {
        if let Some(feats) = arg.strip_prefix("--disable-features=") {
            for f in feats.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                extra_disabled.push(f.to_string());
            }
        } else {
            passthrough.push(arg);
        }
    }
    if !extra_disabled.is_empty() {
        if let Some(existing) = base.iter_mut().find(|a| a.starts_with("--disable-features=")) {
            for f in extra_disabled {
                let current = existing.strip_prefix("--disable-features=").unwrap_or("");
                if !current.split(',').any(|x| x == f) {
                    existing.push(',');
                    existing.push_str(&f);
                }
            }
        } else {
            base.push(format!("--disable-features={}", extra_disabled.join(",")));
        }
    }
    base.extend(passthrough);
    base
}

pub struct BrowserProcess {
    pub child: Child,
    pub port: u16,
    pub ws_url: String,
    // Held for RAII: the temp profile is removed when the process struct drops unless detached
    _temp_dir: Option<TempDir>,
    pub user_data_dir: PathBuf,
    detached: Arc<AtomicBool>,
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
        detached: Arc<AtomicBool>,
        start_url: Option<String>,
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
            "--disable-features=IsolateOrigins,site-per-process,TLSTrustAnchorIDs".to_string(),
            "--disable-quic".to_string(),
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

        // Auto-add `--no-sandbox` / `--disable-dev-shm-usage` on Linux containers, root, CI,
        // or Ubuntu 24.04+ hosts where AppArmor restricts unprivileged user namespaces.
        #[cfg(target_os = "linux")]
        {
            let is_container = std::path::Path::new("/.dockerenv").exists()
                || std::path::Path::new("/run/.containerenv").exists();
            let is_ci = std::env::var_os("CI").is_some();
            let is_root = std::fs::read_to_string("/proc/self/status")
                .ok()
                .and_then(|s| {
                    s.lines()
                        .find(|l| l.starts_with("Uid:"))
                        .and_then(|l| l.split_whitespace().nth(1))
                        .map(|uid| uid == "0")
                })
                .unwrap_or(false);
            let apparmor_restricted =
                std::fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
                    .ok()
                    .map(|s| s.trim() == "1")
                    .unwrap_or(false);
            if is_container || is_ci || is_root || apparmor_restricted {
                if !args.iter().any(|a| a == "--no-sandbox") {
                    args.push("--no-sandbox".to_string());
                }
                if !args.iter().any(|a| a == "--disable-dev-shm-usage") {
                    args.push("--disable-dev-shm-usage".to_string());
                }
            }
        }

        args = merge_browser_args(args, extra_args);

        if let Some(url) = start_url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            args.push(url.to_string());
        }

        let mut child = Command::new(&binary)
            .args(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| DriverError::Launch(format!("Lỗi khi khởi chạy process trình duyệt: {}", e)))?;

        let mut browser_ws_url = String::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Some(ws) = fetch_browser_ws_url(port).await {
                browser_ws_url = ws;
                break;
            }
            if let Ok(Some(status)) = child.try_wait() {
                return Err(DriverError::Launch(format!(
                    "Trình duyệt thoát sớm trước khi mở cổng CDP (status={})",
                    status
                )));
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
            detached,
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
        if self.detached.load(Ordering::SeqCst) {
            // Preserve the profile directory on disk while Chromium keeps running independently
            if let Some(td) = self._temp_dir.take() {
                let _ = td.keep();
            }
        } else {
            self.kill();
        }
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

    #[test]
    fn derive_tls_preset_matches_browser_and_os() {
        assert_eq!(
            derive_tls_preset(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/157.0.0.0 Safari/537.36",
                "Windows"
            ),
            "chrome_157"
        );
        assert_eq!(
            derive_tls_preset(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/149.0.0.0 Safari/537.36",
                "macOS"
            ),
            "chrome_149_macos"
        );
        assert_eq!(
            derive_tls_preset(
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/145.0.0.0 Safari/537.36",
                "Linux"
            ),
            "chrome_145_linux"
        );
        assert_eq!(
            derive_tls_preset(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/148.0.0.0 Safari/537.36 Edg/148.0.0.0",
                "Windows"
            ),
            "edge_148"
        );
    }

    #[test]
    fn merge_browser_args_combines_disable_features() {
        let base = vec![
            "--disable-features=IsolateOrigins,TLSTrustAnchorIDs".to_string(),
            "--disable-quic".to_string(),
        ];
        let merged = merge_browser_args(
            base,
            Some(vec![
                "--disable-features=TLSTrustAnchorIDs,Translate".to_string(),
                "--lang=en-US".to_string(),
            ]),
        );
        assert_eq!(
            merged,
            vec![
                "--disable-features=IsolateOrigins,TLSTrustAnchorIDs,Translate".to_string(),
                "--disable-quic".to_string(),
                "--lang=en-US".to_string(),
            ]
        );
    }

    #[test]
    fn parse_cdp_host_port_handles_all_formats() {
        assert_eq!(parse_cdp_host_port("9222"), Some(("127.0.0.1".to_string(), 9222)));
        assert_eq!(parse_cdp_host_port("http://127.0.0.1:9222"), Some(("127.0.0.1".to_string(), 9222)));
        assert_eq!(parse_cdp_host_port("http://localhost:9333/json/version"), Some(("localhost".to_string(), 9333)));
        assert_eq!(parse_cdp_host_port("127.0.0.1:9444"), Some(("127.0.0.1".to_string(), 9444)));
        assert_eq!(parse_cdp_host_port("not-valid"), None);
    }
}
