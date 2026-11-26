// src/lib.rs — Rust 2021, PyO3 0.30 + pyo3-async-runtimes: Python bindings over flattened CDP sessions
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use tokio::sync::{Mutex, OnceCell};
use tokio::task::JoinHandle;

mod browser;
mod cdp;
mod error;
mod input;
mod locator;
mod tab;

use browser::{build_headless_ua_override, resolve_cdp_ws_url, BrowserProcess};
use cdp::{CdpConnection, SessionEventRx};
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

/// Converts a Python `str` or `os.PathLike` (`pathlib.Path`) into a UTF-8 path string.
fn py_to_path(obj: &Bound<'_, PyAny>) -> PyResult<String> {
    if let Ok(s) = obj.extract::<String>() {
        return Ok(s);
    }
    if let Ok(fspath_fn) = obj.getattr("__fspath__") {
        let res = fspath_fn.call0()?;
        if let Ok(s) = res.extract::<String>() {
            return Ok(s);
        }
        if let Ok(b) = res.extract::<Vec<u8>>() {
            return String::from_utf8(b)
                .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()));
        }
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "expected str or os.PathLike",
    ))
}

fn py_to_opt_path(obj: Option<&Bound<'_, PyAny>>) -> PyResult<Option<String>> {
    match obj {
        Some(v) if !v.is_none() => Ok(Some(py_to_path(v)?)),
        _ => Ok(None),
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProxyAuth {
    user: String,
    pass: String,
}

#[derive(Clone, Default)]
struct ContextOptions {
    user_agent: Option<String>,
    accept_language: Option<String>,
    platform: Option<String>,
    viewport: Option<(i64, i64, f64, bool)>,
    locale: Option<String>,
    timezone: Option<String>,
    extra_headers: Option<Value>,
    init_scripts: Arc<Mutex<Vec<String>>>,
}

impl ContextOptions {
    async fn apply_to_tab(&self, tab: &Tab) -> DriverResult<()> {
        if let Some(ref ua) = self.user_agent {
            tab.set_user_agent(ua, self.accept_language.as_deref(), self.platform.as_deref())
                .await?;
        }
        if let Some((w, h, scale, mobile)) = self.viewport {
            tab.set_viewport(w, h, scale, mobile).await?;
        }
        if let Some(ref loc) = self.locale {
            tab.set_locale(loc).await?;
        }
        if let Some(ref tz) = self.timezone {
            tab.set_timezone(tz).await?;
        }
        if let Some(ref hdrs) = self.extra_headers {
            tab.set_extra_headers(hdrs.clone()).await?;
        }
        let scripts = self.init_scripts.lock().await.clone();
        for src in scripts {
            let _ = tab.add_init_script(&src).await;
        }
        Ok(())
    }
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
/// Pipelines commands concurrently with optional `Runtime.runIfWaitingForDebugger` in strict wire order so
/// `<a target="_blank">` (`canAccessOpener: false`) targets paused at `waitForDebuggerOnStart` never deadlock.
async fn arm_target_once(
    conn: &Arc<CdpConnection>,
    armed: &ArmedMap,
    target_id: &str,
    session_id: &str,
    headless_ua: Option<&Value>,
    resume_if_waiting: bool,
) {
    let cell = {
        let mut map = armed.lock().await;
        Arc::clone(
            map.entry(target_id.to_string())
                .or_insert_with(|| Arc::new(OnceCell::new())),
        )
    };
    cell.get_or_init(|| async {
        let f_ua = async {
            if let Some(ua_params) = headless_ua {
                let _ = conn
                    .call_raw(Some(session_id), "Emulation.setUserAgentOverride", ua_params.clone())
                    .await;
            }
        };
        let f_evasion = conn.call_raw(
            Some(session_id),
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": EVASION_JS }),
        );
        let f_page = conn.call_raw(Some(session_id), "Page.enable", json!({}));
        let f_auto = conn.call_raw(
            Some(session_id),
            "Target.setAutoAttach",
            json!({
                "autoAttach": true,
                "waitForDebuggerOnStart": true,
                "flatten": true
            }),
        );
        let f_resume = async {
            if resume_if_waiting {
                let _ = conn
                    .call_raw(Some(session_id), "Runtime.runIfWaitingForDebugger", json!({}))
                    .await;
            }
        };
        let (_, _, _, _, _) = tokio::join!(f_ua, f_evasion, f_page, f_auto, f_resume);
    })
    .await;
}

type ContextConfigMap = Arc<Mutex<HashMap<String, (Option<String>, Option<ProxyAuth>, ContextOptions)>>>;

pub struct BrowserCore {
    process: Mutex<Option<BrowserProcess>>,
    conn: Arc<CdpConnection>,
    tabs: Arc<Mutex<HashMap<String, Arc<Tab>>>>,
    sessions_by_target: Arc<Mutex<HashMap<String, String>>>,
    armed_targets: ArmedMap,
    crashed_sessions: Arc<Mutex<HashSet<String>>>,
    api_created_targets: Arc<Mutex<HashSet<String>>>,
    context_configs: ContextConfigMap,
    headless_ua: Option<Value>,
    proxy: Option<String>,
    proxy_auth: Option<ProxyAuth>,
    auto_attach_task: Mutex<Option<JoinHandle<()>>>,
    detached: Arc<AtomicBool>,
    closed: AtomicBool,
    port: u16,
    user_data_dir: String,
    ws_url: String,
}

fn evict_python_session(session_id: &str) {
    Python::attach(|py| {
        if let Ok(m) = py.import("best_driver.network") {
            let _ = m.call_method1("_evict_session", (session_id,));
        }
    });
}

fn evict_python_context(context_id: &str) {
    Python::attach(|py| {
        if let Ok(m) = py.import("best_driver.network") {
            let _ = m.call_method1("_evict_context", (context_id,));
        }
    });
}

fn evict_python_sessions(session_ids: &[String]) {
    if session_ids.is_empty() {
        return;
    }
    Python::attach(|py| {
        if let Ok(m) = py.import("best_driver.network") {
            let _ = m.call_method1("_evict_sessions", (session_ids.to_vec(),));
        }
    });
}

impl BrowserCore {
    async fn launch(
        headless: bool,
        user_data_dir: Option<String>,
        browser_path: Option<String>,
        extra_args: Option<Vec<String>>,
        proxy: Option<String>,
        keep_open: bool,
        start_url: Option<String>,
    ) -> DriverResult<Self> {
        let mut args = extra_args.unwrap_or_default();
        let raw_proxy = proxy.clone();
        let mut proxy_auth = None;
        if let Some(p) = proxy {
            let (server, auth) = split_proxy(&p);
            args.push(format!("--proxy-server={}", server));
            // Prevent local IP leakage over WebRTC STUN/UDP when routing through a proxy
            if !args.iter().any(|a| a.starts_with("--force-webrtc-ip-handling-policy")) {
                args.push("--force-webrtc-ip-handling-policy=disable_non_proxied_udp".to_string());
            }
            proxy_auth = auth;
        }
        let detached = Arc::new(AtomicBool::new(keep_open));
        let proc = BrowserProcess::launch(
            headless,
            user_data_dir,
            browser_path,
            Some(args),
            Arc::clone(&detached),
            start_url,
        )
        .await?;
        let port = proc.port;
        let ws_url = proc.ws_url.clone();
        let user_data_dir = proc.user_data_dir.to_string_lossy().to_string();

        Self::from_parts(Some(proc), port, user_data_dir, ws_url, raw_proxy, proxy_auth, detached).await
    }

    async fn connect_existing(endpoint: &str, proxy: Option<String>) -> DriverResult<Self> {
        let (ws_url, port) = resolve_cdp_ws_url(endpoint).await?;
        let raw_proxy = proxy.clone();
        let proxy_auth = proxy.as_deref().and_then(|p| split_proxy(p).1);
        // Externally launched browsers are always detached so stop()/Drop never kill them
        let detached = Arc::new(AtomicBool::new(true));
        Self::from_parts(None, port, String::new(), ws_url, raw_proxy, proxy_auth, detached).await
    }

    async fn from_parts(
        proc: Option<BrowserProcess>,
        port: u16,
        user_data_dir: String,
        ws_url: String,
        raw_proxy: Option<String>,
        proxy_auth: Option<ProxyAuth>,
        detached: Arc<AtomicBool>,
    ) -> DriverResult<Self> {
        // Single browser-level CDP WebSocket shared by all tabs, popups, and OOPIF iframes
        let conn = CdpConnection::connect(&ws_url).await?;

        let headless_ua = match conn.call("Browser.getVersion", json!({})).await {
            Ok(ver) => ver
                .get("userAgent")
                .and_then(|v| v.as_str())
                .and_then(build_headless_ua_override),
            Err(_) => None,
        };

        let tabs: Arc<Mutex<HashMap<String, Arc<Tab>>>> = Arc::new(Mutex::new(HashMap::new()));
        let sessions_by_target: Arc<Mutex<HashMap<String, String>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let armed_targets: ArmedMap = Arc::new(Mutex::new(HashMap::new()));
        let crashed_sessions: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let api_created_targets: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let context_configs: ContextConfigMap = Arc::new(Mutex::new(HashMap::new()));

        // Subscribe BEFORE enabling Target.setAutoAttach so no early child target event is missed
        let mut rx = conn.subscribe();
        let conn_sup = Arc::clone(&conn);
        let tabs_sup = Arc::clone(&tabs);
        let sessions_sup = Arc::clone(&sessions_by_target);
        let armed_sup = Arc::clone(&armed_targets);
        let crashed_sup = Arc::clone(&crashed_sessions);
        let api_sup = Arc::clone(&api_created_targets);
        let ua_sup = headless_ua.clone();

        let auto_attach_task = tokio::spawn(async move {
            loop {
                let ev = match rx.recv().await {
                    Ok(ev) => ev,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                };
                match ev.method.as_str() {
                    "Target.targetCreated" => {
                        let ttype = ev
                            .params
                            .pointer("/targetInfo/type")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        let tid = ev
                            .params
                            .pointer("/targetInfo/targetId")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let opener_id = ev
                            .params
                            .pointer("/targetInfo/openerId")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty());
                        let bcid = ev
                            .params
                            .pointer("/targetInfo/browserContextId")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(str::to_string);
                        // Opener-less page targets (e.g. `<a target="_blank">` with implicit `rel="noopener"`)
                        // are not auto-attached by the opener session; emit popupReady once we verify
                        // they were not created programmatically by `new_tab()`.
                        if ttype == "page" && !tid.is_empty() && opener_id.is_none() {
                            let api_c = Arc::clone(&api_sup);
                            let sessions_c = Arc::clone(&sessions_sup);
                            let conn_c = Arc::clone(&conn_sup);
                            tokio::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(35)).await;
                                if api_c.lock().await.contains(&tid)
                                    || sessions_c.lock().await.contains_key(&tid)
                                {
                                    return;
                                }
                                conn_c.emit_internal(
                                    "_internal.popupReady",
                                    json!({
                                        "targetInfo": {
                                            "targetId": tid,
                                            "openerId": Value::Null,
                                            "browserContextId": bcid,
                                        }
                                    }),
                                );
                            });
                        }
                    }
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
                        let opener_id = ev
                            .params
                            .pointer("/targetInfo/openerId")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(str::to_string);
                        let bcid = ev
                            .params
                            .pointer("/targetInfo/browserContextId")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(str::to_string);
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
                        let api_c = Arc::clone(&api_sup);
                        let ua_c = ua_sup.clone();
                        tokio::spawn(async move {
                            if (ttype == "page" || ttype == "iframe") && !tid.is_empty() {
                                arm_target_once(&conn_c, &armed_c, &tid, &sid, ua_c.as_ref(), waiting).await;
                            } else if waiting {
                                // Unconditionally resume non-page/iframe paused targets (workers, etc.)
                                let _ = conn_c
                                    .call_raw(Some(&sid), "Runtime.runIfWaitingForDebugger", json!({}))
                                    .await;
                            }
                            // Notify popup waiters ONLY after stealth arming + debugger resume finish.
                            // Also emit when openerId is absent (implicit rel="noopener" on <a target="_blank">)
                            // as long as the target was not created programmatically by new_tab().
                            if ttype == "page" && !tid.is_empty() {
                                let is_api = api_c.lock().await.contains(&tid);
                                if opener_id.is_some() || !is_api {
                                    conn_c.emit_internal(
                                        "_internal.popupReady",
                                        json!({
                                            "targetInfo": {
                                                "targetId": tid,
                                                "openerId": opener_id,
                                                "browserContextId": bcid,
                                            }
                                        }),
                                    );
                                }
                            }
                        });
                    }
                    "Inspector.targetCrashed" => {
                        if let Some(sid) = ev.session_id {
                            crashed_sup.lock().await.insert(sid);
                        }
                    }
                    "Target.detachedFromTarget" => {
                        if let Some(sid) = ev.params.get("sessionId").and_then(|v| v.as_str()) {
                            sessions_sup.lock().await.retain(|_, v| v != sid);
                            tabs_sup.lock().await.retain(|_, t| t.session_id() != sid);
                            crashed_sup.lock().await.remove(sid);
                        }
                    }
                    "Target.targetDestroyed" => {
                        if let Some(tid) = ev.params.get("targetId").and_then(|v| v.as_str()) {
                            tabs_sup.lock().await.remove(tid);
                            sessions_sup.lock().await.remove(tid);
                            armed_sup.lock().await.remove(tid);
                            api_sup.lock().await.remove(tid);
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
            process: Mutex::new(proc),
            conn,
            tabs,
            sessions_by_target,
            armed_targets,
            crashed_sessions,
            api_created_targets,
            context_configs,
            headless_ua,
            proxy: raw_proxy,
            proxy_auth,
            auto_attach_task: Mutex::new(Some(auto_attach_task)),
            detached,
            closed: AtomicBool::new(false),
            port,
            user_data_dir,
            ws_url,
        })
    }

    async fn ensure_open(&self) -> DriverResult<()> {
        if self.closed.load(Ordering::SeqCst) || !self.conn.is_open() {
            return Err(DriverError::BrowserClosed);
        }
        let mut lock = self.process.lock().await;
        if let Some(proc) = lock.as_mut() {
            if !proc.is_alive() {
                return Err(DriverError::BrowserClosed);
            }
        }
        Ok(())
    }

    async fn attach(&self, target_id: String) -> DriverResult<Arc<Tab>> {
        self.attach_with_context(target_id, None, None, None).await
    }

    async fn attach_with_context(
        &self,
        target_id: String,
        proxy_override: Option<String>,
        proxy_auth_override: Option<ProxyAuth>,
        browser_context_id: Option<String>,
    ) -> DriverResult<Arc<Tab>> {
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
            false,
        )
        .await;

        let effective_proxy = proxy_override.or_else(|| self.proxy.clone());
        let effective_auth = proxy_auth_override.or_else(|| self.proxy_auth.clone());

        let session = self.conn.session(session_id);
        let tab = Tab::new(target_id.clone(), session, effective_proxy, browser_context_id).await?;
        if let Some(a) = effective_auth {
            tab.enable_proxy_auth(a.user, a.pass).await?;
        }
        let tab = Arc::new(tab);
        self.tabs.lock().await.insert(target_id, Arc::clone(&tab));
        Ok(tab)
    }

    async fn create_context(
        &self,
        proxy: Option<String>,
        proxy_bypass_list: Option<String>,
    ) -> DriverResult<(String, Option<String>, Option<ProxyAuth>)> {
        self.ensure_open().await?;
        let mut params = serde_json::Map::new();
        params.insert("disposeOnDetach".into(), json!(true));
        let mut ctx_auth = None;
        let raw_proxy = proxy.clone();
        if let Some(p) = proxy {
            let (server, auth) = split_proxy(&p);
            params.insert("proxyServer".into(), json!(server));
            ctx_auth = auth;
        }
        if let Some(bypass) = proxy_bypass_list {
            params.insert("proxyBypassList".into(), json!(bypass));
        }
        let res = self
            .conn
            .call("Target.createBrowserContext", Value::Object(params))
            .await?;
        let ctx_id = res
            .get("browserContextId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DriverError::Connection("Target.createBrowserContext không trả browserContextId".to_string()))?
            .to_string();
        Ok((ctx_id, raw_proxy, ctx_auth))
    }

    async fn new_tab_in_context(
        &self,
        ctx_id: &str,
        url: &str,
        proxy: Option<String>,
        proxy_auth: Option<ProxyAuth>,
        opts: &ContextOptions,
    ) -> DriverResult<Arc<Tab>> {
        self.ensure_open().await?;
        let res = self
            .conn
            .call(
                "Target.createTarget",
                json!({ "url": "about:blank", "browserContextId": ctx_id }),
            )
            .await?;
        let target_id = res
            .get("targetId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DriverError::Connection("Target.createTarget không trả targetId".to_string()))?
            .to_string();
        self.api_created_targets.lock().await.insert(target_id.clone());
        let tab = self
            .attach_with_context(target_id, proxy, proxy_auth, Some(ctx_id.to_string()))
            .await?;
        opts.apply_to_tab(&tab).await?;
        if url != "about:blank" {
            tab.get(url, None, "load").await?;
        }
        Ok(tab)
    }

    async fn list_tabs_in_context(
        &self,
        ctx_id: &str,
        proxy: Option<String>,
        proxy_auth: Option<ProxyAuth>,
        opts: &ContextOptions,
    ) -> DriverResult<Vec<Arc<Tab>>> {
        self.ensure_open().await?;
        let res = self.conn.call("Target.getTargets", json!({})).await?;
        let page_ids: Vec<String> = res
            .get("targetInfos")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|t| {
                        t.get("type").and_then(|v| v.as_str()) == Some("page")
                            && t.get("browserContextId").and_then(|v| v.as_str()) == Some(ctx_id)
                    })
                    .filter_map(|t| t.get("targetId").and_then(|v| v.as_str()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let mut out = Vec::with_capacity(page_ids.len());
        for tid in page_ids {
            let cached = self.tabs.lock().await.get(&tid).cloned();
            match cached {
                Some(tab) => out.push(tab),
                None => {
                    let tab = self
                        .attach_with_context(
                            tid,
                            proxy.clone(),
                            proxy_auth.clone(),
                            Some(ctx_id.to_string()),
                        )
                        .await?;
                    let _ = opts.apply_to_tab(&tab).await;
                    out.push(tab);
                }
            }
        }
        Ok(out)
    }

    async fn attach_discovered_target(
        &self,
        target_id: String,
        browser_context_id: Option<String>,
    ) -> DriverResult<Arc<Tab>> {
        self.ensure_open().await?;
        if let Some(cached) = self.tabs.lock().await.get(&target_id).cloned() {
            return Ok(cached);
        }
        let ctx_cfg = match browser_context_id.as_deref() {
            Some(cid) => self.context_configs.lock().await.get(cid).cloned(),
            None => None,
        };
        let (proxy_override, auth_override, opts) = match ctx_cfg {
            Some((p, a, o)) => (p, a, Some(o)),
            None => (None, None, None),
        };
        let effective_cid = if opts.is_some() { browser_context_id } else { None };
        let tab = self
            .attach_with_context(target_id, proxy_override, auth_override, effective_cid)
            .await?;
        if let Some(o) = opts {
            let _ = o.apply_to_tab(&tab).await;
        }
        Ok(tab)
    }

    async fn is_tab_alive(&self, target_id: &str, session_id: &str) -> bool {
        if !self.is_alive().await {
            return false;
        }
        if self.crashed_sessions.lock().await.contains(session_id) {
            return false;
        }
        match self.conn.call("Target.getTargets", json!({})).await {
            Ok(res) => res
                .get("targetInfos")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .any(|t| t.get("targetId").and_then(|v| v.as_str()) == Some(target_id))
                })
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    async fn dispose_context(&self, ctx_id: &str) -> DriverResult<()> {
        self.context_configs.lock().await.remove(ctx_id);
        self.conn
            .call(
                "Target.disposeBrowserContext",
                json!({ "browserContextId": ctx_id }),
            )
            .await
            .map(|_| ())
    }

    /// Page targets via browser WebSocket `Target.getTargets` (zero HTTP polling).
    async fn list_tabs(&self) -> DriverResult<Vec<Arc<Tab>>> {
        self.ensure_open().await?;
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
        self.ensure_open().await?;
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
        self.api_created_targets.lock().await.insert(target_id.clone());
        let tab = self.attach(target_id).await?;
        if url != "about:blank" {
            tab.get(url, None, "load").await?;
        }
        Ok(tab)
    }

    async fn is_alive(&self) -> bool {
        if self.closed.load(Ordering::SeqCst) || !self.conn.is_open() {
            return false;
        }
        let mut lock = self.process.lock().await;
        match lock.as_mut() {
            Some(proc) => proc.is_alive(),
            None => true,
        }
    }

    fn detach(&self) {
        self.detached.store(true, Ordering::SeqCst);
    }

    async fn wait_until_closed(&self, poll_ms: u64) {
        let step = std::time::Duration::from_millis(poll_ms.max(50));
        let mut rx = self.conn.subscribe();
        while self.is_alive().await {
            tokio::select! {
                ev = rx.recv() => {
                    match ev {
                        Ok(e) if e.method == "_internal.wsClosed" => break,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        _ => {}
                    }
                }
                _ = tokio::time::sleep(step) => {}
            }
        }
    }

    async fn stop(&self) -> Vec<String> {
        self.closed.store(true, Ordering::SeqCst);
        if let Some(h) = self.auto_attach_task.lock().await.take() {
            h.abort();
        }
        let mut sids: Vec<String> = {
            let mut tabs = self.tabs.lock().await;
            let list = tabs.values().map(|t| t.session_id().to_string()).collect();
            tabs.clear();
            list
        };
        {
            let mut sessions = self.sessions_by_target.lock().await;
            for sid in sessions.values() {
                if !sids.contains(sid) {
                    sids.push(sid.clone());
                }
            }
            sessions.clear();
        }
        self.armed_targets.lock().await.clear();
        self.crashed_sessions.lock().await.clear();
        self.api_created_targets.lock().await.clear();
        self.context_configs.lock().await.clear();

        if self.detached.load(Ordering::SeqCst) {
            // Detached / keep_open / externally connected: close our CDP WebSocket immediately
            // so Chromium tears down Fetch/sessions created on this connection, while leaving Chromium alive.
            self.conn.close();
            let _ = self.process.lock().await.take();
            return sids;
        }

        // Ask Chromium to close gracefully first so persistent profiles flush SQLite/Cookies cleanly
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(1500),
            self.conn.call("Browser.close", json!({})),
        )
        .await;
        self.conn.close();
        if let Some(mut proc) = self.process.lock().await.take() {
            proc.wait_graceful_or_kill(std::time::Duration::from_millis(1500));
        }
        sids
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

    fn is_visible<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let nid = self.backend_node_id;
        fut!(py, self.tab, |t| t.is_visible_node(nid).await)
    }

    fn is_enabled<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let nid = self.backend_node_id;
        fut!(py, self.tab, |t| t.is_enabled_node(nid).await)
    }

    fn is_checked<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let nid = self.backend_node_id;
        fut!(py, self.tab, |t| t.is_checked_node(nid).await)
    }

    fn check<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let nid = self.backend_node_id;
        fut!(py, self.tab, |t| t.check_node(nid, true).await)
    }

    fn uncheck<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let nid = self.backend_node_id;
        fut!(py, self.tab, |t| t.check_node(nid, false).await)
    }

    fn select_option<'py>(&self, py: Python<'py>, value: String) -> PyResult<Bound<'py, PyAny>> {
        let nid = self.backend_node_id;
        fut!(py, self.tab, |t| t.select_option_node(nid, &value).await)
    }

    #[pyo3(signature = (file_path=None))]
    fn screenshot<'py>(
        &self,
        py: Python<'py>,
        file_path: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let path = py_to_opt_path(file_path)?;
        let nid = self.backend_node_id;
        fut!(py, self.tab, |t| t.screenshot_node(nid, path).await.map(PyBin))
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
    browser: Option<Weak<BrowserCore>>,
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

    #[getter]
    fn proxy(&self) -> Option<String> {
        self.inner.proxy().map(str::to_string)
    }

    #[getter]
    fn context_id(&self) -> Option<String> {
        self.inner.context_id().map(str::to_string)
    }

    fn add_init_script<'py>(&self, py: Python<'py>, script: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.add_init_script(&script).await)
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (url, method="GET".to_string(), headers=None, body=None, json=None, credentials="include".to_string()))]
    fn fetch<'py>(
        &self,
        py: Python<'py>,
        url: String,
        method: String,
        headers: Option<&Bound<'py, PyAny>>,
        body: Option<String>,
        json: Option<&Bound<'py, PyAny>>,
        credentials: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mut hdrs = match headers {
            Some(h) => py_to_value(h)?,
            None => Value::Object(serde_json::Map::new()),
        };
        let final_body = if let Some(j) = json {
            let jv = py_to_value(j)?;
            if let Some(map) = hdrs.as_object_mut() {
                let has_ct = map.keys().any(|k| k.eq_ignore_ascii_case("content-type"));
                if !has_ct {
                    map.insert("Content-Type".into(), Value::String("application/json".into()));
                }
            }
            Some(jv.to_string())
        } else {
            body
        };
        fut!(py, self.inner, |t| t
            .fetch(&url, &method, Some(hdrs), final_body, &credentials)
            .await
            .map(PyJson))
    }

    // fingerprint & http_tls session sync
    fn fingerprint<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.fingerprint().await.map(PyJson))
    }

    #[pyo3(signature = (path=None))]
    fn storage_state<'py>(
        &self,
        py: Python<'py>,
        path: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let p = py_to_opt_path(path)?;
        fut!(py, self.inner, |t| t.storage_state(p.as_deref()).await.map(PyJson))
    }

    fn load_storage_state<'py>(&self, py: Python<'py>, state_or_path: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let path_opt: Option<String> = py_to_path(state_or_path).ok();
        let val_opt: Option<Value> = if path_opt.is_none() {
            Some(py_to_value(state_or_path)?)
        } else {
            None
        };
        let t = Arc::clone(&self.inner);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let state = match (path_opt, val_opt) {
                (Some(p), _) => {
                    let raw = tokio::fs::read(&p)
                        .await
                        .map_err(|e| rt(DriverError::Io(format!("Lỗi đọc storage_state '{}': {}", p, e))))?;
                    serde_json::from_slice::<Value>(&raw)
                        .map_err(|e| rt(DriverError::InvalidInput(format!("JSON storage_state hỏng: {}", e))))?
                }
                (_, Some(v)) => v,
                _ => return Err(rt(DriverError::InvalidInput("storage_state không hợp lệ".to_string()))),
            };
            t.load_storage_state(state).await.map(PyJson).map_err(rt)
        })
    }

    #[pyo3(signature = (async_mode=true, **kwargs))]
    fn tls_session<'py>(
        slf: Py<Self>,
        py: Python<'py>,
        async_mode: bool,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let m = py.import("best_driver.tls")?;
        m.call_method("from_tab", (slf, async_mode), kwargs)
    }

    #[pyo3(signature = (session, default_url=None))]
    fn sync_from_tls<'py>(
        slf: Py<Self>,
        py: Python<'py>,
        session: Bound<'py, PyAny>,
        default_url: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let m = py.import("best_driver.tls")?;
        m.call_method1("sync_to_tab", (slf, session, default_url))
    }

    // iframes & downloads
    #[pyo3(signature = (selector, timeout_ms=10000))]
    fn iframe<'py>(
        &self,
        py: Python<'py>,
        selector: String,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let browser = self.browser.clone();
        fut!(py, self.inner, |t| t
            .iframe(&selector, timeout_ms)
            .await
            .map(|f| PyTab { inner: Arc::new(f), browser }))
    }

    fn frames<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.frames().await.map(PyJson))
    }

    fn set_download_dir<'py>(
        &self,
        py: Python<'py>,
        download_dir: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let dir = py_to_path(download_dir)?;
        fut!(py, self.inner, |t| t.set_download_dir(&dir).await)
    }

    #[pyo3(signature = (download_dir, timeout_ms=30000))]
    fn wait_for_download<'py>(
        &self,
        py: Python<'py>,
        download_dir: &Bound<'py, PyAny>,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let dir = py_to_path(download_dir)?;
        fut!(py, self.inner, |t| t
            .wait_for_download(&dir, timeout_ms)
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
        let t = Arc::clone(&self.inner);
        let sid = self.inner.session_id().to_string();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let res = t.close().await;
            evict_python_session(&sid);
            res.map_err(rt)
        })
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
        let list: Vec<String> = if let Ok(one) = py_to_path(files) {
            vec![one]
        } else {
            let mut out = Vec::new();
            for item in files.try_iter()? {
                out.push(py_to_path(&item?)?);
            }
            out
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

    #[pyo3(signature = (permissions, origin=None))]
    fn grant_permissions<'py>(
        &self,
        py: Python<'py>,
        permissions: Vec<String>,
        origin: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.grant_permissions(&permissions, origin.as_deref()).await)
    }

    fn get_window_bounds<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.get_window_bounds().await.map(PyJson))
    }

    #[pyo3(signature = (width=None, height=None, left=None, top=None, state=None))]
    fn set_window_bounds<'py>(
        &self,
        py: Python<'py>,
        width: Option<i64>,
        height: Option<i64>,
        left: Option<i64>,
        top: Option<i64>,
        state: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |t| t.set_window_bounds(width, height, left, top, state.as_deref()).await)
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

    fn save_cookies<'py>(&self, py: Python<'py>, path: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let p = py_to_path(path)?;
        fut!(py, self.inner, |t| t.save_cookies(&p).await)
    }

    fn load_cookies<'py>(&self, py: Python<'py>, path: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let p = py_to_path(path)?;
        fut!(py, self.inner, |t| t.load_cookies(&p).await)
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
        file_path: Option<&Bound<'py, PyAny>>,
        full_page: bool,
        format: String,
        quality: Option<i64>,
        selector: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let p = py_to_opt_path(file_path)?;
        fut!(py, self.inner, |t| t
            .screenshot(p, full_page, &format, quality, selector.as_deref())
            .await
            .map(PyBin))
    }

    #[pyo3(signature = (file_path=None, landscape=false))]
    fn pdf<'py>(
        &self,
        py: Python<'py>,
        file_path: Option<&Bound<'py, PyAny>>,
        landscape: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let p = py_to_opt_path(file_path)?;
        fut!(py, self.inner, |t| t.pdf(p, landscape).await.map(PyBin))
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

    fn is_alive<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let b = self.browser.as_ref().and_then(|w| w.upgrade());
        let tid = self.inner.target_id.clone();
        let sid = self.inner.session_id().to_string();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            match b {
                Some(core) => Ok::<bool, PyErr>(core.is_tab_alive(&tid, &sid).await),
                None => Ok(false),
            }
        })
    }

    // network routing, event streams & lazy auto-retrying locators
    #[pyo3(signature = (methods=None))]
    fn _event_stream(&self, methods: Option<Vec<String>>) -> PyEventStream {
        PyEventStream {
            rx: Arc::new(Mutex::new(self.inner.cdp.subscribe())),
            methods: Arc::new(methods),
            closed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn _popup_event_stream(&self) -> PyEventStream {
        PyEventStream {
            rx: Arc::new(Mutex::new(self.inner.cdp.subscribe_popups(
                self.inner.target_id.clone(),
                self.inner.context_id().map(str::to_string),
            ))),
            methods: Arc::new(None),
            closed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    #[pyo3(signature = (target_id, browser_context_id=None))]
    fn _attach_target<'py>(
        &self,
        py: Python<'py>,
        target_id: String,
        browser_context_id: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let Some(core) = self.browser.as_ref().and_then(|w| w.upgrade()) else {
            return Err(rt(DriverError::BrowserClosed));
        };
        let browser = self.browser.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            core.attach_discovered_target(target_id, browser_context_id)
                .await
                .map(|t| PyTab { inner: t, browser })
                .map_err(rt)
        })
    }

    #[pyo3(signature = (timeout_ms=30000))]
    fn expect_popup<'py>(
        slf: Py<Self>,
        py: Python<'py>,
        timeout_ms: u64,
    ) -> PyResult<Bound<'py, PyAny>> {
        let m = py.import("best_driver.events")?;
        m.call_method1("PopupExpectation", (slf, timeout_ms))
    }

    fn listen_popups<'py>(slf: Py<Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let m = py.import("best_driver.events")?;
        m.call_method1("PopupStream", (slf,))
    }

    #[pyo3(signature = (types=None))]
    fn listen_console<'py>(
        slf: Py<Self>,
        py: Python<'py>,
        types: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let m = py.import("best_driver.events")?;
        m.call_method1("ConsoleStream", (slf, types))
    }

    fn listen_page_errors<'py>(slf: Py<Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let m = py.import("best_driver.events")?;
        m.call_method1("PageErrorStream", (slf,))
    }

    fn listen_crashes<'py>(slf: Py<Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let m = py.import("best_driver.events")?;
        m.call_method1("CrashStream", (slf,))
    }

    fn _abort_auth_task(&self) {
        self.inner.abort_auth_task();
    }

    #[pyo3(signature = (url_pattern, handler, stage="request".to_string()))]
    fn route<'py>(
        slf: Py<Self>,
        py: Python<'py>,
        url_pattern: String,
        handler: Bound<'py, PyAny>,
        stage: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let m = py.import("best_driver.network")?;
        m.call_method1("add_route", (slf, url_pattern, handler, stage))
    }

    #[pyo3(signature = (url_pattern=None))]
    fn unroute<'py>(
        slf: Py<Self>,
        py: Python<'py>,
        url_pattern: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let m = py.import("best_driver.network")?;
        m.call_method1("remove_route", (slf, url_pattern))
    }

    #[pyo3(signature = (include_bodies=true))]
    fn record_har<'py>(
        slf: Py<Self>,
        py: Python<'py>,
        include_bodies: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let m = py.import("best_driver.network")?;
        m.call_method1("start_har", (slf, include_bodies))
    }

    #[pyo3(signature = (path=None))]
    fn export_har<'py>(
        slf: Py<Self>,
        py: Python<'py>,
        path: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let p = py_to_opt_path(path)?;
        let m = py.import("best_driver.network")?;
        m.call_method1("export_har", (slf, p))
    }

    fn stop_har<'py>(slf: Py<Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let m = py.import("best_driver.network")?;
        m.call_method1("stop_har", (slf,))
    }

    #[pyo3(signature = (url_pattern=None))]
    fn listen_responses<'py>(
        slf: Py<Self>,
        py: Python<'py>,
        url_pattern: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let m = py.import("best_driver.network")?;
        m.call_method1("ResponseStream", (slf, url_pattern))
    }

    #[pyo3(signature = (url_pattern=None))]
    fn listen_requests<'py>(
        slf: Py<Self>,
        py: Python<'py>,
        url_pattern: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let m = py.import("best_driver.network")?;
        m.call_method1("RequestStream", (slf, url_pattern))
    }

    fn locator<'py>(slf: Py<Self>, py: Python<'py>, query: String) -> PyResult<Bound<'py, PyAny>> {
        let m = py.import("best_driver.locator")?;
        m.call_method1("Locator", (slf, vec![query]))
    }
}

