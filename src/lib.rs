// src/lib.rs — Rust 2024, PyO3 0.30 + pyo3-async-runtimes: Python bindings for best_driver
use std::collections::HashMap;
use std::sync::Arc;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use serde_json::Value;
use tokio::sync::Mutex;

mod browser;
mod cdp;
mod tab;

use browser::BrowserProcess;
use tab::Tab;

// ───────────────────────────── conversions ─────────────────────────────

fn rt(e: String) -> PyErr {
    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e)
}

fn value_to_py<'py>(py: Python<'py>, val: &Value) -> PyResult<Bound<'py, PyAny>> {
    Ok(match val {
        Value::Null => py.None().into_bound(py),
        Value::Bool(b) => b.into_pyobject(py)?.to_owned().into_any(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into_pyobject(py)?.into_any()
            } else if let Some(u) = n.as_u64() {
                u.into_pyobject(py)?.into_any()
            } else {
                n.as_f64().unwrap_or(f64::NAN).into_pyobject(py)?.into_any()
            }
        }
        Value::String(s) => s.into_pyobject(py)?.into_any(),
        Value::Array(arr) => {
            let list = PyList::empty(py);
            for item in arr {
                list.append(value_to_py(py, item)?)?;
            }
            list.into_any()
        }
        Value::Object(map) => {
            let dict = PyDict::new(py);
            for (k, v) in map {
                dict.set_item(k, value_to_py(py, v)?)?;
            }
            dict.into_any()
        }
    })
}

/// Python object → serde Value through the stdlib json module (dict/list/str/num/bool/None).
fn py_to_value(obj: &Bound<'_, PyAny>) -> PyResult<Value> {
    let dumped: String = obj.py().import("json")?.call_method1("dumps", (obj,))?.extract()?;
    serde_json::from_str(&dumped).map_err(|e| rt(e.to_string()))
}

struct PyJson(Value);
impl<'py> IntoPyObject<'py> for PyJson {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;
    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        value_to_py(py, &self.0)
    }
}

struct PyBin(Vec<u8>);
impl<'py> IntoPyObject<'py> for PyBin {
    type Target = PyBytes;
    type Output = Bound<'py, PyBytes>;
    type Error = PyErr;
    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        Ok(PyBytes::new(py, &self.0))
    }
}

/// Clone the Arc into an async block and hand it to asyncio.
macro_rules! fut {
    ($py:expr, $arc:expr, |$t:ident| $body:expr) => {{
        let $t = Arc::clone(&$arc);
        pyo3_async_runtimes::tokio::future_into_py($py, async move { $body.map_err(rt) })
    }};
}

// ───────────────────────────── browser core ─────────────────────────────

struct ProxyAuth {
    user: String,
    pass: String,
}

/// "scheme://user:pass@host:port" → ("scheme://host:port", Some(auth))
fn split_proxy(raw: &str) -> (String, Option<ProxyAuth>) {
    let (scheme, rest) = raw.split_once("://").unwrap_or(("http", raw));
    match rest.rsplit_once('@') {
        Some((creds, hostport)) => {
            let (u, p) = creds.split_once(':').unwrap_or((creds, ""));
            (format!("{}://{}", scheme, hostport), Some(ProxyAuth { user: u.to_string(), pass: p.to_string() }))
        }
        None => (format!("{}://{}", scheme, rest), None),
    }
}

pub struct BrowserCore {
    process: Mutex<Option<BrowserProcess>>,
    tabs: Mutex<HashMap<String, Arc<Tab>>>,
    proxy_auth: Option<ProxyAuth>,
    port: u16,
    user_data_dir: String,
}

