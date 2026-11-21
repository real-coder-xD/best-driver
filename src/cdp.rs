// src/cdp.rs — Rust 2021, tokio + tokio-tungstenite: single browser-level CDP WebSocket with flattened session multiplexing
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::protocol::{Message, WebSocketConfig};

use crate::error::{DriverError, DriverResult};

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

const CALL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct CdpEvent {
    pub session_id: Option<String>,
    pub method: String,
    pub params: Value,
}

/// Single browser-level CDP WebSocket multiplexer supporting flattened sessions (`sessionId` routing).
pub struct CdpConnection {
    next_id: AtomicU64,
    outgoing_tx: mpsc::UnboundedSender<String>,
    pending: Pending,
    event_tx: broadcast::Sender<CdpEvent>,
}

impl CdpConnection {
    pub async fn connect(ws_url: &str) -> DriverResult<Arc<Self>> {
        // Full-page screenshots and large response bodies can exceed tungstenite's default 16MB cap
        let mut cfg = WebSocketConfig::default();
        cfg.max_message_size = None;
        cfg.max_frame_size = None;

        let (ws_stream, _) = connect_async_with_config(ws_url, Some(cfg), false)
            .await
            .map_err(|e| DriverError::Connection(format!("Không kết nối được WebSocket CDP: {}", e)))?;

        let (mut write, mut read) = ws_stream.split();
        let (outgoing_tx, mut outgoing_rx) = mpsc::unbounded_channel::<String>();
        let (event_tx, _) = broadcast::channel(8192);

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let pending_recv = Arc::clone(&pending);
        let event_tx_clone = event_tx.clone();

        tokio::spawn(async move {
            while let Some(msg) = outgoing_rx.recv().await {
                if write.send(Message::Text(msg.into())).await.is_err() {
                    break;
                }
            }
            let _ = write.close().await;
        });

        tokio::spawn(async move {
            while let Some(Ok(msg)) = read.next().await {
                match msg {
                    Message::Text(text) => {
                        let Ok(json) = serde_json::from_str::<Value>(&text) else { continue };
                        if let Some(id) = json.get("id").and_then(|v| v.as_u64()) {
                            let chan = pending_recv.lock().await.remove(&id);
                            if let Some(chan) = chan {
                                let res = match json.get("error") {
                                    Some(err) => Err(err.to_string()),
                                    None => Ok(json.get("result").cloned().unwrap_or(Value::Null)),
                                };
                                let _ = chan.send(res);
                            }
                        } else if let Some(method) = json.get("method").and_then(|m| m.as_str()) {
                            let session_id = json
                                .get("sessionId")
                                .and_then(|s| s.as_str())
                                .map(str::to_string);
                            let params = json.get("params").cloned().unwrap_or(Value::Null);
                            let _ = event_tx_clone.send(CdpEvent {
                                session_id,
                                method: method.to_string(),
                                params,
                            });
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            // Socket closed: fail all in-flight callers so nothing hangs
            pending_recv.lock().await.clear();
        });

        Ok(Arc::new(Self {
            next_id: AtomicU64::new(1),
            outgoing_tx,
            pending,
            event_tx,
        }))
    }

    pub async fn call_raw(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
    ) -> DriverResult<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let payload = match session_id {
            Some(sid) => json!({ "id": id, "sessionId": sid, "method": method, "params": params }),
            None => json!({ "id": id, "method": method, "params": params }),
        };

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        if let Err(e) = self.outgoing_tx.send(payload.to_string()) {
            self.pending.lock().await.remove(&id);
            return Err(DriverError::Connection(format!("Không thể gửi lệnh CDP [{}]: {}", method, e)));
        }

        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(Ok(val))) => Ok(val),
            Ok(Ok(Err(detail))) => Err(DriverError::Cdp {
                method: method.to_string(),
                detail,
            }),
            Ok(Err(_)) => Err(DriverError::Connection(format!(
                "CDP Call [{}] bị huỷ hoặc kết nối đã đóng",
                method
            ))),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(DriverError::Timeout(format!(
                    "CDP Call [{}] timeout sau {}s",
                    method,
                    CALL_TIMEOUT.as_secs()
                )))
            }
        }
    }

    /// Sends a browser-level CDP command (no `sessionId`).
    pub async fn call(&self, method: &str, params: Value) -> DriverResult<Value> {
        self.call_raw(None, method, params).await
    }

    /// Creates a lightweight flattened session handle over this shared WebSocket.
    pub fn session(self: &Arc<Self>, session_id: String) -> Arc<CdpSession> {
        Arc::new(CdpSession {
            conn: Arc::clone(self),
            session_id,
        })
    }

    /// Subscribes to ALL events across the browser WebSocket (all sessions).
    pub fn subscribe(&self) -> broadcast::Receiver<CdpEvent> {
        self.event_tx.subscribe()
    }