// ───────────────────────────── EventStream ─────────────────────────────

#[pyclass(name = "EventStream")]
pub struct PyEventStream {
    rx: Arc<Mutex<SessionEventRx>>,
    methods: Arc<Option<Vec<String>>>,
    closed: Arc<std::sync::atomic::AtomicBool>,
}

#[pymethods]
impl PyEventStream {
    #[pyo3(signature = (timeout_ms=None))]
    fn next<'py>(&self, py: Python<'py>, timeout_ms: Option<u64>) -> PyResult<Bound<'py, PyAny>> {
        let rx = Arc::clone(&self.rx);
        let methods = Arc::clone(&self.methods);
        let closed = Arc::clone(&self.closed);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let deadline = timeout_ms.map(|ms| std::time::Instant::now() + std::time::Duration::from_millis(ms));
            let mut guard = rx.lock().await;
            loop {
                if closed.load(std::sync::atomic::Ordering::Relaxed) {
                    return Ok(PyJson(Value::Null));
                }
                let ev = if let Some(dl) = deadline {
                    let now = std::time::Instant::now();
                    if now >= dl {
                        return Ok(PyJson(Value::Null));
                    }
                    match tokio::time::timeout(dl - now, guard.recv()).await {
                        Ok(r) => r,
                        Err(_) => return Ok(PyJson(Value::Null)),
                    }
                } else {
                    guard.recv().await
                };
                match ev {
                    Ok((method, params)) => {
                        let matched = match methods.as_ref() {
                            Some(list) => list.iter().any(|m| m == &method),
                            None => true,
                        };
                        if matched {
                            return Ok(PyJson(json!({ "method": method, "params": params })));
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => return Ok(PyJson(Value::Null)),
                }
            }
        })
    }

    fn close(&self) {
        self.closed.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

// ───────────────────────────── BrowserContext ─────────────────────────────

#[pyclass(name = "BrowserContext")]
pub struct PyBrowserContext {
    browser: Arc<BrowserCore>,
    context_id: String,
    proxy: Option<String>,
    proxy_auth: Option<ProxyAuth>,
    options: ContextOptions,
}

#[pymethods]
impl PyBrowserContext {
    fn __repr__(&self) -> String {
        format!("<BrowserContext id={} proxy={:?}>", self.context_id, self.proxy)
    }

    #[getter]
    fn context_id(&self) -> String {
        self.context_id.clone()
    }

    #[getter]
    fn proxy(&self) -> Option<String> {
        self.proxy.clone()
    }

    fn add_init_script<'py>(&self, py: Python<'py>, script: String) -> PyResult<Bound<'py, PyAny>> {
        let b = Arc::clone(&self.browser);
        let cid = self.context_id.clone();
        let proxy = self.proxy.clone();
        let auth = self.proxy_auth.clone();
        let opts = self.options.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            opts.init_scripts.lock().await.push(script.clone());
            if let Ok(tabs) = b.list_tabs_in_context(&cid, proxy, auth, &opts).await {
                for t in tabs {
                    let _ = t.add_init_script(&script).await;
                }
            }
            Ok::<(), PyErr>(())
        })
    }

    #[pyo3(signature = (url="about:blank".to_string()))]
    fn new_tab<'py>(&self, py: Python<'py>, url: String) -> PyResult<Bound<'py, PyAny>> {
        let b = Arc::clone(&self.browser);
        let cid = self.context_id.clone();
        let proxy = self.proxy.clone();
        let auth = self.proxy_auth.clone();
        let opts = self.options.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let bw = Some(Arc::downgrade(&b));
            b.new_tab_in_context(&cid, &url, proxy, auth, &opts)
                .await
                .map(|t| PyTab { inner: t, browser: bw })
                .map_err(rt)
        })
    }

    fn main_tab<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let b = Arc::clone(&self.browser);
        let cid = self.context_id.clone();
        let proxy = self.proxy.clone();
        let auth = self.proxy_auth.clone();
        let opts = self.options.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let bw = Some(Arc::downgrade(&b));
            let tabs = b
                .list_tabs_in_context(&cid, proxy.clone(), auth.clone(), &opts)
                .await
                .map_err(rt)?;
            match tabs.into_iter().next() {
                Some(t) => Ok(PyTab { inner: t, browser: bw }),
                None => b
                    .new_tab_in_context(&cid, "about:blank", proxy, auth, &opts)
                    .await
                    .map(|t| PyTab { inner: t, browser: bw })
                    .map_err(rt),
            }
        })
    }

    fn tabs<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let b = Arc::clone(&self.browser);
        let cid = self.context_id.clone();
        let proxy = self.proxy.clone();
        let auth = self.proxy_auth.clone();
        let opts = self.options.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let bw = Arc::downgrade(&b);
            b.list_tabs_in_context(&cid, proxy, auth, &opts)
                .await
                .map(|v| {
                    v.into_iter()
                        .map(|t| PyTab { inner: t, browser: Some(bw.clone()) })
                        .collect::<Vec<_>>()
                })
                .map_err(rt)
        })
    }

    #[pyo3(signature = (url, new_tab=false, wait_until="load".to_string()))]
    fn get<'py>(
        &self,
        py: Python<'py>,
        url: String,
        new_tab: bool,
        wait_until: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let b = Arc::clone(&self.browser);
        let cid = self.context_id.clone();
        let proxy = self.proxy.clone();
        let auth = self.proxy_auth.clone();
        let opts = self.options.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let bw = Some(Arc::downgrade(&b));
            let tab = if new_tab {
                b.new_tab_in_context(&cid, "about:blank", proxy, auth, &opts)
                    .await
                    .map_err(rt)?
            } else {
                let existing = b
                    .list_tabs_in_context(&cid, proxy.clone(), auth.clone(), &opts)
                    .await
                    .map_err(rt)?;
                match existing.into_iter().next() {
                    Some(t) => t,
                    None => b
                        .new_tab_in_context(&cid, "about:blank", proxy, auth, &opts)
                        .await
                        .map_err(rt)?,
                }
            };
            tab.get(&url, None, &wait_until).await.map_err(rt)?;
            Ok(PyTab { inner: tab, browser: bw })
        })
    }

    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let b = Arc::clone(&self.browser);
        let cid = self.context_id.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let res = b.dispose_context(&cid).await;
            evict_python_context(&cid);
            res.map_err(rt)
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
        let b = Arc::clone(&self.browser);
        let cid = self.context_id.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let _ = b.dispose_context(&cid).await;
            evict_python_context(&cid);
            Ok::<bool, PyErr>(false)
        })
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

    #[getter]
    fn proxy(&self) -> Option<String> {
        self.inner.proxy.clone()
    }

    fn is_alive<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |c| Ok::<bool, DriverError>(c.is_alive().await))
    }

    fn detach(&self) {
        self.inner.detach();
    }

    #[pyo3(signature = (poll_ms=250))]
    fn wait_until_closed<'py>(&self, py: Python<'py>, poll_ms: u64) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |c| {
            c.wait_until_closed(poll_ms).await;
            Ok::<(), DriverError>(())
        })
    }

    fn main_tab<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |c| {
            let bw = Some(Arc::downgrade(&c));
            c.main_tab().await.map(|t| PyTab { inner: t, browser: bw })
        })
    }

    #[pyo3(signature = (url="about:blank".to_string()))]
    fn new_tab<'py>(&self, py: Python<'py>, url: String) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |c| {
            let bw = Some(Arc::downgrade(&c));
            c.new_tab(&url).await.map(|t| PyTab { inner: t, browser: bw })
        })
    }

    fn tabs<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        fut!(py, self.inner, |c| {
            let bw = Arc::downgrade(&c);
            c.list_tabs().await.map(|v| {
                v.into_iter()
                    .map(|t| PyTab { inner: t, browser: Some(bw.clone()) })
                    .collect::<Vec<_>>()
            })
        })
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
            let bw = Some(Arc::downgrade(&c));
            let tab = if new_tab {
                c.new_tab("about:blank").await?
            } else {
                c.main_tab().await?
            };
            tab.get(&url, None, &wait_until).await?;
            Ok::<PyTab, DriverError>(PyTab { inner: tab, browser: bw })
        }
        .await)
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (
        proxy=None,
        proxy_bypass_list=None,
        user_agent=None,
        accept_language=None,
        platform=None,
        viewport=None,
        locale=None,
        timezone=None,
        extra_headers=None
    ))]
    fn new_context<'py>(
        &self,
        py: Python<'py>,
        proxy: Option<String>,
        proxy_bypass_list: Option<String>,
        user_agent: Option<String>,
        accept_language: Option<String>,
        platform: Option<String>,
        viewport: Option<&Bound<'py, PyDict>>,
        locale: Option<String>,
        timezone: Option<String>,
        extra_headers: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let parsed_vp = match viewport {
            Some(d) => {
                let w: i64 = d
                    .get_item("width")?
                    .ok_or_else(|| rt(DriverError::InvalidInput("viewport thiếu 'width'".into())))?
                    .extract()?;
                let h: i64 = d
                    .get_item("height")?
                    .ok_or_else(|| rt(DriverError::InvalidInput("viewport thiếu 'height'".into())))?
                    .extract()?;
                let scale: f64 = d
                    .get_item("scale")?
                    .map(|v| v.extract())
                    .transpose()?
                    .unwrap_or(1.0);
                let mobile: bool = d
                    .get_item("mobile")?
                    .map(|v| v.extract())
                    .transpose()?
                    .unwrap_or(false);
                Some((w, h, scale, mobile))
            }
            None => None,
        };
        let parsed_headers = match extra_headers {
            Some(h) => Some(py_to_value(h)?),
            None => None,
        };
        let opts = ContextOptions {
            user_agent,
            accept_language,
            platform,
            viewport: parsed_vp,
            locale,
            timezone,
            extra_headers: parsed_headers,
            init_scripts: Arc::new(Mutex::new(Vec::new())),
        };
        let b = Arc::clone(&self.inner);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let (cid, raw_proxy, auth) = b
                .create_context(proxy, proxy_bypass_list)
                .await
                .map_err(rt)?;
            b.context_configs.lock().await.insert(
                cid.clone(),
                (raw_proxy.clone(), auth.clone(), opts.clone()),
            );
            Ok(PyBrowserContext {
                browser: b,
                context_id: cid,
                proxy: raw_proxy,
                proxy_auth: auth,
                options: opts,
            })
        })
    }

    fn stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let c = Arc::clone(&self.inner);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let sids = c.stop().await;
            evict_python_sessions(&sids);
            Ok::<(), PyErr>(())
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
        let c = Arc::clone(&self.inner);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let sids = c.stop().await;
            evict_python_sessions(&sids);
            Ok::<bool, PyErr>(false)
        })
    }
}