impl BrowserCore {
    async fn launch(
        headless: bool,
        user_data_dir: Option<String>,
        browser_path: Option<String>,
        extra_args: Option<Vec<String>>,
        proxy: Option<String>,
    ) -> Result<Self, String> {
        let mut args = extra_args.unwrap_or_default();
        let mut proxy_auth = None;
        if let Some(p) = proxy {
            let (server, auth) = split_proxy(&p);
            args.push(format!("--proxy-server={}", server));
            proxy_auth = auth;
        }
        let proc = BrowserProcess::launch(headless, user_data_dir, browser_path, Some(args)).await?;
        let port = proc.port;
        let user_data_dir = proc.user_data_dir.to_string_lossy().to_string();
        Ok(Self {
            process: Mutex::new(Some(proc)),
            tabs: Mutex::new(HashMap::new()),
            proxy_auth,
            port,
            user_data_dir,
        })
    }

    async fn attach(&self, target_id: String, ws: &str) -> Result<Arc<Tab>, String> {
        let tab = Tab::new(target_id.clone(), ws).await?;
        if let Some(a) = &self.proxy_auth {
            tab.enable_proxy_auth(a.user.clone(), a.pass.clone()).await?;
        }
        let tab = Arc::new(tab);
        self.tabs.lock().await.insert(target_id, Arc::clone(&tab));
        Ok(tab)
    }

    /// Page targets in browser order; reuses existing connections, drops closed ones.
    async fn list_tabs(&self) -> Result<Vec<Arc<Tab>>, String> {
        let targets = {
            let lock = self.process.lock().await;
            lock.as_ref().ok_or("Browser đã đóng")?.list_targets().await?
        };
        let pages: Vec<_> = targets.into_iter().filter(|t| t.target_type == "page").collect();
        {
            let mut cache = self.tabs.lock().await;
            cache.retain(|id, _| pages.iter().any(|t| &t.id == id));
        }
        let mut out = Vec::with_capacity(pages.len());
        for t in pages {
            let cached = self.tabs.lock().await.get(&t.id).cloned();
            match cached {
                Some(tab) => out.push(tab),
                None => {
                    if let Some(ws) = t.web_socket_debugger_url {
                        out.push(self.attach(t.id, &ws).await?);
                    }
                }
            }
        }
        Ok(out)
    }

    async fn main_tab(&self) -> Result<Arc<Tab>, String> {
        match self.list_tabs().await?.into_iter().next() {
            Some(t) => Ok(t),
            None => self.new_tab("about:blank").await,
        }
    }

    async fn new_tab(&self, url: &str) -> Result<Arc<Tab>, String> {
        // Open blank first so evasion + proxy auth are in place before the first request
        let target = {
            let lock = self.process.lock().await;
            lock.as_ref().ok_or("Browser đã đóng")?.new_tab_target("about:blank").await?
        };
        let ws = target.web_socket_debugger_url.ok_or("Target không có WebSocket debugger URL")?;
        let tab = self.attach(target.id, &ws).await?;
        if url != "about:blank" {
            tab.get(url, None, "load").await?;
        }
        Ok(tab)
    }

    async fn stop(&self) {
        self.tabs.lock().await.clear();
        if let Some(mut proc) = self.process.lock().await.take() {
            proc.kill();
        }
    }
}

// ───────────────────────────── Tab ─────────────────────────────

#[pyclass(name = "Tab")]
pub struct PyTab {
    inner: Arc<Tab>,
}

#[pymethods]
impl PyTab {
    fn __repr__(&self) -> String {
        format!("<Tab target_id={}>", self.inner.target_id)
    }

    #[getter]
    fn target_id(&self) -> String {
        self.inner.target_id.clone()
    }

