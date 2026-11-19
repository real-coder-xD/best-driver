// src/browser.rs
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use serde::Deserialize;
use tempfile::TempDir;

/// Loopback DevTools calls must never go through HTTP(S)_PROXY from the environment.
fn http_client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap_or_default()
}

#[derive(Deserialize, Debug, Clone)]
pub struct TargetInfo {
    pub id: String,
    #[serde(rename = "type")]
    pub target_type: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    pub web_socket_debugger_url: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct VersionInfo {
    #[serde(rename = "webSocketDebuggerUrl")]
    pub web_socket_debugger_url: Option<String>,
}

pub struct BrowserProcess {
    pub child: Child,
    pub port: u16,
    // Held for RAII: the temp profile is removed when the process struct drops
    _temp_dir: Option<TempDir>,
    pub user_data_dir: PathBuf,
}

impl BrowserProcess {
    pub fn find_chrome(custom_path: Option<&str>) -> Result<PathBuf, String> {
        if let Some(p) = custom_path {
            let path = PathBuf::from(p);
            if path.exists() {
                return Ok(path);
            }
            return Err(format!("Đường dẫn Chrome chỉ định không tồn tại: {}", p));
        }

        let candidates = [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
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
            .map_err(|_| "Không tìm thấy trình duyệt Chromium trên hệ thống".to_string())
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
    ) -> Result<Self, String> {
        let binary = Self::find_chrome(browser_path.as_deref())?;
        let port = Self::get_free_port();

        let (temp_dir, data_dir) = match user_data_dir {
            Some(d) => (None, PathBuf::from(d)),
            None => {
                let tmp = tempfile::Builder::new()
                    .prefix("best_driver_profile_")
                    .tempdir()
                    .map_err(|e| format!("Không thể tạo temp profile: {}", e))?;
                let path = tmp.path().to_path_buf();
                (Some(tmp), path)
            }
        };

        let mut args = vec![
            format!("--remote-debugging-port={}", port),
            format!("--user-data-dir={}", data_dir.display()),
            "--no-first-run".to_string(),
            "--no-default-browser-check".to_string(),
            // Tắt thanh cảnh báo "unsupported command-line flag" của Chromium
            "--test-type".to_string(),
            "--disable-infobars".to_string(),
            "--silent-debugger-extension-api".to_string(),
            // Evasion flags cốt lõi như nodriver
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

        // Mở trang ban đầu
        args.push("about:blank".to_string());

        let child = Command::new(&binary)
            .args(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Lỗi khi khởi chạy process trình duyệt: {}", e))?;

        let client = http_client();
        let version_url = format!("http://127.0.0.1:{}/json/version", port);
        let mut browser_ws_url = String::new();

        // Chờ DevTools endpoint phản hồi (tối đa 8 giây)
        for _ in 0..80 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Ok(res) = client.get(&version_url).send().await {
                if let Ok(info) = res.json::<VersionInfo>().await {
                    if let Some(ws) = info.web_socket_debugger_url {
                        browser_ws_url = ws;
                        break;
                    }
                }
            }
        }

        if browser_ws_url.is_empty() {
            let mut proc = child;
            let _ = proc.kill();
            return Err("Timeout khi kết nối remote debugging port của Chrome".to_string());
        }

        Ok(Self {
            child,
            port,
            _temp_dir: temp_dir,
            user_data_dir: data_dir,
        })
    }

    pub async fn list_targets(&self) -> Result<Vec<TargetInfo>, String> {
        let client = http_client();
        let url = format!("http://127.0.0.1:{}/json/list", self.port);
        let res = client.get(&url).send().await
            .map_err(|e| format!("Lỗi HTTP json/list: {}", e))?;
        let targets = res.json::<Vec<TargetInfo>>().await
            .map_err(|e| format!("Lỗi parse targets: {}", e))?;
        Ok(targets)
    }

    pub async fn new_tab_target(&self, url: &str) -> Result<TargetInfo, String> {
        let client = http_client();
        let endpoint = format!("http://127.0.0.1:{}/json/new?{}", self.port, url);
        let res = client.put(&endpoint).send().await
            .map_err(|e| format!("Lỗi tạo tab mới qua CDP: {}", e))?;
        let target = res.json::<TargetInfo>().await
            .map_err(|e| format!("Lỗi parse new tab target: {}", e))?;
        Ok(target)
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