/// Launch a browser over a single flattened browser-level CDP WebSocket.
#[allow(clippy::too_many_arguments)]
#[pyfunction]
#[pyo3(signature = (headless=false, user_data_dir=None, browser_path=None, extra_args=None, proxy=None, keep_open=false, start_url=None))]
fn start<'py>(
    py: Python<'py>,
    headless: bool,
    user_data_dir: Option<&Bound<'py, PyAny>>,
    browser_path: Option<&Bound<'py, PyAny>>,
    extra_args: Option<Vec<String>>,
    proxy: Option<String>,
    keep_open: bool,
    start_url: Option<String>,
) -> PyResult<Bound<'py, PyAny>> {
    let data_dir = py_to_opt_path(user_data_dir)?;
    let bin_path = py_to_opt_path(browser_path)?;
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let core = BrowserCore::launch(
            headless,
            data_dir,
            bin_path,
            extra_args,
            proxy,
            keep_open,
            start_url,
        )
        .await
        .map_err(rt)?;
        Ok(PyBrowser {
            inner: Arc::new(core),
        })
    })
}

/// Connect to an already-running Chromium instance over CDP (`port`, `"http://host:port"`, or `"ws://..."`).
#[pyfunction]
#[pyo3(signature = (endpoint, proxy=None))]
fn connect<'py>(
    py: Python<'py>,
    endpoint: &Bound<'py, PyAny>,
    proxy: Option<String>,
) -> PyResult<Bound<'py, PyAny>> {
    let ep: String = if let Ok(p) = endpoint.extract::<u16>() {
        p.to_string()
    } else if let Ok(s) = endpoint.extract::<String>() {
        s
    } else {
        return Err(rt(DriverError::InvalidInput(
            "endpoint phải là số port (int) hoặc chuỗi URL/host:port (str)".to_string(),
        )));
    };
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let core = BrowserCore::connect_existing(&ep, proxy).await.map_err(rt)?;
        Ok(PyBrowser {
            inner: Arc::new(core),
        })
    })
}

#[pymodule]
fn best_driver(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyBrowser>()?;
    m.add_class::<PyBrowserContext>()?;
    m.add_class::<PyTab>()?;
    m.add_class::<PyElement>()?;
    m.add_class::<PyEventStream>()?;
    m.add_function(wrap_pyfunction!(start, m)?)?;
    m.add_function(wrap_pyfunction!(connect, m)?)?;
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