    // navigation
    #[pyo3(signature = (url, timeout_ms=None, wait_until="load".to_string()))]
    fn get<'py>(&self, py: Python<'py>, url: String, timeout_ms: Option<u64>, wait_until: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.get(&url, timeout_ms, &wait_until).await)
    }

    #[pyo3(signature = (state="load".to_string(), timeout_ms=30000))]
    fn wait_for_load<'py>(&self, py: Python<'py>, state: String, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.wait_for_load(&state, timeout_ms).await)
    }

    #[pyo3(signature = (ignore_cache=false, timeout_ms=30000))]
    fn reload<'py>(&self, py: Python<'py>, ignore_cache: bool, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.reload(ignore_cache, timeout_ms).await)
    }

    #[pyo3(signature = (timeout_ms=30000))]
    fn back<'py>(&self, py: Python<'py>, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.back(timeout_ms).await)
    }

    #[pyo3(signature = (timeout_ms=30000))]
    fn forward<'py>(&self, py: Python<'py>, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.forward(timeout_ms).await)
    }

    fn bring_to_front<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.bring_to_front().await)
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.close().await)
    }

    // reading
    fn evaluate<'py>(&self, py: Python<'py>, expression: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.evaluate(&expression).await.map(PyJson))
    }

    fn content<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.content().await)
    }

    fn title<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.title().await)
    }

    fn url<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.current_url().await)
    }

    fn exists<'py>(&self, py: Python<'py>, selector: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.exists(&selector).await)
    }

    fn count<'py>(&self, py: Python<'py>, selector: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.count(&selector).await)
    }

    fn text<'py>(&self, py: Python<'py>, selector: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.text(&selector).await)
    }

    fn text_all<'py>(&self, py: Python<'py>, selector: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.text_all(&selector).await)
    }

    fn attr<'py>(&self, py: Python<'py>, selector: String, name: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.attr(&selector, &name).await)
    }

    fn attr_all<'py>(&self, py: Python<'py>, selector: String, name: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.attr_all(&selector, &name).await)
    }

    fn value<'py>(&self, py: Python<'py>, selector: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.value(&selector).await)
    }

    fn html<'py>(&self, py: Python<'py>, selector: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.inner_html(&selector).await)
    }

    // waiting
    #[pyo3(signature = (selector, timeout_ms=10000, visible=false))]
    fn wait_for<'py>(&self, py: Python<'py>, selector: String, timeout_ms: u64, visible: bool) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.wait_for_selector(&selector, timeout_ms, visible).await)
    }

    #[pyo3(signature = (selector, timeout_ms=10000))]
    fn wait_for_hidden<'py>(&self, py: Python<'py>, selector: String, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.wait_for_hidden(&selector, timeout_ms).await)
    }

    #[pyo3(signature = (text, timeout_ms=10000))]
    fn wait_for_text<'py>(&self, py: Python<'py>, text: String, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.wait_for_text(&text, timeout_ms).await)
    }

    #[pyo3(signature = (contains, timeout_ms=10000))]
    fn wait_for_url<'py>(&self, py: Python<'py>, contains: String, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.wait_for_url(&contains, timeout_ms).await)
    }

    #[pyo3(signature = (expression, timeout_ms=10000, poll_ms=100))]
    fn wait_for_function<'py>(&self, py: Python<'py>, expression: String, timeout_ms: u64, poll_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.wait_for_function(&expression, timeout_ms, poll_ms).await.map(PyJson))
    }

    // mouse
    #[pyo3(signature = (selector, button="left".to_string(), click_count=1, timeout_ms=10000))]
    fn click<'py>(&self, py: Python<'py>, selector: String, button: String, click_count: i64, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.click(&selector, &button, click_count, timeout_ms).await)
    }

    #[pyo3(signature = (selector, timeout_ms=10000))]
    fn double_click<'py>(&self, py: Python<'py>, selector: String, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.click(&selector, "left", 2, timeout_ms).await)
    }

    #[pyo3(signature = (selector, timeout_ms=10000))]
    fn right_click<'py>(&self, py: Python<'py>, selector: String, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.click(&selector, "right", 1, timeout_ms).await)
    }

    #[pyo3(signature = (text, exact=false, timeout_ms=10000))]
    fn click_text<'py>(&self, py: Python<'py>, text: String, exact: bool, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.click_text(&text, exact, timeout_ms).await)
    }

    #[pyo3(signature = (x, y, button="left".to_string(), click_count=1))]
    fn mouse_click<'py>(&self, py: Python<'py>, x: f64, y: f64, button: String, click_count: i64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.mouse_click(x, y, &button, click_count).await)
    }

    fn mouse_move<'py>(&self, py: Python<'py>, x: f64, y: f64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.mouse_move(x, y).await)
    }

    #[pyo3(signature = (selector, timeout_ms=10000))]
    fn hover<'py>(&self, py: Python<'py>, selector: String, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.hover(&selector, timeout_ms).await)
    }

    #[pyo3(signature = (from_selector, to_selector, timeout_ms=10000))]
    fn drag<'py>(&self, py: Python<'py>, from_selector: String, to_selector: String, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.drag(&from_selector, &to_selector, timeout_ms).await)
    }

    // keyboard
    #[pyo3(signature = (selector, timeout_ms=10000))]
    fn focus<'py>(&self, py: Python<'py>, selector: String, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.focus(&selector, timeout_ms).await)
    }

    #[pyo3(signature = (selector, timeout_ms=10000))]
    fn clear<'py>(&self, py: Python<'py>, selector: String, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.clear(&selector, timeout_ms).await)
    }

    #[pyo3(signature = (selector, text, clear=false, delay_ms=60, timeout_ms=10000))]
    fn send_keys<'py>(&self, py: Python<'py>, selector: String, text: String, clear: bool, delay_ms: u64, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.send_keys(&selector, &text, clear, delay_ms, timeout_ms).await)
    }

    #[pyo3(signature = (text, delay_ms=60))]
    fn type_text<'py>(&self, py: Python<'py>, text: String, delay_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.type_text(&text, delay_ms).await)
    }

    #[pyo3(signature = (selector, text, timeout_ms=10000))]
    fn fill<'py>(&self, py: Python<'py>, selector: String, text: String, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.fill(&selector, &text, timeout_ms).await)
    }

    fn press_key<'py>(&self, py: Python<'py>, key: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.press_key(&key).await)
    }

    // forms
    #[pyo3(signature = (selector, value, timeout_ms=10000))]
    fn select_option<'py>(&self, py: Python<'py>, selector: String, value: String, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.select_option(&selector, &value, timeout_ms).await)
    }

    #[pyo3(signature = (selector, timeout_ms=10000))]
    fn check<'py>(&self, py: Python<'py>, selector: String, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.set_checked(&selector, true, timeout_ms).await)
    }

    #[pyo3(signature = (selector, timeout_ms=10000))]
    fn uncheck<'py>(&self, py: Python<'py>, selector: String, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.set_checked(&selector, false, timeout_ms).await)
    }

    /// `files` may be a single path or a list of paths.
    #[pyo3(signature = (selector, files, timeout_ms=10000))]
    fn upload_file<'py>(&self, py: Python<'py>, selector: String, files: &Bound<'py, PyAny>, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        let list: Vec<String> = match files.extract::<String>() {
            Ok(one) => vec![one],
            Err(_) => files.extract()?,
        };
        fut!(py, self.inner, |t| t.upload_file(&selector, &list, timeout_ms).await)
    }

    // scrolling
    #[pyo3(signature = (pixels=600.0))]
    fn scroll_down<'py>(&self, py: Python<'py>, pixels: f64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.scroll_by(0.0, pixels).await)
    }

    #[pyo3(signature = (pixels=600.0))]
    fn scroll_up<'py>(&self, py: Python<'py>, pixels: f64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.scroll_by(0.0, -pixels).await)
    }

    #[pyo3(signature = (step=800.0, pause_ms=600, max_rounds=50))]
    fn scroll_to_bottom<'py>(&self, py: Python<'py>, step: f64, pause_ms: u64, max_rounds: u32) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.scroll_to_bottom(step, pause_ms, max_rounds).await)
    }

    fn scroll_to_top<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.scroll_to_top().await)
    }

    #[pyo3(signature = (selector, timeout_ms=10000))]
    fn scroll_into_view<'py>(&self, py: Python<'py>, selector: String, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.scroll_into_view(&selector, timeout_ms).await)
    }

    // emulation
    #[pyo3(signature = (width, height, scale=1.0, mobile=false))]
    fn set_viewport<'py>(&self, py: Python<'py>, width: i64, height: i64, scale: f64, mobile: bool) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.set_viewport(width, height, scale, mobile).await)
    }

    fn clear_viewport<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.clear_viewport().await)
    }

    #[pyo3(signature = (user_agent, accept_language=None, platform=None))]
    fn set_user_agent<'py>(&self, py: Python<'py>, user_agent: String, accept_language: Option<String>, platform: Option<String>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.set_user_agent(&user_agent, accept_language.as_deref(), platform.as_deref()).await)
    }

    fn set_timezone<'py>(&self, py: Python<'py>, timezone_id: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.set_timezone(&timezone_id).await)
    }

    fn set_locale<'py>(&self, py: Python<'py>, locale: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.set_locale(&locale).await)
    }

    #[pyo3(signature = (latitude, longitude, accuracy=50.0))]
    fn set_geolocation<'py>(&self, py: Python<'py>, latitude: f64, longitude: f64, accuracy: f64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.set_geolocation(latitude, longitude, accuracy).await)
    }

    // network
    fn set_extra_headers<'py>(&self, py: Python<'py>, headers: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let h = py_to_value(headers)?;
        fut!(py, self.inner, |t| t.set_extra_headers(h).await)
    }

    fn block_urls<'py>(&self, py: Python<'py>, patterns: Vec<String>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.block_urls(&patterns).await)
    }

    #[pyo3(signature = (disabled=true))]
    fn set_cache_disabled<'py>(&self, py: Python<'py>, disabled: bool) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.set_cache_disabled(disabled).await)
    }

    // cookies & storage
    #[pyo3(signature = (urls=None))]
    fn get_cookies<'py>(&self, py: Python<'py>, urls: Option<Vec<String>>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.get_cookies(urls).await.map(PyJson))
    }

    fn set_cookies<'py>(&self, py: Python<'py>, cookies: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let c = py_to_value(cookies)?;
        fut!(py, self.inner, |t| t.set_cookies(c).await)
    }

    #[pyo3(signature = (name, domain=None, url=None))]
    fn delete_cookies<'py>(&self, py: Python<'py>, name: String, domain: Option<String>, url: Option<String>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.delete_cookies(&name, domain.as_deref(), url.as_deref()).await)
    }

    fn clear_cookies<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.clear_cookies().await)
    }

    fn save_cookies<'py>(&self, py: Python<'py>, path: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.save_cookies(&path).await)
    }

    fn load_cookies<'py>(&self, py: Python<'py>, path: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.load_cookies(&path).await)
    }

    #[pyo3(signature = (key=None, session=false))]
    fn storage_get<'py>(&self, py: Python<'py>, key: Option<String>, session: bool) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.storage_get(key.as_deref(), session).await.map(PyJson))
    }

    #[pyo3(signature = (key, value, session=false))]
    fn storage_set<'py>(&self, py: Python<'py>, key: String, value: String, session: bool) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.storage_set(&key, &value, session).await)
    }

    #[pyo3(signature = (session=false))]
    fn storage_clear<'py>(&self, py: Python<'py>, session: bool) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.storage_clear(session).await)
    }

    // dialogs
    /// accept=None turns auto-handling off.
    #[pyo3(signature = (accept=Some(true), prompt_text=None))]
    fn auto_dialog<'py>(&self, py: Python<'py>, accept: Option<bool>, prompt_text: Option<String>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| async move { t.set_dialog_handler(accept.map(|a| (a, prompt_text))) }.await)
    }

    #[pyo3(signature = (accept=true, prompt_text=None))]
    fn handle_dialog<'py>(&self, py: Python<'py>, accept: bool, prompt_text: Option<String>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.handle_dialog(accept, prompt_text.as_deref()).await)
    }

    // capture
    #[pyo3(signature = (path=None, full_page=false, format="png".to_string(), quality=None, selector=None))]
    fn screenshot<'py>(
        &self,
        py: Python<'py>,
        path: Option<String>,
        full_page: bool,
        format: String,
        quality: Option<i64>,
        selector: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.screenshot(path, full_page, &format, quality, selector.as_deref()).await.map(PyBin))
    }

    #[pyo3(signature = (path=None, landscape=false))]
    fn pdf<'py>(&self, py: Python<'py>, path: Option<String>, landscape: bool) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.pdf(path, landscape).await.map(PyBin))
    }

    fn sleep<'py>(&self, py: Python<'py>, seconds: f64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| { t.sleep(seconds).await; Ok::<(), String>(()) })
    }
}

