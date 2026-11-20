// src/lib.rs — Rust 2021, PyO3 0.30 + pyo3-async-runtimes: Python bindings over flattened CDP sessions
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, OnceCell};
use tokio::task::JoinHandle;

mod browser;
mod cdp;
mod error;
mod input;
mod locator;
mod tab;

use browser::{build_headless_ua_override, BrowserProcess};
use cdp::CdpConnection;
use error::{DriverError, DriverResult};
use tab::{Tab, EVASION_JS};

// ───────────────────────────── Conversions ─────────────────────────────

fn rt(e: DriverError) -> PyErr {
    match e {
        DriverError::Timeout(msg) => PyErr::new::<pyo3::exceptions::PyTimeoutError, _>(msg),
        DriverError::InvalidInput(msg) => PyErr::new::<pyo3::exceptions::PyValueError, _>(msg),
        other => PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(other.to_string()),
    }
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

/// Direct conversion from Python primitives to serde Value, bypassing redundant string serialization.
fn py_to_value(obj: &Bound<'_, PyAny>) -> PyResult<Value> {
    if obj.is_none() {
        return Ok(Value::Null);
    }
    if let Ok(b) = obj.extract::<bool>() {
        return Ok(Value::Bool(b));
    }
    if let Ok(i) = obj.extract::<i64>() {
        return Ok(Value::Number(i.into()));
    }
    if let Ok(f) = obj.extract::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Ok(Value::Number(n));
        }
    }
    if let Ok(s) = obj.extract::<String>() {
        return Ok(Value::String(s));
    }
    if let Ok(list) = obj.cast::<PyList>() {
        let mut arr = Vec::with_capacity(list.len());
        for item in list.iter() {
            arr.push(py_to_value(&item)?);
        }
        return Ok(Value::Array(arr));
    }
    if let Ok(dict) = obj.cast::<PyDict>() {
        let mut map = serde_json::Map::with_capacity(dict.len());
        for (k, v) in dict.iter() {
            let key = k.extract::<String>()?;
            map.insert(key, py_to_value(&v)?);
        }
        return Ok(Value::Object(map));
    }
    let dumped: String = obj
        .py()
        .import("json")?
        .call_method1("dumps", (obj,))?
        .extract()?;
    serde_json::from_str(&dumped).map_err(|e| rt(DriverError::InvalidInput(e.to_string())))
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

// ───────────────────────────── Browser Core (Single Flattened WebSocket) ─────────────────────────────

#[derive(Debug, PartialEq, Eq)]
struct ProxyAuth {
    user: String,
    pass: String,
}

/// `"scheme://user:pass@host:port"` -> `("scheme://host:port", Some(auth))`
fn split_proxy(raw: &str) -> (String, Option<ProxyAuth>) {
    let (scheme, rest) = raw.split_once("://").unwrap_or(("http", raw));
    match rest.rsplit_once('@') {
        Some((creds, hostport)) => {
            let (u, p) = creds.split_once(':').unwrap_or((creds, ""));
            (
                format!("{}://{}", scheme, hostport),
                Some(ProxyAuth {
                    user: u.to_string(),
                    pass: p.to_string(),
                }),
            )
        }
        None => (format!("{}://{}", scheme, rest), None),
    }
}

type ArmedMap = Arc<Mutex<HashMap<String, Arc<OnceCell<()>>>>>;

/// Injects evasion script + headless UA override + nested OOPIF auto-attach ONCE per target (race-free via OnceCell).
async fn arm_target_once(
    conn: &Arc<CdpConnection>,
    armed: &ArmedMap,
    target_id: &str,
    session_id: &str,
    headless_ua: Option<&Value>,
) {
    let cell = {
        let mut map = armed.lock().await;
        Arc::clone(
            map.entry(target_id.to_string())
                .or_insert_with(|| Arc::new(OnceCell::new())),
        )
    };
    cell.get_or_init(|| async {
        let _ = conn.call_raw(Some(session_id), "Page.enable", json!({})).await;
        if let Some(ua_params) = headless_ua {
            let _ = conn
                .call_raw(Some(session_id), "Emulation.setUserAgentOverride", ua_params.clone())
                .await;
        }
        let _ = conn
            .call_raw(
                Some(session_id),
                "Page.addScriptToEvaluateOnNewDocument",
                json!({ "source": EVASION_JS }),
            )
            .await;
        // Auto-attach nested OOPIF iframes inside this target before their JS runs
        let _ = conn
            .call_raw(
                Some(session_id),
                "Target.setAutoAttach",
                json!({
                    "autoAttach": true,
                    "waitForDebuggerOnStart": true,
                    "flatten": true
                }),
            )
            .await;
    })
    .await;
}

