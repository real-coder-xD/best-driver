// src/cdp.rs — Rust 2024, tokio + tokio-tungstenite: CDP JSON-RPC multiplexer over one WebSocket
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tokio_tungstenite::tungstenite::protocol::{Message, WebSocketConfig};
use tokio_tungstenite::connect_async_with_config;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;
pub type EventRx = broadcast::Receiver<(String, Value)>;

const CALL_TIMEOUT: Duration = Duration::from_secs(60);

pub struct CdpSession {
    next_id: AtomicU64,
    outgoing_tx: mpsc::UnboundedSender<String>,
    pending: Pending,
    event_tx: broadcast::Sender<(String, Value)>,
}

impl CdpSession {
    pub async fn connect(ws_url: &str) -> Result<Arc<Self>, String> {
        // Ảnh chụp full-page có thể vượt giới hạn frame 16MB mặc định
        let mut cfg = WebSocketConfig::default();
        cfg.max_message_size = None;
        cfg.max_frame_size = None;

        let (ws_stream, _) = connect_async_with_config(ws_url, Some(cfg), false)
            .await
            .map_err(|e| format!("Lỗi kết nối WebSocket CDP: {}", e))?;

        let (mut write, mut read) = ws_stream.split();
        let (outgoing_tx, mut outgoing_rx) = mpsc::unbounded_channel::<String>();
        let (event_tx, _) = broadcast::channel(4096);

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
                            let params = json.get("params").cloned().unwrap_or(Value::Null);
                            let _ = event_tx_clone.send((method.to_string(), params));
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            // Socket đóng: huỷ mọi lệnh đang chờ để caller không treo
            pending_recv.lock().await.clear();
        });

        Ok(Arc::new(Self {
            next_id: AtomicU64::new(1),
            outgoing_tx,
            pending,
            event_tx,
        }))
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let payload = serde_json::json!({ "id": id, "method": method, "params": params });

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        if let Err(e) = self.outgoing_tx.send(payload.to_string()) {
            self.pending.lock().await.remove(&id);
            return Err(format!("Không thể gửi lệnh qua CDP WebSocket: {}", e));
        }

        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(Ok(val))) => Ok(val),
            Ok(Ok(Err(err))) => Err(format!("CDP Error [{}]: {}", method, err)),
            Ok(Err(_)) => Err(format!("CDP Call [{}] bị huỷ hoặc kết nối đã đóng", method)),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(format!("CDP Call [{}] timeout sau {}s", method, CALL_TIMEOUT.as_secs()))
            }
        }
    }

    /// Đăng ký nhận event TRƯỚC khi gửi lệnh để không lỡ event đến sớm
    pub fn subscribe(&self) -> EventRx {
        self.event_tx.subscribe()
    }

    pub async fn wait_until<F>(rx: &mut EventRx, timeout_ms: u64, mut pred: F) -> Result<(String, Value), String>
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
                        return Err("Kênh sự kiện CDP đã đóng".to_string());
                    }
                }
            }
        };
        tokio::time::timeout(Duration::from_millis(timeout_ms), fut)
            .await
            .unwrap_or_else(|_| Err(format!("Timeout chờ sự kiện sau {}ms", timeout_ms)))
    }
}