// ───────────────────────────── Browser ─────────────────────────────

#[pyclass(name = "Browser")]
pub struct PyBrowser {
    inner: Arc<BrowserCore>,
}

#[pymethods]
impl PyBrowser {
    fn __repr__(&self) -> String {
        format!("<Browser port={}>", self.inner.port)
    }

    #[getter]
    fn port(&self) -> u16 {
        self.inner.port
    }

    #[getter]
    fn user_data_dir(&self) -> String {
        self.inner.user_data_dir.clone()
    }

    fn main_tab<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |c| c.main_tab().await.map(|inner| PyTab { inner }))
    }

    #[pyo3(signature = (url="about:blank".to_string()))]
    fn new_tab<'py>(&self, py: Python<'py>, url: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |c| c.new_tab(&url).await.map(|inner| PyTab { inner }))
    }

    /// Like nodriver: navigate the main tab (or a new one with new_tab=True) and return it.
    #[pyo3(signature = (url, new_tab=false, wait_until="load".to_string()))]
    fn get<'py>(&self, py: Python<'py>, url: String, new_tab: bool, wait_until: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |c| async {
            if new_tab {
                c.new_tab(&url).await
            } else {
                let tab = c.main_tab().await?;
                tab.get(&url, None, &wait_until).await?;
                Ok(tab)
            }
        }.await.map(|inner| PyTab { inner }))
    }

    fn tabs<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |c| c.list_tabs().await
            .map(|v| v.into_iter().map(|inner| PyTab { inner }).collect::<Vec<_>>()))
    }

    fn stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |c| { c.stop().await; Ok::<(), String>(()) })
    }

    fn __aenter__<'py>(slf: Bound<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let obj: Py<PyAny> = slf.into_any().unbind();
        pyo3_async_runtimes::tokio::future_into_py(py, async move { Ok(obj) })
    }

    #[pyo3(signature = (*_args))]
    fn __aexit__<'py>(&self, py: Python<'py>, _args: &Bound<'py, pyo3::types::PyTuple>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |c| { c.stop().await; Ok::<bool, String>(false) })
    }
}

/// Launch a browser (like nodriver.start()).
/// proxy: "http://host:port", "http://user:pass@host:port", "socks5://host:port"
#[pyfunction]
#[pyo3(signature = (headless=false, user_data_dir=None, browser_path=None, extra_args=None, proxy=None))]
fn start<'py>(
    py: Python<'py>,
    headless: bool,
    user_data_dir: Option<String>,
    browser_path: Option<String>,
    extra_args: Option<Vec<String>>,
    proxy: Option<String>,
) -> PyResult<Bound<'py, PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let core = BrowserCore::launch(headless, user_data_dir, browser_path, extra_args, proxy).await.map_err(rt)?;
        Ok(PyBrowser { inner: Arc::new(core) })
    })
}

#[pymodule]
fn best_driver(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyBrowser>()?;
    m.add_class::<PyTab>()?;
    m.add_function(wrap_pyfunction!(start, m)?)?;
    Ok(())
}