pub struct BrowserCore {
    process: Mutex<Option<BrowserProcess>>,
    conn: Arc<CdpConnection>,
    tabs: Mutex<HashMap<String, Arc<Tab>>>,
    sessions_by_target: Arc<Mutex<HashMap<String, String>>>,
    armed_targets: ArmedMap,
    headless_ua: Option<Value>,
    proxy_auth: Option<ProxyAuth>,
    auto_attach_task: Mutex<Option<JoinHandle<()>>>,
    port: u16,
    user_data_dir: String,
    ws_url: String,
}

impl BrowserCore {
    async fn launch(
        headless: bool,
        user_data_dir: Option<String>,
        browser_path: Option<String>,
        extra_args: Option<Vec<String>>,
        proxy: Option<String>,
    ) -> DriverResult<Self> {
        let mut args = extra_args.unwrap_or_default();
        let mut proxy_auth = None;
        if let Some(p) = proxy {
            let (server, auth) = split_proxy(&p);
            args.push(format!("--proxy-server={}", server));
            proxy_auth = auth;
        }
        let proc = BrowserProcess::launch(headless, user_data_dir, browser_path, Some(args)).await?;
        let port = proc.port;
        let ws_url = proc.ws_url.clone();
        let user_data_dir = proc.user_data_dir.to_string_lossy().to_string();

        // Single browser-level CDP WebSocket shared by all tabs, popups, and OOPIF iframes
        let conn = CdpConnection::connect(&ws_url).await?;

        let headless_ua = match conn.call("Browser.getVersion", json!({})).await {
            Ok(ver) => ver
                .get("userAgent")
                .and_then(|v| v.as_str())
                .and_then(build_headless_ua_override),
            Err(_) => None,
        };

        let sessions_by_target: Arc<Mutex<HashMap<String, String>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let armed_targets: ArmedMap = Arc::new(Mutex::new(HashMap::new()));

        // Subscribe BEFORE enabling Target.setAutoAttach so no early child target event is missed
        let mut rx = conn.subscribe();
        let conn_sup = Arc::clone(&conn);
        let sessions_sup = Arc::clone(&sessions_by_target);
        let armed_sup = Arc::clone(&armed_targets);
        let ua_sup = headless_ua.clone();

        let auto_attach_task = tokio::spawn(async move {
            loop {
                let ev = match rx.recv().await {
                    Ok(ev) => ev,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                };
                match ev.method.as_str() {
                    "Target.attachedToTarget" => {
                        let Some(sid) = ev.params.get("sessionId").and_then(|v| v.as_str()).map(str::to_string) else {
                            continue;
                        };
                        let tid = ev
                            .params
                            .pointer("/targetInfo/targetId")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let ttype = ev
                            .params
                            .pointer("/targetInfo/type")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let waiting = ev
                            .params
                            .get("waitingForDebugger")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);

                        if !tid.is_empty() {
                            sessions_sup.lock().await.entry(tid.clone()).or_insert_with(|| sid.clone());
                        }

                        let conn_c = Arc::clone(&conn_sup);
                        let armed_c = Arc::clone(&armed_sup);
                        let ua_c = ua_sup.clone();
                        tokio::spawn(async move {
                            if (ttype == "page" || ttype == "iframe") && !tid.is_empty() {
                                arm_target_once(&conn_c, &armed_c, &tid, &sid, ua_c.as_ref()).await;
                            }
                            // Unconditionally resume paused targets so nothing can ever hang
                            if waiting {
                                let _ = conn_c
                                    .call_raw(Some(&sid), "Runtime.runIfWaitingForDebugger", json!({}))
                                    .await;
                            }
                        });
                    }
                    "Target.detachedFromTarget" => {
                        if let Some(sid) = ev.params.get("sessionId").and_then(|v| v.as_str()) {
                            let mut map = sessions_sup.lock().await;
                            map.retain(|_, v| v != sid);
                        }
                    }
                    "Target.targetDestroyed" => {
                        if let Some(tid) = ev.params.get("targetId").and_then(|v| v.as_str()) {
                            sessions_sup.lock().await.remove(tid);
                            armed_sup.lock().await.remove(tid);
                        }
                    }
                    _ => {}
                }
            }
        });

