// src/tab.rs — Rust 2024: per-tab CDP controller (navigation, DOM, input, network, storage, capture)
use std::option::Option::{self, Some, None};
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use base64::prelude::*;
use serde_json::{json, Map, Value};
use tokio::task::JoinHandle;
use crate::cdp::CdpSession;

const EVASION_JS: &str = include_str!("evasion.js");

/// Quote a Rust string as a JS string literal (handles quotes, backslashes, unicode).
fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

fn sel_expr(selector: &str) -> String {
    format!("document.querySelector({})", js_str(selector))
}

/// Small jitter without pulling in a RNG crate.
fn jitter(base_ms: u64, spread_ms: u64) -> Duration {
    if spread_ms == 0 {
        return Duration::from_millis(base_ms);
    }
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0) as u64;
    Duration::from_millis(base_ms + nanos % spread_ms)
}

/// (key, code, windowsVirtualKeyCode, text)
fn key_def(key: &str) -> Option<(&'static str, &'static str, i64, &'static str)> {
    Some(match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => ("Enter", "Enter", 13, "\r"),
        "tab" => ("Tab", "Tab", 9, ""),
        "escape" | "esc" => ("Escape", "Escape", 27, ""),
        "backspace" => ("Backspace", "Backspace", 8, ""),
        "delete" | "del" => ("Delete", "Delete", 46, ""),
        "space" => (" ", "Space", 32, " "),
        "arrowup" | "up" => ("ArrowUp", "ArrowUp", 38, ""),
        "arrowdown" | "down" => ("ArrowDown", "ArrowDown", 40, ""),
        "arrowleft" | "left" => ("ArrowLeft", "ArrowLeft", 37, ""),
        "arrowright" | "right" => ("ArrowRight", "ArrowRight", 39, ""),
        "home" => ("Home", "Home", 36, ""),
        "end" => ("End", "End", 35, ""),
        "pageup" => ("PageUp", "PageUp", 33, ""),
        "pagedown" => ("PageDown", "PageDown", 34, ""),
        _ => return None,
    })
}

pub struct Tab {
    pub target_id: String,
    pub cdp: Arc<CdpSession>,
    mouse: StdMutex<(f64, f64)>,
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
    pub async fn new(target_id: String, ws_url: &str) -> Result<Self, String> {
        let cdp = CdpSession::connect(ws_url).await?;

        cdp.call("Page.enable", json!({})).await?;
        cdp.call("Runtime.enable", json!({})).await?;
        cdp.call("DOM.enable", json!({})).await?;
        cdp.call("Network.enable", json!({})).await?;
        cdp.call("Page.addScriptToEvaluateOnNewDocument", json!({ "source": EVASION_JS })).await?;

        Ok(Self {
            target_id,
            cdp,
            mouse: StdMutex::new((0.0, 0.0)),
            dialog_task: StdMutex::new(None),
            auth_task: StdMutex::new(None),
        })
    }

    // ───────────────────────────── Navigation ─────────────────────────────

    pub async fn get(&self, url: &str, timeout_ms: Option<u64>, wait_until: &str) -> Result<(), String> {
        let t_ms = timeout_ms.unwrap_or(30000);
        // Subscribe before navigating so a fast load event is not missed
        let mut rx = self.cdp.subscribe();
        let res = self.cdp.call("Page.navigate", json!({ "url": url })).await?;
        if let Some(err) = res.get("errorText").and_then(|v| v.as_str()) {
            if !err.is_empty() {
                return Err(format!("Điều hướng thất bại '{}': {}", url, err));
            }
        }
        // Same-document navigation (hash change) fires no load event
        if res.get("loaderId").is_none() {
            return Ok(());
        }
        self.wait_events(&mut rx, wait_until, t_ms).await
    }

