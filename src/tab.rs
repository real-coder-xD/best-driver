// src/tab.rs — Rust 2021: per-tab CDP controller over a flattened CdpSession
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use base64::prelude::*;
use serde_json::{json, Map, Value};
use tokio::task::JoinHandle;

use crate::browser::derive_tls_preset;
use crate::cdp::{CdpSession, SessionEventRx};
use crate::error::{DriverError, DriverResult};
use crate::input::{bezier_trajectory, jitter, key_def, swipe_trajectory};
use crate::locator::{
    child_sel_all_fn, child_sel_fn, find_pierced_backend_node_ids, js_str, sel_all_expr, sel_expr,
    NODE_META_FN,
};

pub const EVASION_JS: &str = include_str!("evasion.js");

pub struct Tab {
    pub target_id: String,
    pub cdp: Arc<CdpSession>,
    proxy: Option<String>,
    browser_context_id: Option<String>,
    frame_chain: Vec<String>,
    mouse: StdMutex<(f64, f64)>,
    last_remote_ip: StdMutex<Option<String>>,
    dialog_task: StdMutex<Option<JoinHandle<()>>>,
    auth_task: StdMutex<Option<JoinHandle<()>>>,
}

impl Drop for Tab {
    fn drop(&mut self) {
        for slot in [&self.dialog_task, &self.auth_task] {
            if let Ok(mut g) = slot.lock() {
                if let Some(h) = g.take() {
                    h.abort();
                }
            }
        }
    }
}

impl Tab {
    pub async fn new(
        target_id: String,
        cdp: Arc<CdpSession>,
        proxy: Option<String>,
        browser_context_id: Option<String>,
    ) -> DriverResult<Self> {
        // Intentionally omit `Runtime.enable` so V8 never fires console serialization hooks
        // probed by anti-bot scripts (`console.debug(err)` / `Error.stack` getter traps).
        cdp.call("Page.enable", json!({})).await?;
        cdp.call("DOM.enable", json!({})).await?;
        cdp.call("Network.enable", json!({})).await?;

        Ok(Self {
            target_id,
            cdp,
            proxy,
            browser_context_id,
            frame_chain: Vec::new(),
            mouse: StdMutex::new((0.0, 0.0)),
            last_remote_ip: StdMutex::new(None),
            dialog_task: StdMutex::new(None),
            auth_task: StdMutex::new(None),
        })
    }

    pub fn session_id(&self) -> &str {
        &self.cdp.session_id
    }

    pub fn frame_id(&self) -> Option<&str> {
        self.frame_chain.last().map(String::as_str)
    }

    pub fn proxy(&self) -> Option<&str> {
        self.proxy.as_deref()
    }

    pub fn context_id(&self) -> Option<&str> {
        self.browser_context_id.as_deref()
    }

    // ───────────────────────────── Navigation ─────────────────────────────

    fn record_remote_ip(&self, method: &str, params: &Value) {
        if method == "Network.responseReceived"
            && params.get("type").and_then(|v| v.as_str()) == Some("Document")
        {
            if let Some(ip) = params.pointer("/response/remoteIPAddress").and_then(|v| v.as_str()) {
                if !ip.is_empty() {
                    if let Ok(mut g) = self.last_remote_ip.lock() {
                        *g = Some(ip.to_string());
                    }
                }
            }
        }
    }

    pub async fn get(&self, url: &str, timeout_ms: Option<u64>, wait_until: &str) -> DriverResult<()> {
        let t_ms = timeout_ms.unwrap_or(30000);
        if let Ok(mut g) = self.last_remote_ip.lock() {
            *g = None;
        }
        // Subscribe before navigating so fast load events are never missed
        let mut rx = self.cdp.subscribe();
        let res = self.cdp.call("Page.navigate", json!({ "url": url })).await?;
        if let Some(err) = res.get("errorText").and_then(|v| v.as_str()) {
            if !err.is_empty() {
                return Err(DriverError::Connection(format!(
                    "Điều hướng thất bại '{}': {}",
                    url, err
                )));
            }
        }
        // Same-document navigation (hash change) fires no load event
        if res.get("loaderId").is_none() {
            return Ok(());
        }
        self.wait_events(&mut rx, wait_until, t_ms).await
    }

    async fn wait_events(&self, rx: &mut SessionEventRx, wait_until: &str, t_ms: u64) -> DriverResult<()> {
        match wait_until {
            "none" | "commit" => Ok(()),
            "domcontentloaded" => {
                let _ = CdpSession::wait_until(rx, t_ms, |m, p| {
                    self.record_remote_ip(m, p);
                    m == "Page.domContentEventFired"
                })
                .await;
                Ok(())
            }
            "networkidle" => {
                let start = Instant::now();
                let _ = CdpSession::wait_until(rx, t_ms, |m, p| {
                    self.record_remote_ip(m, p);
                    m == "Page.loadEventFired"
                })
                .await;
                let left = t_ms.saturating_sub(start.elapsed().as_millis() as u64);
                self.wait_network_idle_rx(rx, 500, left).await
            }
            _ => {
                let _ = CdpSession::wait_until(rx, t_ms, |m, p| {
                    self.record_remote_ip(m, p);
                    m == "Page.loadEventFired"
                })
                .await;
                Ok(())
            }
        }
    }