    /// Emits an internal lifecycle event on the shared broadcast bus (e.g. after auto-attach resume).
    pub fn emit_internal(&self, method: &str, params: Value) {
        let _ = self.event_tx.send(CdpEvent {
            session_id: None,
            method: method.to_string(),
            params,
        });
    }
}

/// Lightweight per-target flattened session handle multiplexed over the single browser `CdpConnection`.
pub struct CdpSession {
    conn: Arc<CdpConnection>,
    pub session_id: String,
}

enum SessionFilter {
    Session(String),
    Popups {
        opener_target_id: String,
        parent_session_id: String,
    },
}

/// Event stream filtered strictly to one flattened `sessionId` or popups opened by a parent target.
pub struct SessionEventRx {
    rx: broadcast::Receiver<CdpEvent>,
    filter: SessionFilter,
}

impl SessionEventRx {
    pub async fn recv(&mut self) -> Result<(String, Value), broadcast::error::RecvError> {
        loop {
            match self.rx.recv().await {
                Ok(ev) => match &self.filter {
                    SessionFilter::Session(sid) => {
                        if ev.session_id.as_deref() == Some(sid.as_str()) {
                            return Ok((ev.method, ev.params));
                        }
                        // Terminate session streams immediately when their CDP target detaches
                        // (tab.close(), ctx.close(), or JS window.close()).
                        if ev.method == "Target.detachedFromTarget"
                            && ev.params.get("sessionId").and_then(|v| v.as_str())
                                == Some(sid.as_str())
                        {
                            return Err(broadcast::error::RecvError::Closed);
                        }
                    }
                    SessionFilter::Popups {
                        opener_target_id,
                        parent_session_id,
                    } => {
                        if ev.method == "Target.detachedFromTarget"
                            && ev.params.get("sessionId").and_then(|v| v.as_str())
                                == Some(parent_session_id.as_str())
                        {
                            return Err(broadcast::error::RecvError::Closed);
                        }
                        if ev.method == "_internal.popupReady" {
                            let opener = ev
                                .params
                                .pointer("/targetInfo/openerId")
                                .and_then(|v| v.as_str());
                            if opener == Some(opener_target_id.as_str()) {
                                return Ok((ev.method, ev.params));
                            }
                        }
                    }
                },
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(e) => return Err(e),
            }
        }
    }
}

impl CdpSession {
    /// Sends a CDP command scoped to this target's flattened `sessionId`.
    pub async fn call(&self, method: &str, params: Value) -> DriverResult<Value> {
        self.conn.call_raw(Some(&self.session_id), method, params).await
    }

    /// Sends a browser-level CDP command (e.g. `Target.closeTarget`, `Browser.grantPermissions`).
    pub async fn call_browser(&self, method: &str, params: Value) -> DriverResult<Value> {
        self.conn.call(method, params).await
    }

    /// Subscribes to CDP events belonging ONLY to this session.
    pub fn subscribe(&self) -> SessionEventRx {
        SessionEventRx {
            rx: self.conn.subscribe(),
            filter: SessionFilter::Session(self.session_id.clone()),
        }
    }

    /// Subscribes to browser-level `Target.targetCreated` popup events opened by `opener_target_id`.
    pub fn subscribe_popups(&self, opener_target_id: String) -> SessionEventRx {
        SessionEventRx {
            rx: self.conn.subscribe(),
            filter: SessionFilter::Popups {
                opener_target_id,
                parent_session_id: self.session_id.clone(),
            },
        }
    }

    /// Subscribes to ALL events on the underlying browser WebSocket (used for browser-scoped events like downloads).
    pub fn subscribe_all(&self) -> broadcast::Receiver<CdpEvent> {
        self.conn.subscribe()
    }

    /// Returns the shared browser-level CDP connection (for attaching child OOPIF sessions).
    pub fn connection(&self) -> &Arc<CdpConnection> {
        &self.conn
    }

    pub async fn wait_until<F>(
        rx: &mut SessionEventRx,
        timeout_ms: u64,
        mut pred: F,
    ) -> DriverResult<(String, Value)>
    where
        F: FnMut(&str, &Value) -> bool,
    {
        let fut = async {
            loop {
                match rx.recv().await {
                    Ok((method, params)) => {
                        if pred(&method, &params) {
                            return Ok((method, params));
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(DriverError::Connection("Kênh sự kiện CDP đã đóng".to_string()));
                    }
                }
            }
        };
        tokio::time::timeout(Duration::from_millis(timeout_ms), fut)
            .await
            .unwrap_or_else(|_| {
                Err(DriverError::Timeout(format!(
                    "Chờ sự kiện CDP quá {}ms",
                    timeout_ms
                )))
            })
    }
}