    async fn wait_events(&self, rx: &mut crate::cdp::EventRx, wait_until: &str, t_ms: u64) -> Result<(), String> {
        match wait_until {
            "none" | "commit" => Ok(()),
            "domcontentloaded" => {
                let _ = CdpSession::wait_until(rx, t_ms, |m, _| m == "Page.domContentEventFired").await;
                Ok(())
            }
            "networkidle" => {
                let start = Instant::now();
                let _ = CdpSession::wait_until(rx, t_ms, |m, _| m == "Page.loadEventFired").await;
                let left = t_ms.saturating_sub(start.elapsed().as_millis() as u64);
                self.wait_network_idle_rx(rx, 500, left).await
            }
            _ => {
                let _ = CdpSession::wait_until(rx, t_ms, |m, _| m == "Page.loadEventFired").await;
                Ok(())
            }
        }
    }

    /// Resolves once no Network.* event has arrived for `idle_ms`.
    async fn wait_network_idle_rx(&self, rx: &mut crate::cdp::EventRx, idle_ms: u64, timeout_ms: u64) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(());
            }
            let window = Duration::from_millis(idle_ms).min(deadline - now);
            match tokio::time::timeout(window, rx.recv()).await {
                Err(_) => return Ok(()), // quiet window elapsed
                Ok(Ok((m, _))) if m.starts_with("Network.") => continue,
                Ok(Ok(_)) => continue,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(_)) => return Err("Kênh sự kiện CDP đã đóng".to_string()),
            }
        }
    }

    pub async fn wait_for_load(&self, state: &str, timeout_ms: u64) -> Result<(), String> {
        let mut rx = self.cdp.subscribe();
        let ready = self.evaluate_string("document.readyState").await.unwrap_or_default();
        match state {
            "networkidle" => self.wait_network_idle_rx(&mut rx, 500, timeout_ms).await,
            "domcontentloaded" if ready != "loading" => Ok(()),
            "load" if ready == "complete" => Ok(()),
            other => self.wait_events(&mut rx, other, timeout_ms).await,
        }
    }

    pub async fn reload(&self, ignore_cache: bool, timeout_ms: u64) -> Result<(), String> {
        let mut rx = self.cdp.subscribe();
        self.cdp.call("Page.reload", json!({ "ignoreCache": ignore_cache })).await?;
        self.wait_events(&mut rx, "load", timeout_ms).await
    }

    async fn history_step(&self, delta: i64, timeout_ms: u64) -> Result<bool, String> {
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

    pub async fn back(&self, timeout_ms: u64) -> Result<bool, String> {
        self.history_step(0 - 1, timeout_ms).await
    }

    pub async fn forward(&self, timeout_ms: u64) -> Result<bool, String> {
        self.history_step(1, timeout_ms).await
    }

    pub async fn bring_to_front(&self) -> Result<(), String> {
        self.cdp.call("Page.bringToFront", json!({})).await.map(|_| ())
    }

    pub async fn close(&self) -> Result<(), String> {
        self.cdp.call("Page.close", json!({})).await.map(|_| ())
    }

    // ───────────────────────────── JS / reading ─────────────────────────────

    pub async fn evaluate(&self, expression: &str) -> Result<Value, String> {
        let res = self.cdp.call("Runtime.evaluate", json!({
            "expression": expression,
            "returnByValue": true,
            "awaitPromise": true,
            "userGesture": true
        })).await?;

        if let Some(ex) = res.get("exceptionDetails") {
            let msg = ex.pointer("/exception/description")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| ex.to_string());
            return Err(format!("Lỗi thực thi JS: {}", msg));
        }
        Ok(res.pointer("/result/value").cloned().unwrap_or(Value::Null))
    }

    pub async fn evaluate_string(&self, expression: &str) -> Result<String, String> {
        Ok(match self.evaluate(expression).await? {
            Value::String(s) => s,
            Value::Null => String::new(),
            other => other.to_string(),
        })
    }

    pub async fn content(&self) -> Result<String, String> {
        self.evaluate_string("document.documentElement.outerHTML").await
    }

    pub async fn title(&self) -> Result<String, String> {
        self.evaluate_string("document.title").await
    }

    pub async fn current_url(&self) -> Result<String, String> {
        self.evaluate_string("window.location.href").await
    }

    pub async fn exists(&self, selector: &str) -> Result<bool, String> {
        Ok(self.evaluate(&format!("{} !== null", sel_expr(selector))).await? == Value::Bool(true))
    }

    pub async fn count(&self, selector: &str) -> Result<i64, String> {
        let v = self.evaluate(&format!("document.querySelectorAll({}).length", js_str(selector))).await?;
        Ok(v.as_i64().unwrap_or(0))
    }

    pub async fn text(&self, selector: &str) -> Result<Option<String>, String> {
        let v = self.evaluate(&format!(
            "(() => {{ const el = {}; return el ? (el.innerText ?? el.textContent ?? '').trim() : null; }})()",
            sel_expr(selector)
        )).await?;
        Ok(v.as_str().map(str::to_string))
    }

    pub async fn text_all(&self, selector: &str) -> Result<Vec<String>, String> {
        let v = self.evaluate(&format!(
            "Array.from(document.querySelectorAll({})).map(e => (e.innerText ?? e.textContent ?? '').trim())",
            js_str(selector)
        )).await?;
        Ok(v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default())
    }

    pub async fn attr(&self, selector: &str, name: &str) -> Result<Option<String>, String> {
        let v = self.evaluate(&format!(
            "(() => {{ const el = {}; return el ? el.getAttribute({}) : null; }})()",
            sel_expr(selector), js_str(name)
        )).await?;
        Ok(v.as_str().map(str::to_string))
    }

    pub async fn attr_all(&self, selector: &str, name: &str) -> Result<Vec<Option<String>>, String> {
        let v = self.evaluate(&format!(
            "Array.from(document.querySelectorAll({})).map(e => e.getAttribute({}))",
            js_str(selector), js_str(name)
        )).await?;
        Ok(v.as_array().map(|a| a.iter().map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default())
    }

    pub async fn value(&self, selector: &str) -> Result<Option<String>, String> {
        let v = self.evaluate(&format!(
            "(() => {{ const el = {}; return el && 'value' in el ? String(el.value) : null; }})()",
            sel_expr(selector)
        )).await?;
        Ok(v.as_str().map(str::to_string))
    }

    pub async fn inner_html(&self, selector: &str) -> Result<Option<String>, String> {
        let v = self.evaluate(&format!(
            "(() => {{ const el = {}; return el ? el.innerHTML : null; }})()",
            sel_expr(selector)
        )).await?;
        Ok(v.as_str().map(str::to_string))
    }

    // ───────────────────────────── Waiting ─────────────────────────────

    /// Polls a JS expression until it is truthy; returns its value.
    pub async fn wait_for_function(&self, expression: &str, timeout_ms: u64, poll_ms: u64) -> Result<Value, String> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let wrapped = format!("(() => {{ try {{ return ({}); }} catch (e) {{ return null; }} }})()", expression);
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
                return Err(format!("Timeout {}ms chờ điều kiện: {}", timeout_ms, expression));
            }
            tokio::time::sleep(Duration::from_millis(poll_ms)).await;
        }
    }

    pub async fn wait_for_selector(&self, selector: &str, timeout_ms: u64, visible: bool) -> Result<(), String> {
        let expr = if visible {
            format!(
                "(() => {{ const el = {}; if (!el) return false; const s = getComputedStyle(el); \
                 return s.visibility !== 'hidden' && s.display !== 'none' && el.getClientRects().length > 0; }})()",
                sel_expr(selector)
            )
        } else {
            format!("{} !== null", sel_expr(selector))
        };
        self.wait_for_function(expr.as_str(), timeout_ms, 100).await
            .map(|_| ())
            .map_err(|_| format!("Timeout chờ phần tử '{}' sau {}ms", selector, timeout_ms))
    }

    pub async fn wait_for_hidden(&self, selector: &str, timeout_ms: u64) -> Result<(), String> {
        let expr = format!(
            "(() => {{ const el = {}; if (!el) return true; const s = getComputedStyle(el); \
             return s.visibility === 'hidden' || s.display === 'none' || el.getClientRects().length === 0; }})()",
            sel_expr(selector)
        );
        self.wait_for_function(&expr, timeout_ms, 100).await
            .map(|_| ())
            .map_err(|_| format!("Timeout chờ phần tử '{}' biến mất sau {}ms", selector, timeout_ms))
    }

    pub async fn wait_for_text(&self, text: &str, timeout_ms: u64) -> Result<(), String> {
        let expr = format!("document.body && document.body.innerText.includes({})", js_str(text));
        self.wait_for_function(&expr, timeout_ms, 150).await
            .map(|_| ())
            .map_err(|_| format!("Timeout chờ văn bản '{}' sau {}ms", text, timeout_ms))
    }

    pub async fn wait_for_url(&self, contains: &str, timeout_ms: u64) -> Result<String, String> {
        let expr = format!("location.href.includes({}) ? location.href : null", js_str(contains));
        self.wait_for_function(&expr, timeout_ms, 150).await
            .map(|v| v.as_str().unwrap_or_default().to_string())
            .map_err(|_| format!("Timeout chờ URL chứa '{}' sau {}ms", contains, timeout_ms))
    }

    // ───────────────────────────── Mouse ─────────────────────────────

    /// JS expression → element center in viewport coords (scrolls it into view first).
    async fn element_center(&self, el_expr: &str, what: &str) -> Result<(f64, f64), String> {
        let expr = format!(
            "(() => {{ const el = {}; if (!el) return null; \
             el.scrollIntoView({{ block: 'center', inline: 'center', behavior: 'instant' }}); \
             const r = el.getBoundingClientRect(); \
             if (r.width === 0 && r.height === 0) return null; \
             return {{ x: r.left + r.width / 2, y: r.top + r.height / 2, w: r.width, h: r.height }}; }})()",
            el_expr
        );
        let pos = self.evaluate(&expr).await?;
        let x = pos.get("x").and_then(|v| v.as_f64());
        let y = pos.get("y").and_then(|v| v.as_f64());
        match (x, y) {
            (Some(x), Some(y)) => {
                // Land a little off-center like a real cursor
                let w = pos.get("w").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let h = pos.get("h").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let ox = (jitter(0, 1000).as_millis() as f64 / 1000.0 - 0.5) * (w * 0.3).min(10.0);
                let oy = (jitter(0, 1000).as_millis() as f64 / 1000.0 - 0.5) * (h * 0.3).min(6.0);
                Ok((x + ox, y + oy))
            }
            _ => Err(format!("Không tìm thấy hoặc không hiển thị: {}", what)),
        }
    }

    /// Moves the cursor along an eased path from its last position.
    pub async fn mouse_move(&self, x: f64, y: f64) -> Result<(), String> {
        let (sx, sy) = *self.mouse.lock().map_err(|e| e.to_string())?;
        let dist = ((x - sx).powi(2) + (y - sy).powi(2)).sqrt();
        let steps = ((dist / 40.0) as usize).clamp(4, 25);
        for i in 1..=steps {
            let t = i as f64 / steps as f64;
            let e = t * t * (3.0 - 2.0 * t); // smoothstep
            self.cdp.call("Input.dispatchMouseEvent", json!({
                "type": "mouseMoved",
                "x": sx + (x - sx) * e,
                "y": sy + (y - sy) * e
            })).await?;
            tokio::time::sleep(jitter(6, 10)).await;
        }
        *self.mouse.lock().map_err(|e| e.to_string())? = (x, y);
        Ok(())
    }

    pub async fn mouse_click(&self, x: f64, y: f64, button: &str, click_count: i64) -> Result<(), String> {
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

    pub async fn click(&self, selector: &str, button: &str, click_count: i64, timeout_ms: u64) -> Result<(), String> {
        self.wait_for_selector(selector, timeout_ms, true).await?;
        let (x, y) = self.element_center(&sel_expr(selector), selector).await?;
        self.mouse_click(x, y, button, click_count).await
    }

    /// Finds the deepest visible element whose text contains `text` and clicks it.
    pub async fn click_text(&self, text: &str, exact: bool, timeout_ms: u64) -> Result<(), String> {
        let finder = format!(
            "(() => {{ const t = {}.trim().toLowerCase(); const exact = {}; let best = null; \
             for (const el of document.querySelectorAll('body *')) {{ \
               if (el.getClientRects().length === 0) continue; \
               const s = (el.innerText || el.value || el.getAttribute('aria-label') || '').trim().toLowerCase(); \
               if (exact ? s === t : s.includes(t)) {{ if (!best || best.contains(el)) best = el; }} \
             }} return best; }})()",
            js_str(text), exact
        );
        self.wait_for_function(&format!("{} !== null", finder), timeout_ms, 150).await
            .map_err(|_| format!("Timeout tìm phần tử chứa văn bản '{}'", text))?;
        let (x, y) = self.element_center(&finder, text).await?;
        self.mouse_click(x, y, "left", 1).await
    }

    pub async fn hover(&self, selector: &str, timeout_ms: u64) -> Result<(), String> {
        self.wait_for_selector(selector, timeout_ms, true).await?;
        let (x, y) = self.element_center(&sel_expr(selector), selector).await?;
        self.mouse_move(x, y).await
    }

    pub async fn drag(&self, from_selector: &str, to_selector: &str, timeout_ms: u64) -> Result<(), String> {
        self.wait_for_selector(from_selector, timeout_ms, true).await?;
        self.wait_for_selector(to_selector, timeout_ms, true).await?;
        let (fx, fy) = self.element_center(&sel_expr(from_selector), from_selector).await?;
        self.mouse_move(fx, fy).await?;
        self.cdp.call("Input.dispatchMouseEvent", json!({
            "type": "mousePressed", "button": "left", "clickCount": 1, "x": fx, "y": fy
        })).await?;
        let (tx, ty) = self.element_center(&sel_expr(to_selector), to_selector).await?;
        // Movement while pressed must carry the button state
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
        *self.mouse.lock().map_err(|e| e.to_string())? = (tx, ty);
        Ok(())
    }

    // ───────────────────────────── Keyboard ─────────────────────────────

    pub async fn focus(&self, selector: &str, timeout_ms: u64) -> Result<(), String> {
        self.wait_for_selector(selector, timeout_ms, false).await?;
        self.evaluate(&format!("{}.focus()", sel_expr(selector))).await.map(|_| ())
    }

    pub async fn clear(&self, selector: &str, timeout_ms: u64) -> Result<(), String> {
        self.focus(selector, timeout_ms).await?;
        // Select-all + Backspace goes through the real input pipeline (React/Vue see it)
        self.evaluate(&format!(
            "(() => {{ const el = {}; if (el.select) el.select(); \
             else {{ const r = document.createRange(); r.selectNodeContents(el); const s = getSelection(); s.removeAllRanges(); s.addRange(r); }} }})()",
            sel_expr(selector)
        )).await?;
        self.press_key("Backspace").await
    }

    /// Types into the currently focused element, one key event pair per character.
    pub async fn type_text(&self, text: &str, delay_ms: u64) -> Result<(), String> {
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

    pub async fn send_keys(&self, selector: &str, text: &str, clear_first: bool, delay_ms: u64, timeout_ms: u64) -> Result<(), String> {
        if clear_first {
            self.clear(selector, timeout_ms).await?;
        } else {
            self.focus(selector, timeout_ms).await?;
        }
        self.type_text(text, delay_ms).await
    }

    /// Instant paste-like fill via Input.insertText (fires `input` events).
    pub async fn fill(&self, selector: &str, text: &str, timeout_ms: u64) -> Result<(), String> {
        self.clear(selector, timeout_ms).await?;
        self.cdp.call("Input.insertText", json!({ "text": text })).await.map(|_| ())
    }

    pub async fn press_key(&self, key: &str) -> Result<(), String> {
        let (k, code, vk, text) = key_def(key).ok_or_else(|| format!("Phím không hỗ trợ: {}", key))?;
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

    // ───────────────────────────── Forms ─────────────────────────────

    pub async fn select_option(&self, selector: &str, value: &str, timeout_ms: u64) -> Result<(), String> {
        self.wait_for_selector(selector, timeout_ms, false).await?;
        let ok = self.evaluate(&format!(
            "(() => {{ const el = {}; const v = {}; \
             const opt = Array.from(el.options).find(o => o.value === v || o.text.trim() === v); \
             if (!opt) return false; el.value = opt.value; \
             el.dispatchEvent(new Event('input', {{ bubbles: true }})); \
             el.dispatchEvent(new Event('change', {{ bubbles: true }})); return true; }})()",
            sel_expr(selector), js_str(value)
        )).await?;
        if ok == Value::Bool(true) { Ok(()) } else { Err(format!("Không có option '{}' trong '{}'", value, selector)) }
    }

    /// Ensures a checkbox/radio ends up in the requested state, clicking only if needed.
    pub async fn set_checked(&self, selector: &str, checked: bool, timeout_ms: u64) -> Result<(), String> {
        self.wait_for_selector(selector, timeout_ms, false).await?;
        let cur = self.evaluate(&format!("{}.checked === true", sel_expr(selector))).await?;
        if cur == Value::Bool(checked) {
            return Ok(());
        }
        // Styled checkboxes hide the <input>; fall back to its label
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

    pub async fn upload_file(&self, selector: &str, files: &[String], timeout_ms: u64) -> Result<(), String> {
        self.wait_for_selector(selector, timeout_ms, false).await?;
        let mut abs = Vec::with_capacity(files.len());
        for f in files {
            let p = std::fs::canonicalize(Path::new(f)).map_err(|e| format!("File không tồn tại '{}': {}", f, e))?;
            // canonicalize on Windows yields \\?\C:\..., Chrome wants a plain path
            abs.push(p.to_string_lossy().trim_start_matches(r"\\?\").to_string());
        }
        let res = self.cdp.call("Runtime.evaluate", json!({ "expression": sel_expr(selector) })).await?;
        let object_id = res.pointer("/result/objectId").and_then(|v| v.as_str())
            .ok_or_else(|| format!("Không lấy được objectId của '{}'", selector))?
            .to_string();
        let out = self.cdp.call("DOM.setFileInputFiles", json!({ "files": abs, "objectId": object_id })).await;
        let _ = self.cdp.call("Runtime.releaseObject", json!({ "objectId": object_id })).await;
        out.map(|_| ())
    }

    // ───────────────────────────── Scrolling ─────────────────────────────

    pub async fn scroll_by(&self, dx: f64, dy: f64) -> Result<(), String> {
        let vp = self.evaluate("({ w: innerWidth, h: innerHeight })").await?;
        let cx = vp.get("w").and_then(|v| v.as_f64()).unwrap_or(800.0) / 2.0;
        let cy = vp.get("h").and_then(|v| v.as_f64()).unwrap_or(600.0) / 2.0;
        // Split into wheel ticks the way a physical wheel would
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

    /// Wheel-scrolls until the page height stops growing (infinite feeds) or `max_rounds` is hit.
    pub async fn scroll_to_bottom(&self, step: f64, pause_ms: u64, max_rounds: u32) -> Result<(), String> {
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

    pub async fn scroll_to_top(&self) -> Result<(), String> {
        self.evaluate("window.scrollTo({ top: 0, behavior: 'instant' })").await.map(|_| ())
    }

    pub async fn scroll_into_view(&self, selector: &str, timeout_ms: u64) -> Result<(), String> {
        self.wait_for_selector(selector, timeout_ms, false).await?;
        self.evaluate(&format!("{}.scrollIntoView({{ block: 'center', behavior: 'instant' }})", sel_expr(selector)))
            .await.map(|_| ())
    }

    // ───────────────────────────── Emulation ─────────────────────────────

    pub async fn set_viewport(&self, width: i64, height: i64, scale: f64, mobile: bool) -> Result<(), String> {
        self.cdp.call("Emulation.setDeviceMetricsOverride", json!({
            "width": width, "height": height, "deviceScaleFactor": scale, "mobile": mobile
        })).await.map(|_| ())
    }

    pub async fn clear_viewport(&self) -> Result<(), String> {
        self.cdp.call("Emulation.clearDeviceMetricsOverride", json!({})).await.map(|_| ())
    }

    pub async fn set_user_agent(&self, ua: &str, accept_language: Option<&str>, platform: Option<&str>) -> Result<(), String> {
        let mut p = Map::new();
        p.insert("userAgent".into(), json!(ua));
        if let Some(l) = accept_language { p.insert("acceptLanguage".into(), json!(l)); }
        if let Some(pl) = platform { p.insert("platform".into(), json!(pl)); }
        self.cdp.call("Network.setUserAgentOverride", Value::Object(p)).await.map(|_| ())
    }

    pub async fn set_timezone(&self, timezone_id: &str) -> Result<(), String> {
        self.cdp.call("Emulation.setTimezoneOverride", json!({ "timezoneId": timezone_id })).await.map(|_| ())
    }

    pub async fn set_locale(&self, locale: &str) -> Result<(), String> {
        self.cdp.call("Emulation.setLocaleOverride", json!({ "locale": locale })).await.map(|_| ())
    }

    pub async fn set_geolocation(&self, latitude: f64, longitude: f64, accuracy: f64) -> Result<(), String> {
        let _ = self.cdp.call("Browser.grantPermissions", json!({ "permissions": ["geolocation"] })).await;
        self.cdp.call("Emulation.setGeolocationOverride", json!({
            "latitude": latitude, "longitude": longitude, "accuracy": accuracy
        })).await.map(|_| ())
    }

    // ───────────────────────────── Network ─────────────────────────────

    pub async fn set_extra_headers(&self, headers: Value) -> Result<(), String> {
        self.cdp.call("Network.setExtraHTTPHeaders", json!({ "headers": headers })).await.map(|_| ())
    }

    /// Patterns use `*` wildcards, e.g. ["*.png", "*google-analytics*"].
    pub async fn block_urls(&self, patterns: &[String]) -> Result<(), String> {
        self.cdp.call("Network.setBlockedURLs", json!({ "urls": patterns })).await.map(|_| ())
    }

    pub async fn set_cache_disabled(&self, disabled: bool) -> Result<(), String> {
        self.cdp.call("Network.setCacheDisabled", json!({ "cacheDisabled": disabled })).await.map(|_| ())
    }

    /// Answers proxy auth challenges for this tab via the Fetch domain.
    pub async fn enable_proxy_auth(&self, username: String, password: String) -> Result<(), String> {
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
                let rid = if let Some(r) = params.get("requestId").and_then(|v| v.as_str()) {
                    r.to_string()
                } else {
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
        if let Some(old) = self.auth_task.lock().map_err(|e| e.to_string())?.replace(handle) {
            old.abort();
        }
        Ok(())
    }

    // ───────────────────────────── Cookies & storage ─────────────────────────────

    pub async fn get_cookies(&self, urls: Option<Vec<String>>) -> Result<Value, String> {
        let res = if let Some(u) = urls {
            self.cdp.call("Network.getCookies", json!({ "urls": u })).await?
        } else {
            match self.cdp.call("Storage.getCookies", json!({})).await {
                Ok(v) => v,
                Err(_) => self.cdp.call("Network.getAllCookies", json!({})).await?,
            }
        };
        Ok(res.get("cookies").cloned().unwrap_or_else(|| json!([])))
    }

    /// Accepts cookies as returned by get_cookies (extra fields are stripped).
    pub async fn set_cookies(&self, cookies: Value) -> Result<(), String> {
        const KEEP: [&str; 9] = ["name", "value", "url", "domain", "path", "secure", "httpOnly", "sameSite", "expires"];
        let list = cookies.as_array().ok_or("cookies phải là list các dict")?;
        let mut clean = Vec::with_capacity(list.len());
        for c in list {
            let obj = c.as_object().ok_or("mỗi cookie phải là dict")?;
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
                return Err(format!("Cookie '{}' thiếu 'domain' hoặc 'url'", obj.get("name").and_then(|v| v.as_str()).unwrap_or("?")));
            }
            clean.push(Value::Object(m));
        }
        self.cdp.call("Network.setCookies", json!({ "cookies": clean })).await.map(|_| ())
    }

    pub async fn delete_cookies(&self, name: &str, domain: Option<&str>, url: Option<&str>) -> Result<(), String> {
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

    pub async fn clear_cookies(&self) -> Result<(), String> {
        self.cdp.call("Network.clearBrowserCookies", json!({})).await.map(|_| ())
    }

    pub async fn save_cookies(&self, path: &str) -> Result<usize, String> {
        let cookies = self.get_cookies(None).await?;
        let n = cookies.as_array().map(|a| a.len()).unwrap_or(0);
        let data = serde_json::to_vec_pretty(&cookies).map_err(|e| e.to_string())?;
        tokio::fs::write(path, data).await.map_err(|e| format!("Lỗi ghi '{}': {}", path, e))?;
        Ok(n)
    }

    pub async fn load_cookies(&self, path: &str) -> Result<usize, String> {
        let raw = tokio::fs::read(path).await.map_err(|e| format!("Lỗi đọc '{}': {}", path, e))?;
        let cookies: Value = serde_json::from_slice(&raw).map_err(|e| format!("JSON cookie hỏng: {}", e))?;
        let n = cookies.as_array().map(|a| a.len()).unwrap_or(0);
        self.set_cookies(cookies).await?;
        Ok(n)
    }

    fn storage_obj(session: bool) -> &'static str {
        if session { "sessionStorage" } else { "localStorage" }
    }

    pub async fn storage_get(&self, key: Option<&str>, session: bool) -> Result<Value, String> {
        let s = Self::storage_obj(session);
        if let Some(k) = key {
            self.evaluate(&format!("{}.getItem({})", s, js_str(k))).await
        } else {
            self.evaluate(&format!("Object.fromEntries(Object.entries({}))", s)).await
        }
    }

    pub async fn storage_set(&self, key: &str, value: &str, session: bool) -> Result<(), String> {
        let s = Self::storage_obj(session);
        self.evaluate(&format!("{}.setItem({}, {})", s, js_str(key), js_str(value))).await.map(|_| ())
    }

    pub async fn storage_clear(&self, session: bool) -> Result<(), String> {
        self.evaluate(&format!("{}.clear()", Self::storage_obj(session))).await.map(|_| ())
    }

    // ───────────────────────────── Dialogs ─────────────────────────────

    /// Auto-answers alert/confirm/prompt/beforeunload. Pass None to stop auto-handling.
    pub fn set_dialog_handler(&self, mode: Option<(bool, Option<String>)>) -> Result<(), String> {
        let mut slot = self.dialog_task.lock().map_err(|e| e.to_string())?;
        if let Some(old) = slot.take() {
            old.abort();
        }
        let (accept, prompt_text) = if let Some(m) = mode {
            m
        } else {
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

    pub async fn handle_dialog(&self, accept: bool, prompt_text: Option<&str>) -> Result<(), String> {
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
    ) -> Result<Vec<u8>, String> {
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
            let mut clip = r.as_object().cloned().ok_or("Không lấy được kích thước phần tử")?;
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
        let b64 = res.get("data").and_then(|v| v.as_str()).ok_or("Không nhận được dữ liệu screenshot từ CDP")?;
        let bytes = BASE64_STANDARD.decode(b64.trim()).map_err(|e| format!("Lỗi giải mã base64 screenshot: {}", e))?;

        if let Some(path) = file_path {
            tokio::fs::write(&path, &bytes).await.map_err(|e| format!("Lỗi ghi file ảnh '{}': {}", path, e))?;
        }
        Ok(bytes)
    }

    pub async fn pdf(&self, file_path: Option<String>, landscape: bool) -> Result<Vec<u8>, String> {
        let res = self.cdp.call("Page.printToPDF", json!({ "printBackground": true, "landscape": landscape })).await?;
        let b64 = res.get("data").and_then(|v| v.as_str()).ok_or("Không nhận được dữ liệu PDF")?;
        let bytes = BASE64_STANDARD.decode(b64.trim()).map_err(|e| format!("Lỗi giải mã base64 PDF: {}", e))?;
        if let Some(path) = file_path {
            tokio::fs::write(&path, &bytes).await.map_err(|e| format!("Lỗi ghi file PDF '{}': {}", path, e))?;
        }
        Ok(bytes)
    }

    pub async fn sleep(&self, seconds: f64) {
        tokio::time::sleep(Duration::from_secs_f64(seconds.max(0.0))).await;
    }
}