        let _ = conn.call("Target.setDiscoverTargets", json!({ "discover": true })).await;
        let _ = conn
            .call(
                "Target.setAutoAttach",
                json!({
                    "autoAttach": true,
                    "waitForDebuggerOnStart": true,
                    "flatten": true
                }),
            )
            .await;

        Ok(Self {
            process: Mutex::new(Some(proc)),
            conn,
            tabs: Mutex::new(HashMap::new()),
            sessions_by_target,
            armed_targets,
            headless_ua,
            proxy_auth,
            auto_attach_task: Mutex::new(Some(auto_attach_task)),
            port,
            user_data_dir,
            ws_url,
        })
    }

    async fn attach(&self, target_id: String) -> DriverResult<Arc<Tab>> {
        // Reuse auto-attached flattened session if present; otherwise attach via Target.attachToTarget(flatten: true)
        let session_id = match self.sessions_by_target.lock().await.get(&target_id).cloned() {
            Some(sid) => sid,
            None => {
                let res = self
                    .conn
                    .call(
                        "Target.attachToTarget",
                        json!({ "targetId": target_id, "flatten": true }),
                    )
                    .await?;
                let sid = res
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| DriverError::Connection("Target.attachToTarget không trả sessionId".to_string()))?
                    .to_string();
                self.sessions_by_target
                    .lock()
                    .await
                    .insert(target_id.clone(), sid.clone());
                sid
            }
        };

        // Wait until evasion + headless UA override are guaranteed armed on this target
        arm_target_once(
            &self.conn,
            &self.armed_targets,
            &target_id,
            &session_id,
            self.headless_ua.as_ref(),
        )
        .await;

        let session = self.conn.session(session_id);
        let tab = Tab::new(target_id.clone(), session).await?;
        if let Some(a) = &self.proxy_auth {
            tab.enable_proxy_auth(a.user.clone(), a.pass.clone()).await?;
        }
        let tab = Arc::new(tab);
        self.tabs.lock().await.insert(target_id, Arc::clone(&tab));
        Ok(tab)
    }

    /// Page targets via browser WebSocket `Target.getTargets` (zero HTTP polling).
    async fn list_tabs(&self) -> DriverResult<Vec<Arc<Tab>>> {
        if self.process.lock().await.is_none() {
            return Err(DriverError::BrowserClosed);
        }
        let res = self.conn.call("Target.getTargets", json!({})).await?;
        let page_ids: Vec<String> = res
            .get("targetInfos")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
                    .filter_map(|t| t.get("targetId").and_then(|v| v.as_str()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        {
            let mut cache = self.tabs.lock().await;
            cache.retain(|id, _| page_ids.iter().any(|p| p == id));
        }

        let mut out = Vec::with_capacity(page_ids.len());
        for tid in page_ids {
            let cached = self.tabs.lock().await.get(&tid).cloned();
            match cached {
                Some(tab) => out.push(tab),
                None => out.push(self.attach(tid).await?),
            }
        }
        Ok(out)
    }

    async fn main_tab(&self) -> DriverResult<Arc<Tab>> {
        match self.list_tabs().await?.into_iter().next() {
            Some(t) => Ok(t),
            None => self.new_tab("about:blank").await,
        }
    }

    async fn new_tab(&self, url: &str) -> DriverResult<Arc<Tab>> {
        if self.process.lock().await.is_none() {
            return Err(DriverError::BrowserClosed);
        }
        // Create blank target via CDP WebSocket so evasion + proxy auth are armed before first navigation
        let res = self
            .conn
            .call("Target.createTarget", json!({ "url": "about:blank" }))
            .await?;
        let target_id = res
            .get("targetId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DriverError::Connection("Target.createTarget không trả targetId".to_string()))?
            .to_string();
        let tab = self.attach(target_id).await?;
        if url != "about:blank" {
            tab.get(url, None, "load").await?;
        }
        Ok(tab)
    }

    async fn is_alive(&self) -> bool {
        let mut lock = self.process.lock().await;
        match lock.as_mut() {
            Some(proc) => proc.is_alive(),
            None => false,
        }
    }

    async fn stop(&self) {
        if let Some(h) = self.auto_attach_task.lock().await.take() {
            h.abort();
        }
        self.tabs.lock().await.clear();
        self.sessions_by_target.lock().await.clear();
        self.armed_targets.lock().await.clear();
        // Ask Chromium to close gracefully first so persistent profiles flush SQLite/Cookies cleanly
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(1500),
            self.conn.call("Browser.close", json!({})),
        )
        .await;
        if let Some(mut proc) = self.process.lock().await.take() {
            proc.wait_graceful_or_kill(std::time::Duration::from_millis(1500));
        }
    }
}

// ───────────────────────────── Element ─────────────────────────────

#[pyclass(name = "Element")]
pub struct PyElement {
    tab: Arc<Tab>,
    backend_node_id: i64,
    data: Value,
}

#[pymethods]
impl PyElement {
    fn __getitem__<'py>(&self, py: Python<'py>, key: String) -> PyResult<Bound<'py, PyAny>> {
        if let Some(v) = self.data.get(&key) {
            value_to_py(py, v)
        } else {
            Err(pyo3::exceptions::PyKeyError::new_err(key))
        }
    }

    #[pyo3(signature = (key, default=None))]
    fn get<'py>(
        &self,
        py: Python<'py>,
        key: String,
        default: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        if let Some(v) = self.data.get(&key) {
            value_to_py(py, v)
        } else {
            Ok(default.unwrap_or_else(|| py.None().into_bound(py)))
        }
    }

    #[getter]
    fn backend_node_id(&self) -> i64 {
        self.backend_node_id
    }

    #[getter]
    fn tag(&self) -> String {
        self.data.get("tag").and_then(|v| v.as_str()).unwrap_or("").to_string()
    }

    #[getter]
    fn text(&self) -> String {
        self.data.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string()
    }

    #[getter]
    fn value(&self) -> Option<String> {
        self.data.get("value").and_then(|v| v.as_str()).map(|s| s.to_string())
    }

    #[getter]
    fn id(&self) -> Option<String> {
        self.data.get("id").and_then(|v| v.as_str()).map(|s| s.to_string())
    }

    #[getter]
    fn class_name(&self) -> Option<String> {
        self.data.get("class").and_then(|v| v.as_str()).map(|s| s.to_string())
    }

    #[getter]
    fn rect<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        value_to_py(py, self.data.get("rect").unwrap_or(&Value::Null))
    }

    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        value_to_py(py, &self.data)
    }

    fn __repr__(&self) -> String {
        let tag = self.tag();
        let id = self.id().unwrap_or_default();
        let txt = self.text();
        let short_text = if txt.chars().count() > 25 {
            format!("{}...", txt.chars().take(22).collect::<String>())
        } else {
            txt
        };
        format!("<Element <{}> id='{}' text='{}'>", tag, id, short_text)
    }

    #[pyo3(signature = (button="left".to_string(), click_count=1))]
    fn click<'py>(
        &self,
        py: Python<'py>,
        button: String,
        click_count: i64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let nid = self.backend_node_id;
        fut!(py, self.tab, |t| t.click_node(nid, &button, click_count).await)
    }

    fn hover<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let nid = self.backend_node_id;
        fut!(py, self.tab, |t| t.hover_node(nid).await)
    }

    #[pyo3(signature = (text, clear=false))]
    fn human_type<'py>(
        &self,
        py: Python<'py>,
        text: String,
        clear: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let nid = self.backend_node_id;
        fut!(py, self.tab, |t| t.human_type_node(nid, &text, clear).await)
    }

    fn clear<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let nid = self.backend_node_id;
        fut!(py, self.tab, |t| t.clear_node(nid).await)
    }

    fn fill<'py>(&self, py: Python<'py>, text: String) -> PyResult<Bound<'py, PyAny>> {
        let nid = self.backend_node_id;
        fut!(py, self.tab, |t| t.fill_node(nid, &text).await)
    }

    fn attr<'py>(&self, py: Python<'py>, name: String) -> PyResult<Bound<'py, PyAny>> {
        let nid = self.backend_node_id;
        fut!(py, self.tab, |t| t.attr_node(nid, &name).await)
    }

    fn inner_html<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let nid = self.backend_node_id;
        fut!(py, self.tab, |t| t.inner_html_node(nid).await)
    }

    fn outer_html<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let nid = self.backend_node_id;
        fut!(py, self.tab, |t| t.outer_html_node(nid).await)
    }

    fn scroll_into_view<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let nid = self.backend_node_id;
        fut!(py, self.tab, |t| t.scroll_into_view_node(nid).await)
    }

    #[pyo3(signature = (file_path=None))]
    fn screenshot<'py>(&self, py: Python<'py>, file_path: Option<String>) -> PyResult<Bound<'py, PyAny>> {
        let nid = self.backend_node_id;
        fut!(py, self.tab, |t| t.screenshot_node(nid, file_path).await.map(PyBin))
    }

    fn ele<'py>(&self, py: Python<'py>, query: String) -> PyResult<Bound<'py, PyAny>> {
        let tab = Arc::clone(&self.tab);
        let nid = self.backend_node_id;
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let opt = tab.ele_child(nid, &query).await.map_err(rt)?;
            Ok(opt.map(|(child_id, val)| PyElement {
                tab: Arc::clone(&tab),
                backend_node_id: child_id,
                data: val,
            }))
        })
    }

    fn eles<'py>(&self, py: Python<'py>, query: String) -> PyResult<Bound<'py, PyAny>> {
        let tab = Arc::clone(&self.tab);
        let nid = self.backend_node_id;
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let list = tab.eles_child(nid, &query).await.map_err(rt)?;
            let elements = list
                .into_iter()
                .map(|(child_id, val)| PyElement {
                    tab: Arc::clone(&tab),
                    backend_node_id: child_id,
                    data: val,
                })
                .collect::<Vec<_>>();
            Ok(elements)
        })
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
        format!(
            "<Tab target_id={} session_id={}>",
            self.inner.target_id,
            self.inner.session_id()
        )
    }

    #[getter]
    fn target_id(&self) -> String {
        self.inner.target_id.clone()
    }

    #[getter]
    fn session_id(&self) -> String {
        self.inner.session_id().to_string()
    }

    #[getter]
    fn frame_id(&self) -> Option<String> {
        self.inner.frame_id().map(str::to_string)
    }

    // iframes & downloads
    #[pyo3(signature = (selector, timeout_ms=10000))]
    fn iframe<'py>(
        &self,
        py: Python<'py>,
        selector: String,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t
            .iframe(&selector, timeout_ms)
            .await
            .map(|f| PyTab { inner: Arc::new(f) }))
    }

    fn frames<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.frames().await.map(PyJson))
    }

    fn set_download_dir<'py>(&self, py: Python<'py>, download_dir: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.set_download_dir(&download_dir).await)
    }

    #[pyo3(signature = (download_dir, timeout_ms=30000))]
    fn wait_for_download<'py>(
        &self,
        py: Python<'py>,
        download_dir: String,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t
            .wait_for_download(&download_dir, timeout_ms)
            .await
            .map(PyJson))
    }

    // navigation
    #[pyo3(signature = (url, timeout_ms=None, wait_until="load".to_string()))]
    fn get<'py>(
        &self,
        py: Python<'py>,
        url: String,
        timeout_ms: Option<u64>,
        wait_until: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.get(&url, timeout_ms, &wait_until).await)
    }

    #[pyo3(signature = (state="load".to_string(), timeout_ms=30000))]
    fn wait_for_load<'py>(
        &self,
        py: Python<'py>,
        state: String,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.wait_for_load(&state, timeout_ms).await)
    }

    #[pyo3(signature = (ignore_cache=false, timeout_ms=30000))]
    fn reload<'py>(
        &self,
        py: Python<'py>,
        ignore_cache: bool,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
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

    fn current_url<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
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
    fn wait_for<'py>(
        &self,
        py: Python<'py>,
        selector: String,
        timeout_ms: u64,
        visible: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.wait_for_selector(&selector, timeout_ms, visible).await)
    }

    #[pyo3(signature = (selector, timeout_ms=10000, visible=false))]
    fn wait_for_selector<'py>(
        &self,
        py: Python<'py>,
        selector: String,
        timeout_ms: u64,
        visible: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
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
    fn wait_for_function<'py>(
        &self,
        py: Python<'py>,
        expression: String,
        timeout_ms: u64,
        poll_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.wait_for_function(&expression, timeout_ms, poll_ms).await.map(PyJson))
    }

    // mouse
    #[pyo3(signature = (selector, button="left".to_string(), click_count=1, timeout_ms=10000))]
    fn click<'py>(
        &self,
        py: Python<'py>,
        selector: String,
        button: String,
        click_count: i64,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
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
    fn click_text<'py>(
        &self,
        py: Python<'py>,
        text: String,
        exact: bool,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.click_text(&text, exact, timeout_ms).await)
    }

    #[pyo3(signature = (x, y, button="left".to_string(), click_count=1))]
    fn mouse_click<'py>(
        &self,
        py: Python<'py>,
        x: f64,
        y: f64,
        button: String,
        click_count: i64,
    ) -> PyResult<Bound<'py, PyAny>> {
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
    fn drag<'py>(
        &self,
        py: Python<'py>,
        from_selector: String,
        to_selector: String,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.drag(&from_selector, &to_selector, timeout_ms).await)
    }

    #[pyo3(signature = (x1, y1, x2, y2))]
    fn swipe<'py>(&self, py: Python<'py>, x1: f64, y1: f64, x2: f64, y2: f64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.swipe(x1, y1, x2, y2).await)
    }

    #[pyo3(signature = (selector, text, clear=false, timeout_ms=10000))]
    fn human_type<'py>(
        &self,
        py: Python<'py>,
        selector: String,
        text: String,
        clear: bool,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.human_type(&selector, &text, clear, timeout_ms).await)
    }

    #[pyo3(signature = (query, timeout_ms=10000))]
    fn ele<'py>(&self, py: Python<'py>, query: String, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        let tab = Arc::clone(&self.inner);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let opt = tab.ele(&query, timeout_ms).await.map_err(rt)?;
            Ok(opt.map(|(nid, val)| PyElement {
                tab: Arc::clone(&tab),
                backend_node_id: nid,
                data: val,
            }))
        })
    }

    #[pyo3(signature = (query, timeout_ms=10000))]
    fn eles<'py>(&self, py: Python<'py>, query: String, timeout_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        let tab = Arc::clone(&self.inner);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let list = tab.eles(&query, timeout_ms).await.map_err(rt)?;
            let elements = list
                .into_iter()
                .map(|(nid, val)| PyElement {
                    tab: Arc::clone(&tab),
                    backend_node_id: nid,
                    data: val,
                })
                .collect::<Vec<_>>();
            Ok(elements)
        })
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
    fn send_keys<'py>(
        &self,
        py: Python<'py>,
        selector: String,
        text: String,
        clear: bool,
        delay_ms: u64,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.send_keys(&selector, &text, clear, delay_ms, timeout_ms).await)
    }

    #[pyo3(signature = (text, delay_ms=60))]
    fn type_text<'py>(&self, py: Python<'py>, text: String, delay_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.type_text(&text, delay_ms).await)
    }

    #[pyo3(signature = (selector, text, timeout_ms=10000))]
    fn fill<'py>(
        &self,
        py: Python<'py>,
        selector: String,
        text: String,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.fill(&selector, &text, timeout_ms).await)
    }

    fn press_key<'py>(&self, py: Python<'py>, key: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.press_key(&key).await)
    }

    // forms
    #[pyo3(signature = (selector, value, timeout_ms=10000))]
    fn select_option<'py>(
        &self,
        py: Python<'py>,
        selector: String,
        value: String,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
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

    #[pyo3(signature = (selector, files, timeout_ms=10000))]
    fn upload_file<'py>(
        &self,
        py: Python<'py>,
        selector: String,
        files: &Bound<'py, PyAny>,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
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
    fn scroll_to_bottom<'py>(
        &self,
        py: Python<'py>,
        step: f64,
        pause_ms: u64,
        max_rounds: u32,
    ) -> PyResult<Bound<'py, PyAny>> {
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
    fn set_viewport<'py>(
        &self,
        py: Python<'py>,
        width: i64,
        height: i64,
        scale: f64,
        mobile: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.set_viewport(width, height, scale, mobile).await)
    }

    fn clear_viewport<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.clear_viewport().await)
    }

    #[pyo3(signature = (user_agent, accept_language=None, platform=None))]
    fn set_user_agent<'py>(
        &self,
        py: Python<'py>,
        user_agent: String,
        accept_language: Option<String>,
        platform: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.set_user_agent(&user_agent, accept_language.as_deref(), platform.as_deref()).await)
    }

    fn set_timezone<'py>(&self, py: Python<'py>, timezone_id: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.set_timezone(&timezone_id).await)
    }

    fn set_locale<'py>(&self, py: Python<'py>, locale: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.set_locale(&locale).await)
    }

    #[pyo3(signature = (latitude, longitude, accuracy=50.0))]
    fn set_geolocation<'py>(
        &self,
        py: Python<'py>,
        latitude: f64,
        longitude: f64,
        accuracy: f64,
    ) -> PyResult<Bound<'py, PyAny>> {
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

    #[pyo3(signature = (url_pattern, timeout_ms=30000))]
    fn wait_for_response<'py>(
        &self,
        py: Python<'py>,
        url_pattern: String,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.wait_for_response(&url_pattern, timeout_ms).await.map(PyJson))
    }

    #[pyo3(signature = (disabled=true))]
    fn set_cache_disabled<'py>(&self, py: Python<'py>, disabled: bool) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.set_cache_disabled(disabled).await)
    }

    // cookies & storage
    #[pyo3(signature = (name, value, domain=None, path=None))]
    fn set_cookie<'py>(
        &self,
        py: Python<'py>,
        name: String,
        value: String,
        domain: Option<String>,
        path: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.set_cookie(&name, &value, domain.as_deref(), path.as_deref()).await)
    }

    #[pyo3(signature = (urls=None))]
    fn cookies<'py>(&self, py: Python<'py>, urls: Option<Vec<String>>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.get_cookies(urls).await.map(PyJson))
    }

    #[pyo3(signature = (urls=None))]
    fn get_cookies<'py>(&self, py: Python<'py>, urls: Option<Vec<String>>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.get_cookies(urls).await.map(PyJson))
    }

    fn set_cookies<'py>(&self, py: Python<'py>, cookies: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let c = py_to_value(cookies)?;
        fut!(py, self.inner, |t| t.set_cookies(c).await)
    }

    #[pyo3(signature = (name, domain=None, url=None))]
    fn delete_cookies<'py>(
        &self,
        py: Python<'py>,
        name: String,
        domain: Option<String>,
        url: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
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
    fn storage_set<'py>(
        &self,
        py: Python<'py>,
        key: String,
        value: String,
        session: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.storage_set(&key, &value, session).await)
    }

    #[pyo3(signature = (session=false))]
    fn storage_clear<'py>(&self, py: Python<'py>, session: bool) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.storage_clear(session).await)
    }

    // dialogs
    #[pyo3(signature = (accept=Some(true), prompt_text=None))]
    fn auto_dialog(&self, accept: Option<bool>, prompt_text: Option<String>) -> PyResult<()> {
        self.inner
            .set_dialog_handler(accept.map(|a| (a, prompt_text)))
            .map_err(rt)
    }

    #[pyo3(signature = (accept=true, prompt_text=None))]
    fn handle_dialog<'py>(
        &self,
        py: Python<'py>,
        accept: bool,
        prompt_text: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.handle_dialog(accept, prompt_text.as_deref()).await)
    }

    // capture & misc
    #[pyo3(signature = (file_path=None, full_page=false, format="png".to_string(), quality=None, selector=None))]
    fn screenshot<'py>(
        &self,
        py: Python<'py>,
        file_path: Option<String>,
        full_page: bool,
        format: String,
        quality: Option<i64>,
        selector: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t
            .screenshot(file_path, full_page, &format, quality, selector.as_deref())
            .await
            .map(PyBin))
    }

    #[pyo3(signature = (file_path=None, landscape=false))]
    fn pdf<'py>(&self, py: Python<'py>, file_path: Option<String>, landscape: bool) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.pdf(file_path, landscape).await.map(PyBin))
    }

    fn sleep<'py>(&self, py: Python<'py>, seconds: f64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| {
            t.sleep(seconds).await;
            Ok::<(), DriverError>(())
        })
    }

    /// Raw CDP call scoped to this tab's flattened `sessionId`.
    #[pyo3(signature = (method, params=None))]
    fn send<'py>(
        &self,
        py: Python<'py>,
        method: String,
        params: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let val = match params {
            Some(p) => py_to_value(p)?,
            None => Value::Object(serde_json::Map::new()),
        };
        fut!(py, self.inner, |t| t.cdp.call(&method, val).await.map(PyJson))
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
        format!("<Browser port={} user_data_dir={}>", self.inner.port, self.inner.user_data_dir)
    }

    #[getter]
    fn port(&self) -> u16 {
        self.inner.port
    }

    #[getter]
    fn user_data_dir(&self) -> String {
        self.inner.user_data_dir.clone()
    }

    #[getter]
    fn ws_url(&self) -> String {
        self.inner.ws_url.clone()
    }

    fn is_alive<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |c| Ok::<bool, DriverError>(c.is_alive().await))
    }

    fn main_tab<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |c| c.main_tab().await.map(|t| PyTab { inner: t }))
    }

    #[pyo3(signature = (url="about:blank".to_string()))]
    fn new_tab<'py>(&self, py: Python<'py>, url: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |c| c.new_tab(&url).await.map(|t| PyTab { inner: t }))
    }

    fn tabs<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |c| c
            .list_tabs()
            .await
            .map(|v| v.into_iter().map(|t| PyTab { inner: t }).collect::<Vec<_>>()))
    }

    #[pyo3(signature = (url, new_tab=false, wait_until="load".to_string()))]
    fn get<'py>(
        &self,
        py: Python<'py>,
        url: String,
        new_tab: bool,
        wait_until: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |c| async {
            let tab = if new_tab {
                c.new_tab("about:blank").await?
            } else {
                c.main_tab().await?
            };
            tab.get(&url, None, &wait_until).await?;
            Ok::<PyTab, DriverError>(PyTab { inner: tab })
        }
        .await)
    }

    fn stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |c| {
            c.stop().await;
            Ok::<(), DriverError>(())
        })
    }

    fn __aenter__<'py>(slf: Bound<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let obj: Py<PyAny> = slf.into_any().unbind();
        pyo3_async_runtimes::tokio::future_into_py(py, async move { Ok(obj) })
    }

    #[pyo3(signature = (*_args))]
    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        _args: &Bound<'py, pyo3::types::PyTuple>,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |c| {
            c.stop().await;
            Ok::<bool, DriverError>(false)
        })
    }
}

/// Launch a browser over a single flattened browser-level CDP WebSocket.
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
        let core = BrowserCore::launch(headless, user_data_dir, browser_path, extra_args, proxy)
            .await
            .map_err(rt)?;
        Ok(PyBrowser {
            inner: Arc::new(core),
        })
    })
}

#[pymodule]
fn best_driver(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyBrowser>()?;
    m.add_class::<PyTab>()?;
    m.add_class::<PyElement>()?;
    m.add_function(wrap_pyfunction!(start, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_proxy_handles_auth_and_plain() {
        let (srv, auth) = split_proxy("http://alice:p@ss@1.2.3.4:8080");
        assert_eq!(srv, "http://1.2.3.4:8080");
        assert_eq!(
            auth,
            Some(ProxyAuth {
                user: "alice".to_string(),
                pass: "p@ss".to_string()
            })
        );

        let (srv2, auth2) = split_proxy("socks5://127.0.0.1:1080");
        assert_eq!(srv2, "socks5://127.0.0.1:1080");
        assert!(auth2.is_none());
    }
}