    /// Resolves once no Network.* event has arrived for `idle_ms`.
    async fn wait_network_idle_rx(
        &self,
        rx: &mut SessionEventRx,
        idle_ms: u64,
        timeout_ms: u64,
    ) -> DriverResult<()> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(());
            }
            let window = Duration::from_millis(idle_ms).min(deadline - now);
            match tokio::time::timeout(window, rx.recv()).await {
                Err(_) => return Ok(()),
                Ok(Ok((m, p))) if m.starts_with("Network.") => {
                    self.record_remote_ip(&m, &p);
                    continue;
                }
                Ok(Ok(_)) => continue,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(_)) => return Err(DriverError::Connection("Kênh sự kiện CDP đã đóng".to_string())),
            }
        }
    }

    pub async fn wait_for_load(&self, state: &str, timeout_ms: u64) -> DriverResult<()> {
        let mut rx = self.cdp.subscribe();
        let ready = self.evaluate_string("document.readyState").await.unwrap_or_default();
        match state {
            "networkidle" => self.wait_network_idle_rx(&mut rx, 500, timeout_ms).await,
            "domcontentloaded" if ready != "loading" => Ok(()),
            "load" if ready == "complete" => Ok(()),
            other => self.wait_events(&mut rx, other, timeout_ms).await,
        }
    }

    pub async fn reload(&self, ignore_cache: bool, timeout_ms: u64) -> DriverResult<()> {
        let mut rx = self.cdp.subscribe();
        self.cdp.call("Page.reload", json!({ "ignoreCache": ignore_cache })).await?;
        self.wait_events(&mut rx, "load", timeout_ms).await
    }

    async fn history_step(&self, delta: i64, timeout_ms: u64) -> DriverResult<bool> {
        let hist = self.cdp.call("Page.getNavigationHistory", json!({})).await?;
        let idx = hist.get("currentIndex").and_then(|v| v.as_i64()).unwrap_or(0) + delta;
        let entries = hist.get("entries").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        if idx < 0 || idx as usize >= entries.len() {
            return Ok(false);
        }
        let entry_id = entries[idx as usize].get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        let mut rx = self.cdp.subscribe();
        self.cdp.call("Page.navigateToHistoryEntry", json!({ "entryId": entry_id })).await?;
        self.wait_events(&mut rx, "load", timeout_ms).await?;
        Ok(true)
    }

    pub async fn back(&self, timeout_ms: u64) -> DriverResult<bool> {
        self.history_step(-1, timeout_ms).await
    }

    pub async fn forward(&self, timeout_ms: u64) -> DriverResult<bool> {
        self.history_step(1, timeout_ms).await
    }

    pub async fn bring_to_front(&self) -> DriverResult<()> {
        self.cdp.call("Page.bringToFront", json!({})).await.map(|_| ())
    }

    pub async fn close(&self) -> DriverResult<()> {
        for slot in [&self.dialog_task, &self.auth_task] {
            if let Ok(mut g) = slot.lock() {
                if let Some(h) = g.take() {
                    h.abort();
                }
            }
        }
        self.cdp
            .call_browser("Target.closeTarget", json!({ "targetId": self.target_id }))
            .await
            .map(|_| ())
    }

    // ───────────────────────────── JS / Reading ─────────────────────────────

    /// Evaluates JS in the top document or, for an in-process iframe handle, inside that frame's isolated world.
    async fn eval_raw(&self, expression: &str, return_by_value: bool) -> DriverResult<Value> {
        let mut params = Map::new();
        params.insert("expression".into(), json!(expression));
        params.insert("returnByValue".into(), json!(return_by_value));
        params.insert("awaitPromise".into(), json!(true));
        params.insert("userGesture".into(), json!(true));

        if let Some(fid) = self.frame_id() {
            let world = self
                .cdp
                .call(
                    "Page.createIsolatedWorld",
                    json!({
                        "frameId": fid,
                        "worldName": "",
                        "grantUniveralAccess": true
                    }),
                )
                .await?;
            let ctx_id = world
                .get("executionContextId")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| DriverError::NotFound(format!("Không tạo được executionContext cho iframe '{}'", fid)))?;
            params.insert("contextId".into(), json!(ctx_id));
        }

        let res = self.cdp.call("Runtime.evaluate", Value::Object(params)).await?;
        if let Some(ex) = res.get("exceptionDetails") {
            let msg = ex
                .pointer("/exception/description")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| ex.to_string());
            return Err(DriverError::JsException(msg));
        }
        Ok(res)
    }

    pub async fn evaluate(&self, expression: &str) -> DriverResult<Value> {
        let res = self.eval_raw(expression, true).await?;
        Ok(res.pointer("/result/value").cloned().unwrap_or(Value::Null))
    }

    pub async fn evaluate_string(&self, expression: &str) -> DriverResult<String> {
        Ok(match self.evaluate(expression).await? {
            Value::String(s) => s,
            Value::Null => String::new(),
            other => other.to_string(),
        })
    }

    pub async fn content(&self) -> DriverResult<String> {
        self.evaluate_string("document.documentElement.outerHTML").await
    }

    pub async fn title(&self) -> DriverResult<String> {
        self.evaluate_string("document.title").await
    }

    pub async fn current_url(&self) -> DriverResult<String> {
        self.evaluate_string("window.location.href").await
    }

    pub async fn exists(&self, selector: &str) -> DriverResult<bool> {
        if self.evaluate(&format!("{} !== null", sel_expr(selector))).await? == Value::Bool(true) {
            return Ok(true);
        }
        Ok(!self.pierce_find_ids(selector).await.unwrap_or_default().is_empty())
    }

    pub async fn count(&self, selector: &str) -> DriverResult<i64> {
        let v = self.evaluate(&format!("{}.length", sel_all_expr(selector))).await?;
        let n = v.as_i64().unwrap_or(0);
        if n > 0 {
            return Ok(n);
        }
        Ok(self.pierce_find_ids(selector).await.unwrap_or_default().len() as i64)
    }

    pub async fn text(&self, selector: &str) -> DriverResult<Option<String>> {
        let v = self.evaluate(&format!(
            "(() => {{ const el = {}; return el ? (el.innerText ?? el.textContent ?? '').trim() : null; }})()",
            sel_expr(selector)
        )).await?;
        if let Some(s) = v.as_str() {
            return Ok(Some(s.to_string()));
        }
        if let Some(&nid) = self.pierce_find_ids(selector).await.unwrap_or_default().first() {
            let txt = self.call_on_node(
                nid,
                "function() { return (this.innerText ?? this.textContent ?? '').trim(); }",
                vec![],
            ).await?;
            return Ok(txt.as_str().map(str::to_string));
        }
        Ok(None)
    }

    pub async fn text_all(&self, selector: &str) -> DriverResult<Vec<String>> {
        let v = self.evaluate(&format!(
            "{}.map(e => (e.innerText ?? e.textContent ?? '').trim())",
            sel_all_expr(selector)
        )).await?;
        Ok(v.as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default())
    }

    pub async fn attr(&self, selector: &str, name: &str) -> DriverResult<Option<String>> {
        let v = self.evaluate(&format!(
            "(() => {{ const el = {}; return el ? el.getAttribute({}) : null; }})()",
            sel_expr(selector),
            js_str(name)
        )).await?;
        Ok(v.as_str().map(str::to_string))
    }

    pub async fn attr_all(&self, selector: &str, name: &str) -> DriverResult<Vec<Option<String>>> {
        let v = self.evaluate(&format!(
            "{}.map(e => e.getAttribute({}))",
            sel_all_expr(selector),
            js_str(name)
        )).await?;
        Ok(v.as_array()
            .map(|a| a.iter().map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default())
    }

    pub async fn value(&self, selector: &str) -> DriverResult<Option<String>> {
        let v = self.evaluate(&format!(
            "(() => {{ const el = {}; return el && 'value' in el ? String(el.value) : null; }})()",
            sel_expr(selector)
        )).await?;
        Ok(v.as_str().map(str::to_string))
    }

    pub async fn inner_html(&self, selector: &str) -> DriverResult<Option<String>> {
        let v = self.evaluate(&format!(
            "(() => {{ const el = {}; return el ? el.innerHTML : null; }})()",
            sel_expr(selector)
        )).await?;
        Ok(v.as_str().map(str::to_string))
    }

    // ───────────────────────────── Waiting ─────────────────────────────

    /// Polls a JS expression until it is truthy; returns its value.
    pub async fn wait_for_function(&self, expression: &str, timeout_ms: u64, poll_ms: u64) -> DriverResult<Value> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let wrapped = format!(
            "(() => {{ try {{ return ({}); }} catch (e) {{ return null; }} }})()",
            expression
        );
        loop {
            if let Ok(v) = self.evaluate(&wrapped).await {
                let truthy = match &v {
                    Value::Null => false,
                    Value::Bool(b) => *b,
                    Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
                    Value::String(s) => !s.is_empty(),
                    _ => true,
                };
                if truthy {
                    return Ok(v);
                }
            }
            if Instant::now() >= deadline {
                return Err(DriverError::Timeout(format!(
                    "{}ms chờ điều kiện: {}",
                    timeout_ms, expression
                )));
            }
            tokio::time::sleep(Duration::from_millis(poll_ms)).await;
        }
    }

    pub async fn wait_for_selector(&self, selector: &str, timeout_ms: u64, visible: bool) -> DriverResult<()> {
        let expr = if visible {
            format!(
                "(() => {{ const el = {}; if (!el) return false; const s = getComputedStyle(el); \
                 return s.visibility !== 'hidden' && s.display !== 'none' && el.getClientRects().length > 0; }})()",
                sel_expr(selector)
            )
        } else {
            format!("{} !== null", sel_expr(selector))
        };
        self.wait_for_function(&expr, timeout_ms, 100)
            .await
            .map(|_| ())
            .map_err(|_| DriverError::Timeout(format!("Chờ phần tử '{}' quá {}ms", selector, timeout_ms)))
    }

    pub async fn wait_for_hidden(&self, selector: &str, timeout_ms: u64) -> DriverResult<()> {
        let expr = format!(
            "(() => {{ const el = {}; if (!el) return true; const s = getComputedStyle(el); \
             return s.visibility === 'hidden' || s.display === 'none' || el.getClientRects().length === 0; }})()",
            sel_expr(selector)
        );
        self.wait_for_function(&expr, timeout_ms, 100)
            .await
            .map(|_| ())
            .map_err(|_| DriverError::Timeout(format!("Chờ phần tử '{}' biến mất quá {}ms", selector, timeout_ms)))
    }

    pub async fn wait_for_text(&self, text: &str, timeout_ms: u64) -> DriverResult<()> {
        let expr = format!("document.body && document.body.innerText.includes({})", js_str(text));
        self.wait_for_function(&expr, timeout_ms, 150)
            .await
            .map(|_| ())
            .map_err(|_| DriverError::Timeout(format!("Chờ văn bản '{}' quá {}ms", text, timeout_ms)))
    }

    pub async fn wait_for_url(&self, contains: &str, timeout_ms: u64) -> DriverResult<String> {
        let expr = format!("location.href.includes({}) ? location.href : null", js_str(contains));
        self.wait_for_function(&expr, timeout_ms, 150)
            .await
            .map(|v| v.as_str().unwrap_or_default().to_string())
            .map_err(|_| DriverError::Timeout(format!("Chờ URL chứa '{}' quá {}ms", contains, timeout_ms)))
    }

    // ───────────────────────────── Mouse ─────────────────────────────

    fn jittered_center(pos: &Value) -> Option<(f64, f64)> {
        let x = pos.get("x").and_then(|v| v.as_f64())?;
        let y = pos.get("y").and_then(|v| v.as_f64())?;
        let w = pos.get("w").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let h = pos.get("h").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let ox = (jitter(0, 1000).as_millis() as f64 / 1000.0 - 0.5) * (w * 0.3).min(10.0);
        let oy = (jitter(0, 1000).as_millis() as f64 / 1000.0 - 0.5) * (h * 0.3).min(6.0);
        Some((x + ox, y + oy))
    }

    /// Accumulates top-level viewport offsets of ancestor `<iframe>` elements so mouse events land accurately inside nested frames.
    async fn frame_viewport_offset(&self) -> (f64, f64) {
        if self.frame_chain.is_empty() {
            return (0.0, 0.0);
        }
        let mut ox = 0.0;
        let mut oy = 0.0;
        for fid in &self.frame_chain {
            let Ok(owner) = self.cdp.call("DOM.getFrameOwner", json!({ "frameId": fid })).await else {
                continue;
            };
            let Some(nid) = owner.get("backendNodeId").and_then(|v| v.as_i64()) else {
                continue;
            };
            if let Ok(rect) = self.call_on_node(
                nid,
                "function() { \
                    this.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: 'instant' }); \
                    const r = this.getBoundingClientRect(); \
                    const s = getComputedStyle(this); \
                    const bl = parseFloat(s.borderLeftWidth) || 0; \
                    const bt = parseFloat(s.borderTopWidth) || 0; \
                    const pl = parseFloat(s.paddingLeft) || 0; \
                    const pt = parseFloat(s.paddingTop) || 0; \
                    return { x: r.left + bl + pl, y: r.top + bt + pt }; \
                }",
                vec![],
            ).await {
                ox += rect.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                oy += rect.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
            }
        }
        (ox, oy)
    }

    async fn element_center(&self, el_expr: &str, what: &str) -> DriverResult<(f64, f64)> {
        let (ox, oy) = self.frame_viewport_offset().await;
        let expr = format!(
            "(() => {{ const el = {}; if (!el) return null; \
             el.scrollIntoView({{ block: 'center', inline: 'center', behavior: 'instant' }}); \
             const r = el.getBoundingClientRect(); \
             if (r.width === 0 && r.height === 0) return null; \
             return {{ x: r.left + r.width / 2, y: r.top + r.height / 2, w: r.width, h: r.height }}; }})()",
            el_expr
        );
        let pos = self.evaluate(&expr).await?;
        let (x, y) = Self::jittered_center(&pos).ok_or_else(|| DriverError::NotFound(what.to_string()))?;
        Ok((x + ox, y + oy))
    }

    /// Moves the cursor along a biological cubic Bézier curve with speed easing and micro-tremors.
    pub async fn mouse_move(&self, x: f64, y: f64) -> DriverResult<()> {
        let (sx, sy) = *self.mouse.lock().map_err(|e| DriverError::Connection(e.to_string()))?;
        let points = bezier_trajectory(sx, sy, x, y);
        for (bx, by, base_sleep) in points {
            self.cdp.call("Input.dispatchMouseEvent", json!({
                "type": "mouseMoved",
                "x": bx,
                "y": by
            })).await?;
            tokio::time::sleep(jitter(base_sleep, 6)).await;
        }
        *self.mouse.lock().map_err(|e| DriverError::Connection(e.to_string()))? = (x, y);
        Ok(())
    }

    pub async fn mouse_click(&self, x: f64, y: f64, button: &str, click_count: i64) -> DriverResult<()> {
        self.mouse_move(x, y).await?;
        tokio::time::sleep(jitter(40, 60)).await;
        for n in 1..=click_count {
            self.cdp.call("Input.dispatchMouseEvent", json!({
                "type": "mousePressed", "button": button, "clickCount": n, "x": x, "y": y
            })).await?;
            tokio::time::sleep(jitter(50, 50)).await;
            self.cdp.call("Input.dispatchMouseEvent", json!({
                "type": "mouseReleased", "button": button, "clickCount": n, "x": x, "y": y
            })).await?;
            if n < click_count {
                tokio::time::sleep(jitter(60, 40)).await;
            }
        }
        Ok(())
    }

    pub async fn click(&self, selector: &str, button: &str, click_count: i64, timeout_ms: u64) -> DriverResult<()> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let visible_expr = format!(
            "(() => {{ const el = {}; if (!el) return false; const s = getComputedStyle(el); \
             return s.visibility !== 'hidden' && s.display !== 'none' && el.getClientRects().length > 0; }})()",
            sel_expr(selector)
        );
        loop {
            if self.evaluate(&visible_expr).await.ok() == Some(Value::Bool(true)) {
                let (x, y) = self.element_center(&sel_expr(selector), selector).await?;
                return self.mouse_click(x, y, button, click_count).await;
            }
            if let Ok(ids) = self.pierce_find_ids(selector).await {
                for nid in ids {
                    if self.click_node(nid, button, click_count).await.is_ok() {
                        return Ok(());
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err(DriverError::Timeout(format!("Chờ phần tử '{}' quá {}ms", selector, timeout_ms)));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Finds the deepest visible element (including inside Shadow DOM) whose text matches `text` and clicks it.
    pub async fn click_text(&self, text: &str, exact: bool, timeout_ms: u64) -> DriverResult<()> {
        let finder = format!(
            "(() => {{ const t = {}.trim().toLowerCase(); const exact = {}; let best = null; \
             function walk(root) {{ \
               for (const el of root.querySelectorAll('*')) {{ \
                 if (el.shadowRoot) walk(el.shadowRoot); \
                 if (el.getClientRects().length === 0) continue; \
                 const s = (el.innerText || el.value || el.getAttribute('aria-label') || '').trim().toLowerCase(); \
                 if (exact ? s === t : s.includes(t)) {{ if (!best || best.contains(el)) best = el; }} \
               }} \
             }} \
             walk(document.body || document); return best; }})()",
            js_str(text), exact
        );
        self.wait_for_function(&format!("{} !== null", finder), timeout_ms, 150)
            .await
            .map_err(|_| DriverError::Timeout(format!("Tìm phần tử chứa văn bản '{}' quá {}ms", text, timeout_ms)))?;
        let (x, y) = self.element_center(&finder, text).await?;
        self.mouse_click(x, y, "left", 1).await
    }

    pub async fn hover(&self, selector: &str, timeout_ms: u64) -> DriverResult<()> {
        self.wait_for_selector(selector, timeout_ms, true).await?;
        let (x, y) = self.element_center(&sel_expr(selector), selector).await?;
        self.mouse_move(x, y).await
    }

    pub async fn drag(&self, from_selector: &str, to_selector: &str, timeout_ms: u64) -> DriverResult<()> {
        self.wait_for_selector(from_selector, timeout_ms, true).await?;
        self.wait_for_selector(to_selector, timeout_ms, true).await?;
        let (fx, fy) = self.element_center(&sel_expr(from_selector), from_selector).await?;
        self.mouse_move(fx, fy).await?;
        self.cdp.call("Input.dispatchMouseEvent", json!({
            "type": "mousePressed", "button": "left", "clickCount": 1, "x": fx, "y": fy
        })).await?;
        let (tx, ty) = self.element_center(&sel_expr(to_selector), to_selector).await?;
        let steps = 15;
        for i in 1..=steps {
            let t = i as f64 / steps as f64;
            self.cdp.call("Input.dispatchMouseEvent", json!({
                "type": "mouseMoved", "button": "left", "buttons": 1,
                "x": fx + (tx - fx) * t, "y": fy + (ty - fy) * t
            })).await?;
            tokio::time::sleep(jitter(12, 10)).await;
        }
        self.cdp.call("Input.dispatchMouseEvent", json!({
            "type": "mouseReleased", "button": "left", "clickCount": 1, "x": tx, "y": ty
        })).await?;
        *self.mouse.lock().map_err(|e| DriverError::Connection(e.to_string()))? = (tx, ty);
        Ok(())
    }

    /// Smooth human swipe / drag from `(x1, y1)` to `(x2, y2)` with S-curve physics.
    pub async fn swipe(&self, x1: f64, y1: f64, x2: f64, y2: f64) -> DriverResult<()> {
        self.mouse_move(x1, y1).await?;
        tokio::time::sleep(jitter(60, 40)).await;
        self.cdp.call("Input.dispatchMouseEvent", json!({
            "type": "mousePressed", "button": "left", "clickCount": 1, "x": x1, "y": y1
        })).await?;
        tokio::time::sleep(jitter(50, 30)).await;

        for (cx, cy) in swipe_trajectory(x1, y1, x2, y2) {
            self.cdp.call("Input.dispatchMouseEvent", json!({
                "type": "mouseMoved", "buttons": 1, "x": cx, "y": cy
            })).await?;
            tokio::time::sleep(jitter(8, 8)).await;
        }

        tokio::time::sleep(jitter(40, 30)).await;
        self.cdp.call("Input.dispatchMouseEvent", json!({
            "type": "mouseReleased", "button": "left", "clickCount": 1, "x": x2, "y": y2
        })).await?;
        *self.mouse.lock().map_err(|e| DriverError::Connection(e.to_string()))? = (x2, y2);
        Ok(())
    }

    // ───────────────────────────── Keyboard ─────────────────────────────

    pub async fn focus(&self, selector: &str, timeout_ms: u64) -> DriverResult<()> {
        self.wait_for_selector(selector, timeout_ms, false).await?;
        self.evaluate(&format!("{}.focus()", sel_expr(selector))).await.map(|_| ())
    }

    pub async fn clear(&self, selector: &str, timeout_ms: u64) -> DriverResult<()> {
        self.focus(selector, timeout_ms).await?;
        self.evaluate(&format!(
            "(() => {{ const el = {}; if (el.select) el.select(); \
             else {{ const r = document.createRange(); r.selectNodeContents(el); const s = getSelection(); s.removeAllRanges(); s.addRange(r); }} }})()",
            sel_expr(selector)
        )).await?;
        self.press_key("Backspace").await
    }

    pub async fn type_text(&self, text: &str, delay_ms: u64) -> DriverResult<()> {
        for ch in text.chars() {
            if ch == '\n' {
                self.press_key("Enter").await?;
                continue;
            }
            let s = ch.to_string();
            self.cdp.call("Input.dispatchKeyEvent", json!({
                "type": "keyDown", "text": s, "unmodifiedText": s, "key": s
            })).await?;
            tokio::time::sleep(jitter(15, 20)).await;
            self.cdp.call("Input.dispatchKeyEvent", json!({ "type": "keyUp", "key": s })).await?;
            tokio::time::sleep(jitter(delay_ms, delay_ms / 2 + 1)).await;
        }
        Ok(())
    }

    pub async fn send_keys(&self, selector: &str, text: &str, clear_first: bool, delay_ms: u64, timeout_ms: u64) -> DriverResult<()> {
        if clear_first {
            self.clear(selector, timeout_ms).await?;
        } else {
            self.focus(selector, timeout_ms).await?;
        }
        self.type_text(text, delay_ms).await
    }

    async fn type_human_focused(&self, text: &str) -> DriverResult<()> {
        for (i, ch) in text.chars().enumerate() {
            if ch == '\n' {
                self.press_key("Enter").await?;
                tokio::time::sleep(jitter(120, 100)).await;
                continue;
            }
            let s = ch.to_string();
            self.cdp.call("Input.dispatchKeyEvent", json!({
                "type": "keyDown", "text": s, "unmodifiedText": s, "key": s
            })).await?;
            tokio::time::sleep(jitter(40, 45)).await;
            self.cdp.call("Input.dispatchKeyEvent", json!({ "type": "keyUp", "key": s })).await?;

            let inter_key_delay = match ch {
                ' ' => jitter(90, 80),
                '.' | ',' | '!' | '?' => jitter(150, 120),
                _ if i % 7 == 0 => jitter(80, 60),
                _ => jitter(45, 40),
            };
            tokio::time::sleep(inter_key_delay).await;
        }
        Ok(())
    }

    pub async fn human_type(&self, selector: &str, text: &str, clear_first: bool, timeout_ms: u64) -> DriverResult<()> {
        self.wait_for_selector(selector, timeout_ms, true).await?;
        if clear_first {
            self.clear(selector, timeout_ms).await?;
        } else {
            self.focus(selector, timeout_ms).await?;
        }
        self.type_human_focused(text).await
    }

    pub async fn fill(&self, selector: &str, text: &str, timeout_ms: u64) -> DriverResult<()> {
        self.clear(selector, timeout_ms).await?;
        self.cdp.call("Input.insertText", json!({ "text": text })).await.map(|_| ())
    }

    pub async fn press_key(&self, key: &str) -> DriverResult<()> {
        let (k, code, vk, text) = key_def(key)
            .ok_or_else(|| DriverError::InvalidInput(format!("Phím không hỗ trợ: {}", key)))?;
        let mut down = json!({
            "type": if text.is_empty() { "rawKeyDown" } else { "keyDown" },
            "key": k, "code": code,
            "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk
        });
        if !text.is_empty() {
            if let Some(map) = down.as_object_mut() {
                map.insert("text".to_string(), json!(text));
                map.insert("unmodifiedText".to_string(), json!(text));
            }
        }
        self.cdp.call("Input.dispatchKeyEvent", down).await?;
        tokio::time::sleep(jitter(30, 30)).await;
        self.cdp.call("Input.dispatchKeyEvent", json!({
            "type": "keyUp", "key": k, "code": code,
            "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk
        })).await.map(|_| ())
    }

    // ───────────────────── Native CDP Element Handles (Zero DOM Mutation) ─────────────────────

    async fn release_object(&self, object_id: &str) {
        let _ = self.cdp.call("Runtime.releaseObject", json!({ "objectId": object_id })).await;
    }

    async fn describe_and_release_object(&self, object_id: &str) -> DriverResult<(i64, Value)> {
        let desc = self.cdp.call("DOM.describeNode", json!({ "objectId": object_id })).await;
        let meta = self.cdp.call("Runtime.callFunctionOn", json!({
            "objectId": object_id,
            "functionDeclaration": NODE_META_FN,
            "returnByValue": true
        })).await;
        self.release_object(object_id).await;

        let backend_node_id = desc?
            .pointer("/node/backendNodeId")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| DriverError::NotFound("Không lấy được backendNodeId từ DOM.describeNode".to_string()))?;
        let data = meta?.pointer("/result/value").cloned().unwrap_or(Value::Null);
        Ok((backend_node_id, data))
    }

    async fn extract_array_nodes(&self, arr_object_id: &str) -> DriverResult<Vec<(i64, Value)>> {
        let props = self.cdp.call("Runtime.getProperties", json!({
            "objectId": arr_object_id,
            "ownProperties": true
        })).await;
        self.release_object(arr_object_id).await;
        let props = props?;

        let mut indexed: Vec<(usize, String)> = Vec::new();
        if let Some(arr) = props.get("result").and_then(|v| v.as_array()) {
            for prop in arr {
                if let Some(idx) = prop.get("name").and_then(|v| v.as_str()).and_then(|s| s.parse::<usize>().ok()) {
                    if let Some(oid) = prop.pointer("/value/objectId").and_then(|v| v.as_str()) {
                        indexed.push((idx, oid.to_string()));
                    }
                }
            }
        }
        indexed.sort_by_key(|(idx, _)| *idx);

        let futs: Vec<_> = indexed
            .iter()
            .map(|(_, oid)| self.describe_and_release_object(oid))
            .collect();
        let results = futures_util::future::join_all(futs).await;
        Ok(results.into_iter().flatten().collect())
    }

    async fn resolve_node(&self, backend_node_id: i64) -> DriverResult<String> {
        let res = self.cdp.call("DOM.resolveNode", json!({ "backendNodeId": backend_node_id })).await
            .map_err(|_| DriverError::StaleElement(backend_node_id))?;
        res.pointer("/object/objectId")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or(DriverError::StaleElement(backend_node_id))
    }

    async fn call_on_node(&self, backend_node_id: i64, fn_decl: &str, args: Vec<Value>) -> DriverResult<Value> {
        let object_id = self.resolve_node(backend_node_id).await?;
        let res = self.cdp.call("Runtime.callFunctionOn", json!({
            "objectId": object_id,
            "functionDeclaration": fn_decl,
            "arguments": args,
            "returnByValue": true,
            "awaitPromise": true,
            "userGesture": true
        })).await;
        self.release_object(&object_id).await;
        let res = res?;
        if let Some(ex) = res.get("exceptionDetails") {
            let msg = ex
                .pointer("/exception/description")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| ex.to_string());
            return Err(DriverError::JsException(msg));
        }
        Ok(res.pointer("/result/value").cloned().unwrap_or(Value::Null))
    }

    async fn node_center(&self, backend_node_id: i64) -> DriverResult<(f64, f64)> {
        let (ox, oy) = self.frame_viewport_offset().await;
        let pos = self.call_on_node(
            backend_node_id,
            "function() { \
                this.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' }); \
                const r = this.getBoundingClientRect(); \
                if (r.width === 0 && r.height === 0) return null; \
                return { x: r.left + r.width / 2, y: r.top + r.height / 2, w: r.width, h: r.height }; \
            }",
            vec![],
        ).await?;
        let (x, y) = Self::jittered_center(&pos)
            .ok_or_else(|| DriverError::NotFound(format!("backendNodeId={} không hiển thị", backend_node_id)))?;
        Ok((x + ox, y + oy))
    }

    pub async fn click_node(&self, backend_node_id: i64, button: &str, click_count: i64) -> DriverResult<()> {
        let (x, y) = self.node_center(backend_node_id).await?;
        self.mouse_click(x, y, button, click_count).await
    }

    pub async fn hover_node(&self, backend_node_id: i64) -> DriverResult<()> {
        let (x, y) = self.node_center(backend_node_id).await?;
        self.mouse_move(x, y).await
    }

    pub async fn focus_node(&self, backend_node_id: i64) -> DriverResult<()> {
        self.call_on_node(backend_node_id, "function() { this.focus(); }", vec![]).await.map(|_| ())
    }

    pub async fn clear_node(&self, backend_node_id: i64) -> DriverResult<()> {
        self.focus_node(backend_node_id).await?;
        self.call_on_node(
            backend_node_id,
            "function() { \
                if (this.select) this.select(); \
                else { const r = document.createRange(); r.selectNodeContents(this); const s = getSelection(); s.removeAllRanges(); s.addRange(r); } \
            }",
            vec![],
        ).await?;
        self.press_key("Backspace").await
    }

    pub async fn fill_node(&self, backend_node_id: i64, text: &str) -> DriverResult<()> {
        self.clear_node(backend_node_id).await?;
        self.cdp.call("Input.insertText", json!({ "text": text })).await.map(|_| ())
    }

    pub async fn human_type_node(&self, backend_node_id: i64, text: &str, clear_first: bool) -> DriverResult<()> {
        if clear_first {
            self.clear_node(backend_node_id).await?;
        } else {
            self.focus_node(backend_node_id).await?;
        }
        self.type_human_focused(text).await
    }

    pub async fn attr_node(&self, backend_node_id: i64, name: &str) -> DriverResult<Option<String>> {
        let v = self.call_on_node(
            backend_node_id,
            "function(n) { return this.getAttribute ? this.getAttribute(n) : null; }",
            vec![json!({ "value": name })],
        ).await?;
        Ok(v.as_str().map(str::to_string))
    }

    pub async fn inner_html_node(&self, backend_node_id: i64) -> DriverResult<Option<String>> {
        let v = self.call_on_node(backend_node_id, "function() { return this.innerHTML ?? null; }", vec![]).await?;
        Ok(v.as_str().map(str::to_string))
    }

    pub async fn outer_html_node(&self, backend_node_id: i64) -> DriverResult<String> {
        let v = self.call_on_node(backend_node_id, "function() { return this.outerHTML || ''; }", vec![]).await?;
        Ok(v.as_str().unwrap_or_default().to_string())
    }

    pub async fn scroll_into_view_node(&self, backend_node_id: i64) -> DriverResult<()> {
        self.call_on_node(
            backend_node_id,
            "function() { this.scrollIntoView({ block: 'center', behavior: 'instant' }); }",
            vec![],
        ).await.map(|_| ())
    }

    pub async fn is_visible_node(&self, backend_node_id: i64) -> DriverResult<bool> {
        let v = self.call_on_node(
            backend_node_id,
            "function() { \
                const s = getComputedStyle(this); \
                if (s.display === 'none' || s.visibility === 'hidden' || parseFloat(s.opacity) === 0) return false; \
                const r = this.getBoundingClientRect(); \
                return r.width > 0 && r.height > 0; \
            }",
            vec![],
        ).await?;
        Ok(v == Value::Bool(true))
    }

    pub async fn is_enabled_node(&self, backend_node_id: i64) -> DriverResult<bool> {
        let v = self.call_on_node(
            backend_node_id,
            "function() { return !this.disabled && this.getAttribute('aria-disabled') !== 'true'; }",
            vec![],
        ).await?;
        Ok(v == Value::Bool(true))
    }

    pub async fn is_checked_node(&self, backend_node_id: i64) -> DriverResult<bool> {
        let v = self.call_on_node(
            backend_node_id,
            "function() { return Boolean(this.checked || this.getAttribute('aria-checked') === 'true'); }",
            vec![],
        ).await?;
        Ok(v == Value::Bool(true))
    }

    pub async fn check_node(&self, backend_node_id: i64, checked: bool) -> DriverResult<()> {
        if self.is_checked_node(backend_node_id).await? != checked {
            self.click_node(backend_node_id, "left", 1).await?;
        }
        Ok(())
    }

    pub async fn select_option_node(&self, backend_node_id: i64, value: &str) -> DriverResult<()> {
        let ok = self.call_on_node(
            backend_node_id,
            "function(v) { \
                const opts = Array.from(this.options || []); \
                const opt = opts.find(o => o.value === v || o.text.trim() === v); \
                if (!opt) return false; \
                this.value = opt.value; \
                this.dispatchEvent(new Event('input', { bubbles: true })); \
                this.dispatchEvent(new Event('change', { bubbles: true })); \
                return true; \
            }",
            vec![json!({ "value": value })],
        ).await?;
        if ok != Value::Bool(true) {
            return Err(DriverError::NotFound(format!(
                "Không tìm thấy option '{}' trên node {}",
                value, backend_node_id
            )));
        }
        Ok(())
    }

    pub async fn screenshot_node(&self, backend_node_id: i64, file_path: Option<String>) -> DriverResult<Vec<u8>> {
        let r = self.call_on_node(
            backend_node_id,
            "function() { \
                this.scrollIntoView({ block: 'center', behavior: 'instant' }); \
                const b = this.getBoundingClientRect(); \
                return { x: b.left + scrollX, y: b.top + scrollY, width: b.width, height: b.height }; \
            }",
            vec![],
        ).await?;
        let mut clip = r
            .as_object()
            .cloned()
            .ok_or_else(|| DriverError::NotFound("Không lấy được kích thước phần tử".to_string()))?;
        clip.insert("scale".into(), json!(1));
        let res = self.cdp.call("Page.captureScreenshot", json!({
            "format": "png",
            "clip": Value::Object(clip),
            "captureBeyondViewport": true
        })).await?;
        let b64 = res
            .get("data")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DriverError::Connection("Không nhận được dữ liệu screenshot từ CDP".to_string()))?;
        let bytes = BASE64_STANDARD
            .decode(b64.trim())
            .map_err(|e| DriverError::Io(format!("Lỗi giải mã base64 screenshot: {}", e)))?;
        if let Some(path) = file_path {
            tokio::fs::write(&path, &bytes)
                .await
                .map_err(|e| DriverError::Io(format!("Lỗi ghi file ảnh '{}': {}", path, e)))?;
        }
        Ok(bytes)
    }

    async fn describe_backend_node(&self, backend_node_id: i64) -> DriverResult<(i64, Value)> {
        let data = self.call_on_node(backend_node_id, NODE_META_FN, vec![]).await?;
        Ok((backend_node_id, data))
    }

    /// Walks the CDP pierced DOM tree (including `#shadow-root (closed)`) scoped to the active document or iframe.
    async fn pierce_find_ids(&self, query: &str) -> DriverResult<Vec<i64>> {
        if let Some(fid) = self.frame_id() {
            let owner = self.cdp.call("DOM.getFrameOwner", json!({ "frameId": fid })).await?;
            let Some(owner_nid) = owner.get("backendNodeId").and_then(|v| v.as_i64()) else {
                return Ok(Vec::new());
            };
            let desc = self.cdp.call("DOM.describeNode", json!({
                "backendNodeId": owner_nid,
                "depth": -1,
                "pierce": true
            })).await?;
            let Some(doc) = desc.pointer("/node/contentDocument") else {
                return Ok(Vec::new());
            };
            return Ok(find_pierced_backend_node_ids(doc, query));
        }
        let doc = self.cdp.call("DOM.getDocument", json!({
            "depth": -1,
            "pierce": true
        })).await?;
        let Some(root) = doc.get("root") else {
            return Ok(Vec::new());
        };
        Ok(find_pierced_backend_node_ids(root, query))
    }

    pub async fn ele(&self, query: &str, timeout_ms: u64) -> DriverResult<Option<(i64, Value)>> {
        let expr = sel_expr(query);
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            if let Ok(res) = self.eval_raw(&expr, false).await {
                if let Some(object_id) = res.pointer("/result/objectId").and_then(|v| v.as_str()) {
                    return Ok(Some(self.describe_and_release_object(object_id).await?));
                }
            }
            if let Ok(ids) = self.pierce_find_ids(query).await {
                if let Some(&nid) = ids.first() {
                    if let Ok(node) = self.describe_backend_node(nid).await {
                        return Ok(Some(node));
                    }
                }
            }
            if timeout_ms == 0 || Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn eles(&self, query: &str, timeout_ms: u64) -> DriverResult<Vec<(i64, Value)>> {
        let expr = sel_all_expr(query);
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            if let Ok(res) = self.eval_raw(&expr, false).await {
                if let Some(arr_id) = res.pointer("/result/objectId").and_then(|v| v.as_str()) {
                    let nodes = self.extract_array_nodes(arr_id).await?;
                    if !nodes.is_empty() {
                        return Ok(nodes);
                    }
                }
            }
            if let Ok(ids) = self.pierce_find_ids(query).await {
                if !ids.is_empty() {
                    let futs: Vec<_> = ids.into_iter().map(|nid| self.describe_backend_node(nid)).collect();
                    let nodes: Vec<_> = futures_util::future::join_all(futs)
                        .await
                        .into_iter()
                        .flatten()
                        .collect();
                    if !nodes.is_empty() {
                        return Ok(nodes);
                    }
                }
            }
            if timeout_ms == 0 || Instant::now() >= deadline {
                return Ok(Vec::new());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn ele_child(&self, parent_backend_node_id: i64, query: &str) -> DriverResult<Option<(i64, Value)>> {
        let parent_oid = self.resolve_node(parent_backend_node_id).await?;
        let (fn_decl, arg_val) = child_sel_fn(query);
        let res = self.cdp.call("Runtime.callFunctionOn", json!({
            "objectId": parent_oid,
            "functionDeclaration": fn_decl,
            "arguments": [{ "value": arg_val }],
            "returnByValue": false
        })).await;
        self.release_object(&parent_oid).await;
        let res = res?;
        if let Some(child_oid) = res.pointer("/result/objectId").and_then(|v| v.as_str()) {
            return Ok(Some(self.describe_and_release_object(child_oid).await?));
        }
        // Fallback: pierce closed shadow roots attached under parent_backend_node_id
        if let Ok(desc) = self.cdp.call("DOM.describeNode", json!({
            "backendNodeId": parent_backend_node_id,
            "depth": -1,
            "pierce": true
        })).await {
            if let Some(node) = desc.get("node") {
                if let Some(nid) = find_pierced_backend_node_ids(node, query)
                    .into_iter()
                    .find(|&id| id != parent_backend_node_id)
                {
                    if let Ok(found) = self.describe_backend_node(nid).await {
                        return Ok(Some(found));
                    }
                }
            }
        }
        Ok(None)
    }

    pub async fn eles_child(&self, parent_backend_node_id: i64, query: &str) -> DriverResult<Vec<(i64, Value)>> {
        let parent_oid = self.resolve_node(parent_backend_node_id).await?;
        let (fn_decl, arg_val) = child_sel_all_fn(query);
        let res = self.cdp.call("Runtime.callFunctionOn", json!({
            "objectId": parent_oid,
            "functionDeclaration": fn_decl,
            "arguments": [{ "value": arg_val }],
            "returnByValue": false
        })).await;
        self.release_object(&parent_oid).await;
        let res = res?;
        if let Some(arr_oid) = res.pointer("/result/objectId").and_then(|v| v.as_str()) {
            let nodes = self.extract_array_nodes(arr_oid).await?;
            if !nodes.is_empty() {
                return Ok(nodes);
            }
        }
        if let Ok(desc) = self.cdp.call("DOM.describeNode", json!({
            "backendNodeId": parent_backend_node_id,
            "depth": -1,
            "pierce": true
        })).await {
            if let Some(node) = desc.get("node") {
                let ids: Vec<i64> = find_pierced_backend_node_ids(node, query)
                    .into_iter()
                    .filter(|&id| id != parent_backend_node_id)
                    .collect();
                if !ids.is_empty() {
                    let futs: Vec<_> = ids.into_iter().map(|nid| self.describe_backend_node(nid)).collect();
                    return Ok(futures_util::future::join_all(futs).await.into_iter().flatten().collect());
                }
            }
        }
        Ok(Vec::new())
    }

    // ───────────────────────── Iframes & Downloads ─────────────────────────

    /// Locates an `<iframe>` using Smart Locators (including inside closed shadow roots) and returns a scoped `Tab` controller
    /// (supports same-origin, `srcdoc`, nested iframes, and cross-origin OOPIF targets).
    pub async fn iframe(&self, selector: &str, timeout_ms: u64) -> DriverResult<Tab> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let expr = sel_expr(selector);
        let desc = loop {
            if let Ok(res) = self.eval_raw(&expr, false).await {
                if let Some(object_id) = res.pointer("/result/objectId").and_then(|v| v.as_str()) {
                    let d = self.cdp.call("DOM.describeNode", json!({ "objectId": object_id })).await;
                    self.release_object(object_id).await;
                    break d?;
                }
            }
            if let Ok(ids) = self.pierce_find_ids(selector).await {
                if let Some(&nid) = ids.first() {
                    break self.cdp.call("DOM.describeNode", json!({ "backendNodeId": nid })).await?;
                }
            }
            if Instant::now() >= deadline {
                return Err(DriverError::Timeout(format!("Chờ iframe '{}' quá {}ms", selector, timeout_ms)));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };

        let fid = desc
            .pointer("/node/frameId")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| DriverError::InvalidInput(format!("Phần tử '{}' không phải là <iframe>", selector)))?;

        // Check if Chromium placed this iframe in an Out-of-Process IFrame (OOPIF) target
        if let Ok(targets) = self.cdp.call_browser("Target.getTargets", json!({})).await {
            let is_oopif = targets
                .get("targetInfos")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter().any(|t| {
                        t.get("targetId").and_then(|v| v.as_str()) == Some(fid.as_str())
                            && t.get("type").and_then(|v| v.as_str()) == Some("iframe")
                    })
                })
                .unwrap_or(false);

            if is_oopif {
                let attached = self
                    .cdp
                    .call_browser("Target.attachToTarget", json!({ "targetId": fid, "flatten": true }))
                    .await?;
                if let Some(sid) = attached.get("sessionId").and_then(|v| v.as_str()) {
                    let child_session = self.cdp.connection().session(sid.to_string());
                    return Tab::new(
                        fid,
                        child_session,
                        self.proxy.clone(),
                        self.browser_context_id.clone(),
                    )
                    .await;
                }
            }
        }

        // In-process iframe (same-origin, srcdoc, data:, or non-isolated cross-origin): scope via isolated world + owner offset
        let mut chain = self.frame_chain.clone();
        chain.push(fid);
        Ok(Tab {
            target_id: self.target_id.clone(),
            cdp: Arc::clone(&self.cdp),
            proxy: self.proxy.clone(),
            browser_context_id: self.browser_context_id.clone(),
            frame_chain: chain,
            mouse: StdMutex::new((0.0, 0.0)),
            last_remote_ip: StdMutex::new(None),
            dialog_task: StdMutex::new(None),
            auth_task: StdMutex::new(None),
        })
    }

    /// Lists all frames in this tab's frame tree (`id`, `parent_id`, `name`, `url`).
    pub async fn frames(&self) -> DriverResult<Value> {
        fn collect(node: &Value, out: &mut Vec<Value>) {
            if let Some(f) = node.get("frame") {
                out.push(json!({
                    "id": f.get("id").unwrap_or(&Value::Null),
                    "parent_id": f.get("parentId").unwrap_or(&Value::Null),
                    "name": f.get("name").unwrap_or(&Value::Null),
                    "url": f.get("url").unwrap_or(&Value::Null),
                }));
            }
            if let Some(children) = node.get("childFrames").and_then(|v| v.as_array()) {
                for child in children {
                    collect(child, out);
                }
            }
        }
        let tree = self.cdp.call("Page.getFrameTree", json!({})).await?;
        let mut list = Vec::new();
        if let Some(root) = tree.get("frameTree") {
            collect(root, &mut list);
        }
        Ok(Value::Array(list))
    }

    /// Configures Chromium's download directory and enables download progress events.
    pub async fn set_download_dir(&self, download_dir: &str) -> DriverResult<String> {
        let dir_path = Path::new(download_dir);
        std::fs::create_dir_all(dir_path)
            .map_err(|e| DriverError::Io(format!("Không thể tạo thư mục tải về '{}': {}", download_dir, e)))?;
        let abs = std::fs::canonicalize(dir_path)
            .map_err(|e| DriverError::Io(format!("Đường dẫn không hợp lệ '{}': {}", download_dir, e)))?;
        let clean = abs.to_string_lossy().trim_start_matches(r"\\?\").to_string();
        self.cdp
            .call_browser(
                "Browser.setDownloadBehavior",
                json!({
                    "behavior": "allow",
                    "downloadPath": clean,
                    "eventsEnabled": true
                }),
            )
            .await?;
        Ok(clean)
    }

    /// Configures download dir, awaits the next file download to finish, and returns `{guid, url, filename, path, total_bytes, state}`.
    pub async fn wait_for_download(&self, download_dir: &str, timeout_ms: u64) -> DriverResult<Value> {
        // Subscribe BEFORE setting download behavior so no fast download event is missed
        let mut rx = self.cdp.subscribe_all();
        let abs_dir = self.set_download_dir(download_dir).await?;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);

        let mut guid = String::new();
        let mut url = String::new();
        let mut filename = String::new();

        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(ev)) => {
                    if ev.method == "Browser.downloadWillBegin" || ev.method == "Page.downloadWillBegin" {
                        guid = ev.params.get("guid").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                        url = ev.params.get("url").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                        filename = ev
                            .params
                            .get("suggestedFilename")
                            .and_then(|v| v.as_str())
                            .unwrap_or("download")
                            .to_string();
                        break;
                    }
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                _ => break,
            }
        }

        if guid.is_empty() {
            return Err(DriverError::Timeout(format!(
                "{}ms chờ trình duyệt bắt đầu tải file",
                timeout_ms
            )));
        }

        let mut total_bytes: f64 = 0.0;
        let mut final_state = String::new();
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok(ev)) => {
                    if (ev.method == "Browser.downloadProgress" || ev.method == "Page.downloadProgress")
                        && ev.params.get("guid").and_then(|v| v.as_str()) == Some(guid.as_str())
                    {
                        if let Some(tb) = ev.params.get("totalBytes").and_then(|v| v.as_f64()) {
                            total_bytes = tb;
                        }
                        if let Some(st) = ev.params.get("state").and_then(|v| v.as_str()) {
                            if st == "completed" || st == "canceled" {
                                final_state = st.to_string();
                                break;
                            }
                        }
                    }
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                _ => break,
            }
        }

        if final_state == "canceled" {
            return Err(DriverError::Io(format!("Quá trình tải file '{}' đã bị huỷ", filename)));
        }
        if final_state != "completed" {
            return Err(DriverError::Timeout(format!(
                "{}ms chờ tải xong file '{}'",
                timeout_ms, filename
            )));
        }

        let full_path = Path::new(&abs_dir).join(&filename).to_string_lossy().to_string();
        Ok(json!({
            "guid": guid,
            "url": url,
            "filename": filename,
            "path": full_path,
            "total_bytes": total_bytes as u64,
            "state": final_state
        }))
    }

    // ───────────────────────────── Forms ─────────────────────────────

    pub async fn select_option(&self, selector: &str, value: &str, timeout_ms: u64) -> DriverResult<()> {
        self.wait_for_selector(selector, timeout_ms, false).await?;
        let ok = self.evaluate(&format!(
            "(() => {{ const el = {}; const v = {}; \
             const opt = Array.from(el.options).find(o => o.value === v || o.text.trim() === v); \
             if (!opt) return false; el.value = opt.value; \
             el.dispatchEvent(new Event('input', {{ bubbles: true }})); \
             el.dispatchEvent(new Event('change', {{ bubbles: true }})); return true; }})()",
            sel_expr(selector), js_str(value)
        )).await?;
        if ok == Value::Bool(true) {
            Ok(())
        } else {
            Err(DriverError::NotFound(format!("Option '{}' trong '{}'", value, selector)))
        }
    }

    pub async fn set_checked(&self, selector: &str, checked: bool, timeout_ms: u64) -> DriverResult<()> {
        self.wait_for_selector(selector, timeout_ms, false).await?;
        let cur = self.evaluate(&format!("{}.checked === true", sel_expr(selector))).await?;
        if cur == Value::Bool(checked) {
            return Ok(());
        }
        if self.element_center(&sel_expr(selector), selector).await.is_ok() {
            self.click(selector, "left", 1, timeout_ms).await
        } else {
            let label = format!(
                "(() => {{ const el = {}; return el.closest('label') || (el.id && document.querySelector('label[for=\"' + el.id + '\"]')) || el.parentElement; }})()",
                sel_expr(selector)
            );
            let (x, y) = self.element_center(&label, selector).await?;
            self.mouse_click(x, y, "left", 1).await
        }
    }

    pub async fn upload_file(&self, selector: &str, files: &[String], timeout_ms: u64) -> DriverResult<()> {
        self.wait_for_selector(selector, timeout_ms, false).await?;
        let mut abs = Vec::with_capacity(files.len());
        for f in files {
            let p = std::fs::canonicalize(Path::new(f))
                .map_err(|e| DriverError::Io(format!("File không tồn tại '{}': {}", f, e)))?;
            abs.push(p.to_string_lossy().trim_start_matches(r"\\?\").to_string());
        }
        let res = self.eval_raw(&sel_expr(selector), false).await?;
        let object_id = res
            .pointer("/result/objectId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DriverError::NotFound(format!("Không lấy được objectId của '{}'", selector)))?
            .to_string();
        let out = self.cdp.call("DOM.setFileInputFiles", json!({ "files": abs, "objectId": object_id })).await;
        self.release_object(&object_id).await;
        out.map(|_| ())
    }

    // ───────────────────────────── Scrolling ─────────────────────────────

    pub async fn scroll_by(&self, dx: f64, dy: f64) -> DriverResult<()> {
        let vp = self.evaluate("({ w: innerWidth, h: innerHeight })").await?;
        let cx = vp.get("w").and_then(|v| v.as_f64()).unwrap_or(800.0) / 2.0;
        let cy = vp.get("h").and_then(|v| v.as_f64()).unwrap_or(600.0) / 2.0;
        let ticks = ((dx.abs().max(dy.abs()) / 120.0).ceil() as i64).max(1);
        for _ in 0..ticks {
            self.cdp.call("Input.dispatchMouseEvent", json!({
                "type": "mouseWheel", "x": cx, "y": cy,
                "deltaX": dx / ticks as f64, "deltaY": dy / ticks as f64
            })).await?;
            tokio::time::sleep(jitter(25, 25)).await;
        }
        Ok(())
    }

    pub async fn scroll_to_bottom(&self, step: f64, pause_ms: u64, max_rounds: u32) -> DriverResult<()> {
        let mut stalled = 0;
        let mut last_h: Option<f64> = None;
        for _ in 0..max_rounds {
            self.scroll_by(0.0, step).await?;
            tokio::time::sleep(Duration::from_millis(pause_ms)).await;
            let s = self.evaluate("({ y: scrollY + innerHeight, h: document.documentElement.scrollHeight })").await?;
            let y = s.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let h = s.get("h").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if y + 2.0 >= h {
                if last_h == Some(h) {
                    stalled += 1;
                    if stalled >= 2 {
                        break;
                    }
                } else {
                    stalled = 0;
                }
            }
            last_h = Some(h);
        }
        Ok(())
    }

    pub async fn scroll_to_top(&self) -> DriverResult<()> {
        self.evaluate("window.scrollTo({ top: 0, behavior: 'instant' })").await.map(|_| ())
    }

    pub async fn scroll_into_view(&self, selector: &str, timeout_ms: u64) -> DriverResult<()> {
        self.wait_for_selector(selector, timeout_ms, false).await?;
        self.evaluate(&format!("{}.scrollIntoView({{ block: 'center', behavior: 'instant' }})", sel_expr(selector)))
            .await
            .map(|_| ())
    }

    // ───────────────────────────── Emulation ─────────────────────────────

    pub async fn set_viewport(&self, width: i64, height: i64, scale: f64, mobile: bool) -> DriverResult<()> {
        self.cdp.call("Emulation.setDeviceMetricsOverride", json!({
            "width": width, "height": height, "deviceScaleFactor": scale, "mobile": mobile
        })).await.map(|_| ())
    }

    pub async fn clear_viewport(&self) -> DriverResult<()> {
        self.cdp.call("Emulation.clearDeviceMetricsOverride", json!({})).await.map(|_| ())
    }

    pub async fn set_user_agent(
        &self,
        ua: &str,
        accept_language: Option<&str>,
        platform: Option<&str>,
    ) -> DriverResult<()> {
        let mut p = Map::new();
        p.insert("userAgent".into(), json!(ua));
        if let Some(l) = accept_language { p.insert("acceptLanguage".into(), json!(l)); }
        if let Some(pl) = platform { p.insert("platform".into(), json!(pl)); }
        self.cdp.call("Network.setUserAgentOverride", Value::Object(p)).await.map(|_| ())
    }

    pub async fn set_timezone(&self, timezone_id: &str) -> DriverResult<()> {
        self.cdp.call("Emulation.setTimezoneOverride", json!({ "timezoneId": timezone_id })).await.map(|_| ())
    }

    pub async fn set_locale(&self, locale: &str) -> DriverResult<()> {
        self.cdp.call("Emulation.setLocaleOverride", json!({ "locale": locale })).await.map(|_| ())
    }

    pub async fn set_geolocation(&self, latitude: f64, longitude: f64, accuracy: f64) -> DriverResult<()> {
        let _ = self.cdp.call_browser("Browser.grantPermissions", json!({ "permissions": ["geolocation"] })).await;
        self.cdp.call("Emulation.setGeolocationOverride", json!({
            "latitude": latitude, "longitude": longitude, "accuracy": accuracy
        })).await.map(|_| ())
    }

    // ───────────────────────────── Network ─────────────────────────────

    pub async fn set_extra_headers(&self, headers: Value) -> DriverResult<()> {
        self.cdp.call("Network.setExtraHTTPHeaders", json!({ "headers": headers })).await.map(|_| ())
    }

    pub async fn block_urls(&self, patterns: &[String]) -> DriverResult<()> {
        self.cdp.call("Network.setBlockedURLs", json!({ "urls": patterns })).await.map(|_| ())
    }

    pub async fn set_cache_disabled(&self, disabled: bool) -> DriverResult<()> {
        self.cdp.call("Network.setCacheDisabled", json!({ "cacheDisabled": disabled })).await.map(|_| ())
    }

    /// Awaits a network response matching a URL substring, waits for body transfer completion, and retrieves status/headers/body.
    pub async fn wait_for_response(&self, url_pattern: &str, timeout_ms: u64) -> DriverResult<Value> {
        let mut rx = self.cdp.subscribe();
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut matched_req_id = None;
        let mut matched_resp = None;

        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok((method, params))) => {
                    if method == "Network.responseReceived" {
                        if let Some(resp) = params.get("response") {
                            if let Some(u) = resp.get("url").and_then(|v| v.as_str()) {
                                if u.contains(url_pattern) {
                                    let req_id = params.get("requestId").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                                    matched_req_id = Some(req_id);
                                    matched_resp = Some(resp.clone());
                                    break;
                                }
                            }
                        }
                    }
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                _ => break,
            }
        }

        let (req_id, resp) = match (matched_req_id, matched_resp) {
            (Some(id), Some(r)) => (id, r),
            _ => {
                return Err(DriverError::Timeout(format!(
                    "{}ms chờ response matching '{}'",
                    timeout_ms, url_pattern
                )));
            }
        };

        let finish_deadline = deadline.min(Instant::now() + Duration::from_secs(5));
        while Instant::now() < finish_deadline {
            let remaining = finish_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Ok((method, params))) => {
                    if (method == "Network.loadingFinished" || method == "Network.loadingFailed")
                        && params.get("requestId").and_then(|v| v.as_str()) == Some(req_id.as_str())
                    {
                        break;
                    }
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                _ => break,
            }
        }

        let (body_val, base64_encoded) = match self.cdp.call("Network.getResponseBody", json!({ "requestId": req_id })).await {
            Ok(res) => {
                let is_b64 = res.get("base64Encoded").and_then(|v| v.as_bool()).unwrap_or(false);
                let body = match res.get("body").and_then(|v| v.as_str()) {
                    Some(raw) if is_b64 => match BASE64_STANDARD.decode(raw) {
                        Ok(bytes) => match String::from_utf8(bytes) {
                            Ok(utf8) => Value::String(utf8),
                            Err(_) => Value::String(raw.to_string()),
                        },
                        Err(_) => Value::String(raw.to_string()),
                    },
                    Some(raw) => Value::String(raw.to_string()),
                    None => Value::Null,
                };
                (body, is_b64)
            }
            Err(_) => (Value::Null, false),
        };

        Ok(json!({
            "url": resp.get("url").unwrap_or(&Value::Null),
            "status": resp.get("status").unwrap_or(&Value::Null),
            "headers": resp.get("headers").unwrap_or(&Value::Null),
            "mime_type": resp.get("mimeType").unwrap_or(&Value::Null),
            "body": body_val,
            "base64_encoded": base64_encoded
        }))
    }

    pub async fn enable_proxy_auth(&self, username: String, password: String) -> DriverResult<()> {
        let mut rx = self.cdp.subscribe();
        self.cdp.call("Fetch.enable", json!({
            "patterns": [{ "urlPattern": "*" }],
            "handleAuthRequests": true
        })).await?;
        let cdp = Arc::clone(&self.cdp);
        let handle = tokio::spawn(async move {
            loop {
                let (method, params) = match rx.recv().await {
                    Ok(ev) => ev,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                };
                let Some(rid) = params.get("requestId").and_then(|v| v.as_str()).map(str::to_string) else {
                    continue;
                };
                let cdp = Arc::clone(&cdp);
                match method.as_str() {
                    "Fetch.requestPaused" => {
                        tokio::spawn(async move {
                            let _ = cdp.call("Fetch.continueRequest", json!({ "requestId": rid })).await;
                        });
                    }
                    "Fetch.authRequired" => {
                        let is_proxy = params.pointer("/authChallenge/source").and_then(|v| v.as_str()) == Some("Proxy");
                        let resp = if is_proxy {
                            json!({ "response": "ProvideCredentials", "username": username, "password": password })
                        } else {
                            json!({ "response": "Default" })
                        };
                        tokio::spawn(async move {
                            let _ = cdp.call("Fetch.continueWithAuth", json!({
                                "requestId": rid, "authChallengeResponse": resp
                            })).await;
                        });
                    }
                    _ => {}
                }
            }
        });
        if let Some(old) = self.auth_task.lock().map_err(|e| DriverError::Connection(e.to_string()))?.replace(handle) {
            old.abort();
        }
        Ok(())
    }

    pub fn abort_auth_task(&self) {
        if let Ok(mut g) = self.auth_task.lock() {
            if let Some(h) = g.take() {
                h.abort();
            }
        }
    }

    pub async fn add_init_script(&self, source: &str) -> DriverResult<String> {
        let res = self
            .cdp
            .call("Page.addScriptToEvaluateOnNewDocument", json!({ "source": source }))
            .await?;
        Ok(res
            .get("identifier")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string())
    }

    /// Executes a real in-browser `fetch()` using Chromium's native TLS stack, HTTP/2, and cookie jar.
    pub async fn fetch(
        &self,
        url: &str,
        method: &str,
        headers: Option<Value>,
        body: Option<String>,
        credentials: &str,
    ) -> DriverResult<Value> {
        let hdrs_json = headers.unwrap_or_else(|| json!({}));
        let body_json = match body {
            Some(b) => json!(b),
            None => Value::Null,
        };
        let expr = format!(
            "(async () => {{ \
                const opts = {{ method: {}, headers: {}, credentials: {} }}; \
                const rawBody = {}; \
                if (rawBody !== null) opts.body = rawBody; \
                const resp = await fetch({}, opts); \
                const hdrs = {{}}; \
                resp.headers.forEach((v, k) => {{ hdrs[k] = v; }}); \
                const text = await resp.text(); \
                return {{ \
                    url: resp.url, \
                    status: resp.status, \
                    status_text: resp.statusText, \
                    ok: resp.ok, \
                    headers: hdrs, \
                    body: text \
                }}; \
            }})()",
            js_str(method),
            hdrs_json,
            js_str(credentials),
            body_json,
            js_str(url)
        );
        self.evaluate(&expr).await
    }

    // ───────────────────────────── Fingerprint & TLS Session Handoff ─────────────────────────────

    /// Captures the live browser's HTTP/TLS identity (`impersonate`, `user_agent`, `sec_ch_ua*`,
    /// `accept_language`, `headers`, Chrome wire `header_order`, and `proxy`) for `http_tls` handoff.
    pub async fn fingerprint(&self) -> DriverResult<Value> {
        let info = self.evaluate(
            "(() => { \
                const d = navigator.userAgentData; \
                const brands = d && Array.isArray(d.brands) \
                    ? d.brands.map(b => `\"${b.brand}\";v=\"${b.version}\"`).join(', ') \
                    : null; \
                const langs = Array.isArray(navigator.languages) && navigator.languages.length \
                    ? navigator.languages \
                    : [navigator.language || 'en-US']; \
                const acceptLang = langs.map((l, i) => i === 0 ? l : `${l};q=${Math.max(0.1, (1 - i * 0.1)).toFixed(1)}`).join(','); \
                return { \
                    user_agent: navigator.userAgent || '', \
                    sec_ch_ua: brands, \
                    sec_ch_ua_mobile: d ? (d.mobile ? '?1' : '?0') : '?0', \
                    platform: d && d.platform ? d.platform : (navigator.platform || 'Windows'), \
                    accept_language: acceptLang \
                }; \
            })()"
        ).await?;

        let ua = info.get("user_agent").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let platform_raw = info.get("platform").and_then(|v| v.as_str()).unwrap_or("Windows");
        let platform_norm = if platform_raw.to_lowercase().contains("win") {
            "Windows"
        } else if platform_raw.to_lowercase().contains("mac") {
            "macOS"
        } else if platform_raw.to_lowercase().contains("android") {
            "Android"
        } else if platform_raw.to_lowercase().contains("linux") {
            "Linux"
        } else {
            platform_raw
        };
        let sec_ch_ua_platform = format!("\"{}\"", platform_norm);
        let sec_ch_ua_mobile = info.get("sec_ch_ua_mobile").and_then(|v| v.as_str()).unwrap_or("?0").to_string();
        let accept_language = info.get("accept_language").and_then(|v| v.as_str()).unwrap_or("en-US,en;q=0.9").to_string();
        let impersonate = derive_tls_preset(&ua, platform_norm);

        let mut headers = Map::new();
        headers.insert("User-Agent".into(), json!(ua));
        if let Some(ch) = info.get("sec_ch_ua").and_then(|v| v.as_str()) {
            headers.insert("Sec-CH-UA".into(), json!(ch));
            headers.insert("Sec-CH-UA-Mobile".into(), json!(sec_ch_ua_mobile));
            headers.insert("Sec-CH-UA-Platform".into(), json!(sec_ch_ua_platform));
        }
        headers.insert("Accept-Language".into(), json!(accept_language));

        // Exact Chrome HTTP/2 wire header order (`accept-encoding` before `accept-language`)
        let header_order = json!([
            "sec-ch-ua",
            "sec-ch-ua-mobile",
            "sec-ch-ua-platform",
            "upgrade-insecure-requests",
            "user-agent",
            "accept",
            "sec-fetch-site",
            "sec-fetch-mode",
            "sec-fetch-user",
            "sec-fetch-dest",
            "accept-encoding",
            "accept-language",
            "cookie"
        ]);

        let remote_ip = self.last_remote_ip.lock().ok().and_then(|g| g.clone());
        let local_address: Option<&str> = if self.proxy.is_none() {
            match remote_ip.as_deref() {
                Some(ip) if ip.contains(':') => Some("::"),
                Some(ip) if ip.contains('.') => Some("0.0.0.0"),
                _ => None,
            }
        } else {
            None
        };

        Ok(json!({
            "impersonate": impersonate,
            "user_agent": ua,
            "sec_ch_ua": info.get("sec_ch_ua").cloned().unwrap_or(Value::Null),
            "sec_ch_ua_mobile": sec_ch_ua_mobile,
            "sec_ch_ua_platform": sec_ch_ua_platform,
            "accept_language": accept_language,
            "headers": Value::Object(headers),
            "header_order": header_order,
            "proxy": self.proxy,
            "remote_ip": remote_ip,
            "local_address": local_address,
        }))
    }

    /// Exports Playwright / `http_tls`-compatible `storage_state` containing all CDP cookies (including `HttpOnly`),
    /// exact browser headers, TLS `impersonate` preset, wire `header_order`, and `proxy`.
    pub async fn storage_state(&self, path: Option<&str>) -> DriverResult<Value> {
        let cookies = self.get_cookies(None).await?;
        let fp = self.fingerprint().await?;

        let state = json!({
            "cookies": cookies,
            "origins": [],
            "user_agent": fp.get("user_agent").cloned().unwrap_or(Value::Null),
            "headers": fp.get("headers").cloned().unwrap_or_else(|| json!({})),
            "impersonate": fp.get("impersonate").cloned().unwrap_or(Value::Null),
            "header_order": fp.get("header_order").cloned().unwrap_or_else(|| json!([])),
            "proxy": fp.get("proxy").cloned().unwrap_or(Value::Null),
            "remote_ip": fp.get("remote_ip").cloned().unwrap_or(Value::Null),
            "local_address": fp.get("local_address").cloned().unwrap_or(Value::Null),
        });

        if let Some(p) = path {
            let text = serde_json::to_string_pretty(&state)
                .map_err(|e| DriverError::Io(format!("Lỗi serialize storage_state: {}", e)))?;
            tokio::fs::write(p, text)
                .await
                .map_err(|e| DriverError::Io(format!("Lỗi ghi file storage_state '{}': {}", p, e)))?;
        }
        Ok(state)
    }

    // ───────────────────────────── Cookies & Storage ─────────────────────────────

    pub async fn get_cookies(&self, urls: Option<Vec<String>>) -> DriverResult<Value> {
        let res = if let Some(u) = urls {
            self.cdp.call("Network.getCookies", json!({ "urls": u })).await?
        } else if let Some(ref cid) = self.browser_context_id {
            // In an isolated BrowserContext, Storage.getCookies MUST pass `browserContextId`;
            // omitting it causes Chromium to return the default context's cookies instead.
            match self
                .cdp
                .call_browser("Storage.getCookies", json!({ "browserContextId": cid }))
                .await
            {
                Ok(v) => v,
                Err(_) => self.cdp.call("Network.getCookies", json!({})).await?,
            }
        } else {
            match self.cdp.call("Storage.getCookies", json!({})).await {
                Ok(v) => v,
                Err(_) => self.cdp.call("Network.getAllCookies", json!({})).await?,
            }
        };
        Ok(res.get("cookies").cloned().unwrap_or_else(|| json!([])))
    }

    pub async fn set_cookies(&self, cookies: Value) -> DriverResult<()> {
        const KEEP: [&str; 9] = ["name", "value", "url", "domain", "path", "secure", "httpOnly", "sameSite", "expires"];
        let list = cookies
            .as_array()
            .ok_or_else(|| DriverError::InvalidInput("cookies phải là list các dict".to_string()))?;
        let mut clean = Vec::with_capacity(list.len());
        for c in list {
            let obj = c
                .as_object()
                .ok_or_else(|| DriverError::InvalidInput("mỗi cookie phải là dict".to_string()))?;
            let mut m = Map::new();
            for k in KEEP {
                if let Some(v) = obj.get(k) {
                    m.insert(k.to_string(), v.clone());
                }
            }
            let session = obj.get("session").and_then(|v| v.as_bool()).unwrap_or(false);
            let neg_exp = m.get("expires").and_then(|v| v.as_f64()).map(|e| e <= 0.0).unwrap_or(false);
            if session || neg_exp {
                m.remove("expires");
            }
            if !m.contains_key("url") && !m.contains_key("domain") {
                return Err(DriverError::InvalidInput(format!(
                    "Cookie '{}' thiếu 'domain' hoặc 'url'",
                    obj.get("name").and_then(|v| v.as_str()).unwrap_or("?")
                )));
            }
            clean.push(Value::Object(m));
        }
        if let Some(ref cid) = self.browser_context_id {
            if self
                .cdp
                .call_browser(
                    "Storage.setCookies",
                    json!({ "cookies": clean, "browserContextId": cid }),
                )
                .await
                .is_ok()
            {
                return Ok(());
            }
        }
        self.cdp.call("Network.setCookies", json!({ "cookies": clean })).await.map(|_| ())
    }

    pub async fn set_cookie(&self, name: &str, value: &str, domain: Option<&str>, path: Option<&str>) -> DriverResult<()> {
        let mut cookie = Map::new();
        cookie.insert("name".to_string(), json!(name));
        cookie.insert("value".to_string(), json!(value));
        if let Some(d) = domain {
            cookie.insert("domain".to_string(), json!(d));
        } else {
            let u = self.current_url().await?;
            if u.starts_with("http://") || u.starts_with("https://") {
                cookie.insert("url".to_string(), json!(u));
            } else {
                cookie.insert("domain".to_string(), json!("localhost"));
                cookie.insert("url".to_string(), json!("http://localhost/"));
            }
        }
        cookie.insert("path".to_string(), json!(path.unwrap_or("/")));
        self.set_cookies(Value::Array(vec![Value::Object(cookie)])).await
    }

    pub async fn delete_cookies(&self, name: &str, domain: Option<&str>, url: Option<&str>) -> DriverResult<()> {
        let mut p = Map::new();
        p.insert("name".into(), json!(name));
        if let Some(d) = domain { p.insert("domain".into(), json!(d)); }
        if let Some(u) = url { p.insert("url".into(), json!(u)); }
        if domain.is_none() && url.is_none() {
            let u = self.current_url().await?;
            p.insert("url".into(), json!(u));
        }
        self.cdp.call("Network.deleteCookies", Value::Object(p)).await.map(|_| ())
    }

    pub async fn clear_cookies(&self) -> DriverResult<()> {
        if let Some(ref cid) = self.browser_context_id {
            if self
                .cdp
                .call_browser("Storage.clearCookies", json!({ "browserContextId": cid }))
                .await
                .is_ok()
            {
                return Ok(());
            }
        }
        self.cdp.call("Network.clearBrowserCookies", json!({})).await.map(|_| ())
    }

    pub async fn save_cookies(&self, path: &str) -> DriverResult<usize> {
        let cookies = self.get_cookies(None).await?;
        let n = cookies.as_array().map(|a| a.len()).unwrap_or(0);
        let data = serde_json::to_vec_pretty(&cookies).map_err(|e| DriverError::Io(e.to_string()))?;
        tokio::fs::write(path, data)
            .await
            .map_err(|e| DriverError::Io(format!("Lỗi ghi '{}': {}", path, e)))?;
        Ok(n)
    }

    pub async fn load_cookies(&self, path: &str) -> DriverResult<usize> {
        let raw = tokio::fs::read(path)
            .await
            .map_err(|e| DriverError::Io(format!("Lỗi đọc '{}': {}", path, e)))?;
        let cookies: Value = serde_json::from_slice(&raw)
            .map_err(|e| DriverError::InvalidInput(format!("JSON cookie hỏng: {}", e)))?;
        let n = cookies.as_array().map(|a| a.len()).unwrap_or(0);
        self.set_cookies(cookies).await?;
        Ok(n)
    }

    fn storage_obj(session: bool) -> &'static str {
        if session { "sessionStorage" } else { "localStorage" }
    }

    pub async fn storage_get(&self, key: Option<&str>, session: bool) -> DriverResult<Value> {
        let s = Self::storage_obj(session);
        if let Some(k) = key {
            self.evaluate(&format!("{}.getItem({})", s, js_str(k))).await
        } else {
            self.evaluate(&format!("Object.fromEntries(Object.entries({}))", s)).await
        }
    }

    pub async fn storage_set(&self, key: &str, value: &str, session: bool) -> DriverResult<()> {
        let s = Self::storage_obj(session);
        self.evaluate(&format!("{}.setItem({}, {})", s, js_str(key), js_str(value))).await.map(|_| ())
    }

    pub async fn storage_clear(&self, session: bool) -> DriverResult<()> {
        self.evaluate(&format!("{}.clear()", Self::storage_obj(session))).await.map(|_| ())
    }

    // ───────────────────────────── Dialogs ─────────────────────────────

    pub fn set_dialog_handler(&self, mode: Option<(bool, Option<String>)>) -> DriverResult<()> {
        let mut slot = self.dialog_task.lock().map_err(|e| DriverError::Connection(e.to_string()))?;
        if let Some(old) = slot.take() {
            old.abort();
        }
        let Some((accept, prompt_text)) = mode else {
            return Ok(());
        };
        let mut rx = self.cdp.subscribe();
        let cdp = Arc::clone(&self.cdp);
        *slot = Some(tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok((m, _)) if m == "Page.javascriptDialogOpening" => {
                        let mut p = Map::new();
                        p.insert("accept".into(), json!(accept));
                        if let Some(t) = &prompt_text { p.insert("promptText".into(), json!(t)); }
                        let _ = cdp.call("Page.handleJavaScriptDialog", Value::Object(p)).await;
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        }));
        Ok(())
    }

    pub async fn handle_dialog(&self, accept: bool, prompt_text: Option<&str>) -> DriverResult<()> {
        let mut p = Map::new();
        p.insert("accept".into(), json!(accept));
        if let Some(t) = prompt_text { p.insert("promptText".into(), json!(t)); }
        self.cdp.call("Page.handleJavaScriptDialog", Value::Object(p)).await.map(|_| ())
    }

    // ───────────────────────────── Capture ─────────────────────────────

    pub async fn screenshot(
        &self,
        file_path: Option<String>,
        full_page: bool,
        format: &str,
        quality: Option<i64>,
        selector: Option<&str>,
    ) -> DriverResult<Vec<u8>> {
        let fmt = match format { "jpeg" | "jpg" => "jpeg", "webp" => "webp", _ => "png" };
        let mut p = Map::new();
        p.insert("format".into(), json!(fmt));
        if fmt != "png" {
            p.insert("quality".into(), json!(quality.unwrap_or(85)));
        }

        if let Some(sel) = selector {
            self.wait_for_selector(sel, 10000, true).await?;
            let r = self.evaluate(&format!(
                "(() => {{ const el = {}; el.scrollIntoView({{ block: 'center', behavior: 'instant' }}); \
                 const r = el.getBoundingClientRect(); \
                 return {{ x: r.left + scrollX, y: r.top + scrollY, width: r.width, height: r.height }}; }})()",
                sel_expr(sel)
            )).await?;
            let mut clip = r
                .as_object()
                .cloned()
                .ok_or_else(|| DriverError::NotFound("Không lấy được kích thước phần tử".to_string()))?;
            clip.insert("scale".into(), json!(1));
            p.insert("clip".into(), Value::Object(clip));
            p.insert("captureBeyondViewport".into(), json!(true));
        } else if full_page {
            let m = self.cdp.call("Page.getLayoutMetrics", json!({})).await?;
            let size = m.get("cssContentSize").or_else(|| m.get("contentSize")).cloned().unwrap_or(Value::Null);
            let w = size.get("width").and_then(|v| v.as_f64()).unwrap_or(1280.0).ceil();
            let h = size.get("height").and_then(|v| v.as_f64()).unwrap_or(800.0).ceil();
            p.insert("clip".into(), json!({ "x": 0, "y": 0, "width": w, "height": h, "scale": 1 }));
            p.insert("captureBeyondViewport".into(), json!(true));
        }

        let res = self.cdp.call("Page.captureScreenshot", Value::Object(p)).await?;
        let b64 = res
            .get("data")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DriverError::Connection("Không nhận được dữ liệu screenshot từ CDP".to_string()))?;
        let bytes = BASE64_STANDARD
            .decode(b64.trim())
            .map_err(|e| DriverError::Io(format!("Lỗi giải mã base64 screenshot: {}", e)))?;

        if let Some(path) = file_path {
            tokio::fs::write(&path, &bytes)
                .await
                .map_err(|e| DriverError::Io(format!("Lỗi ghi file ảnh '{}': {}", path, e)))?;
        }
        Ok(bytes)
    }

    pub async fn pdf(&self, file_path: Option<String>, landscape: bool) -> DriverResult<Vec<u8>> {
        let res = self.cdp.call("Page.printToPDF", json!({ "printBackground": true, "landscape": landscape })).await?;
        let b64 = res
            .get("data")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DriverError::Connection("Không nhận được dữ liệu PDF".to_string()))?;
        let bytes = BASE64_STANDARD
            .decode(b64.trim())
            .map_err(|e| DriverError::Io(format!("Lỗi giải mã base64 PDF: {}", e)))?;
        if let Some(path) = file_path {
            tokio::fs::write(&path, &bytes)
                .await
                .map_err(|e| DriverError::Io(format!("Lỗi ghi file PDF '{}': {}", path, e)))?;
        }
        Ok(bytes)
    }

    pub async fn sleep(&self, seconds: f64) {
        tokio::time::sleep(Duration::from_secs_f64(seconds.max(0.0))).await;
    }
}
