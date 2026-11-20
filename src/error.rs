// src/error.rs — Rust 2021: typed error hierarchy for best_driver
use std::fmt;

#[derive(Debug, Clone)]
pub enum DriverError {
    BrowserClosed,
    Launch(String),
    Connection(String),
    Cdp { method: String, detail: String },
    JsException(String),
    Timeout(String),
    NotFound(String),
    StaleElement(i64),
    InvalidInput(String),
    Io(String),
}

pub type DriverResult<T> = Result<T, DriverError>;

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BrowserClosed => write!(f, "Browser đã đóng"),
            Self::Launch(msg) => write!(f, "Lỗi khởi chạy trình duyệt: {}", msg),
            Self::Connection(msg) => write!(f, "Lỗi kết nối CDP: {}", msg),
            Self::Cdp { method, detail } => write!(f, "CDP Error [{}]: {}", method, detail),
            Self::JsException(msg) => write!(f, "Lỗi thực thi JS: {}", msg),
            Self::Timeout(msg) => write!(f, "Timeout: {}", msg),
            Self::NotFound(msg) => write!(f, "Không tìm thấy phần tử: {}", msg),
            Self::StaleElement(id) => write!(f, "Phần tử (backendNodeId={}) không còn tồn tại trong DOM", id),
            Self::InvalidInput(msg) => write!(f, "Tham số không hợp lệ: {}", msg),
            Self::Io(msg) => write!(f, "Lỗi I/O: {}", msg),
        }
    }
}

impl std::error::Error for DriverError {}

impl From<String> for DriverError {
    fn from(s: String) -> Self {
        Self::Connection(s)
    }
}
