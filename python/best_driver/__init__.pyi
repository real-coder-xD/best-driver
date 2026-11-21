"""Type stubs for best-driver — Rust/Tokio undetected Chromium CDP driver.

Smart Locator syntax accepted across `Tab` and `Element` queries:
- CSS selector: `#login-btn`, `.card > a` (automatically pierces open `#shadow-root`)
- XPath: `//button[@type='submit']`, `(//li)[1]`, `.//span`
- Visible text: `text:Sign in`, `t:Submit`
- Attribute shorthand: `@name=username`, `@placeholder=Search`, `@disabled`
"""

from typing import TYPE_CHECKING, Any, AsyncIterator, Awaitable, Callable, Literal, Optional, Sequence, TypedDict, Union, overload

if TYPE_CHECKING:
    from http_tls import AsyncSession as HttpTlsAsyncSession  # type: ignore[import-not-found]
    from http_tls import Session as HttpTlsSession  # type: ignore[import-not-found]
else:
    HttpTlsAsyncSession = Any
    HttpTlsSession = Any

WaitUntil = Literal["load", "domcontentloaded", "networkidle", "none", "commit"]
MouseButton = Literal["left", "right", "middle"]
ImageFormat = Literal["png", "jpeg", "jpg", "webp"]
RouteStage = Literal["request", "response"]
LocatorState = Literal["attached", "visible", "hidden", "detached"]


class ViewportSize(TypedDict, total=False):
    """Viewport configuration passed to `browser.new_context(viewport=...)`."""

    width: int
    height: int
    scale: float
    mobile: bool


class FetchResponse(TypedDict):
    """Response dictionary returned by `Tab.fetch()` (executed inside Chromium's native TLS stack)."""

    url: str
    status: int
    status_text: str
    ok: bool
    headers: dict[str, str]
    body: str
SpecialKey = Literal[
    "Enter",
    "Return",
    "Tab",
    "Escape",
    "Esc",
    "Backspace",
    "Delete",
    "Del",
    "Space",
    "ArrowUp",
    "Up",
    "ArrowDown",
    "Down",
    "ArrowLeft",
    "Left",
    "ArrowRight",
    "Right",
    "Home",
    "End",
    "PageUp",
    "PageDown",
]


class ElementRect(TypedDict):
    """Viewport bounding box returned by `Element.rect`."""

    x: float
    y: float
    width: float
    height: float


ElementDict = TypedDict(
    "ElementDict",
    {
        "tag": str,
        "text": str,
        "value": Optional[str],
        "id": Optional[str],
        "class": Optional[str],
        "rect": ElementRect,
    },
)


class Cookie(TypedDict, total=False):
    """CDP cookie dictionary returned by `Tab.cookies()` / `Tab.get_cookies()`."""

    name: str
    value: str
    domain: str
    path: str
    url: str
    expires: float
    size: int
    httpOnly: bool
    secure: bool
    session: bool
    sameSite: Literal["Strict", "Lax", "None"]


class NetworkResponse(TypedDict):
    """Captured HTTP response returned by `Tab.wait_for_response`."""

    url: Optional[str]
    status: Optional[int]
    headers: Optional[dict[str, Any]]
    mime_type: Optional[str]
    body: Optional[str]
    base64_encoded: bool


class FrameInfo(TypedDict):
    """Metadata for a single frame returned by `Tab.frames`."""

    id: Optional[str]
    parent_id: Optional[str]
    name: Optional[str]
    url: Optional[str]


class DownloadInfo(TypedDict):
    """Completed file download metadata returned by `Tab.wait_for_download`."""

    guid: str
    url: str
    filename: str
    path: str
    total_bytes: int
    state: str


class BrowserFingerprint(TypedDict):
    """Live Chromium HTTP/TLS identity captured by `Tab.fingerprint`."""

    impersonate: str
    user_agent: str
    sec_ch_ua: Optional[str]
    sec_ch_ua_mobile: str
    sec_ch_ua_platform: str
    accept_language: str
    headers: dict[str, str]
    header_order: list[str]
    proxy: Optional[str]
    remote_ip: Optional[str]
    local_address: Optional[str]


class StorageState(TypedDict):
    """Playwright / `http_tls`-compatible state exported by `Tab.storage_state`."""

    cookies: list[Cookie]
    origins: list[Any]
    user_agent: Optional[str]
    headers: dict[str, str]
    impersonate: Optional[str]
    header_order: list[str]
    proxy: Optional[str]
    remote_ip: Optional[str]
    local_address: Optional[str]


class Element:
    """Native CDP element handle backed by `backendNodeId` (zero DOM mutation, Shadow DOM safe)."""

    def __getitem__(self, key: str) -> Any:
        """Dict-style metadata lookup (`tag`, `text`, `value`, `id`, `class`, `rect`)."""
        ...

    def get(self, key: str, default: Any = None) -> Any:
        """Safe dict-style metadata lookup returning `default` if absent."""
        ...

    @property
    def backend_node_id(self) -> int:
        """Chromium's internal `backendNodeId` for this DOM node."""
        ...

    @property
    def tag(self) -> str:
        """Lowercase HTML tag name (e.g. `'button'`, `'input'`)."""
        ...

    @property
    def text(self) -> str:
        """Trimmed text captured when the element was resolved."""
        ...

    @property
    def value(self) -> Optional[str]:
        """Current form control `.value`, if applicable."""
        ...

    @property
    def id(self) -> Optional[str]:
        """Element `id` attribute, or `None`."""
        ...

    @property
    def class_name(self) -> Optional[str]:
        """Element CSS class string, or `None`."""
        ...

    @property
    def rect(self) -> ElementRect:
        """Viewport bounding box (`x`, `y`, `width`, `height`)."""
        ...

    def to_dict(self) -> ElementDict:
        """Return captured metadata as a plain Python dictionary."""
        ...

    async def click(self, button: MouseButton = "left", click_count: int = 1) -> None:
        """Move cursor along a Cubic Bézier trajectory to this element and click."""
        ...

    async def hover(self) -> None:
        """Move cursor over this element using biological Bézier physics."""
        ...

    async def human_type(self, text: str, clear: bool = False) -> None:
        """Type into this element with biological key cadence and punctuation pauses."""
        ...

    async def clear(self) -> None:
        """Focus, select all, and press Backspace so React/Vue input listeners fire."""
        ...

    async def fill(self, text: str) -> None:
        """Clear and insert text instantaneously via CDP `Input.insertText`."""
        ...

    async def attr(self, name: str) -> Optional[str]:
        """Read a live attribute value from this DOM node."""
        ...

    async def inner_html(self) -> Optional[str]:
        """Read live `innerHTML` of this element."""
        ...

    async def outer_html(self) -> str:
        """Read live `outerHTML` of this element (works inside Shadow DOM)."""
        ...

    async def scroll_into_view(self) -> None:
        """Center this element in the viewport."""
        ...

    async def is_visible(self) -> bool:
        """Return `True` if this element has visible computed style and non-zero layout box."""
        ...

    async def is_enabled(self) -> bool:
        """Return `True` if this element is not disabled."""
        ...

    async def is_checked(self) -> bool:
        """Return `True` if this checkbox/radio is checked."""
        ...

    async def check(self) -> None:
        """Ensure this checkbox/radio is checked."""
        ...

    async def uncheck(self) -> None:
        """Ensure this checkbox is unchecked."""
        ...

    async def select_option(self, value: str) -> None:
        """Select a `<select>` option by `value` or visible label."""
        ...

    async def screenshot(self, file_path: Optional[str] = None) -> bytes:
        """Capture PNG screenshot clipped to this element; optionally save to `file_path`."""
        ...

    async def ele(self, query: str) -> Optional[Element]:
        """Find the first matching descendant (supports CSS, XPath, `text:`, `@attr`, and Shadow DOM)."""
        ...

    async def eles(self, query: str) -> list[Element]:
        """Find all matching descendants (supports CSS, XPath, `text:`, `@attr`, and Shadow DOM)."""
        ...


class Tab:
    """Controller for a browser tab or scoped `<iframe>` over a flattened CDP session."""

    @property
    def target_id(self) -> str:
        """Chromium DevTools target identifier."""
        ...

    @property
    def session_id(self) -> str:
        """Flattened CDP session identifier multiplexed over the browser WebSocket."""
        ...

    @property
    def frame_id(self) -> Optional[str]:
        """Frame ID if this controller is scoped to an in-process `<iframe>`, else `None`."""
        ...

    # Iframes & downloads
    async def iframe(self, selector: str, timeout_ms: int = 10000) -> Tab:
        """Locate an `<iframe>` via Smart Locator and return a scoped `Tab` controller (supports same-origin, nested, and OOPIF)."""
        ...

    async def frames(self) -> list[FrameInfo]:
        """Return all frames in the current page's frame tree."""
        ...

    async def set_download_dir(self, download_dir: str) -> str:
        """Configure Chromium's download directory and return the resolved absolute path."""
        ...

    async def wait_for_download(
        self,
        download_dir: str,
        timeout_ms: int = 30000,
    ) -> DownloadInfo:
        """Configure `download_dir` and await completion of the next file download."""
        ...

    # Navigation
    async def get(
        self,
        url: str,
        timeout_ms: Optional[int] = None,
        wait_until: WaitUntil = "load",
    ) -> None:
        """Navigate to `url` and wait for `"load"`, `"domcontentloaded"`, `"networkidle"`, or `"none"`."""
        ...

    async def wait_for_load(
        self,
        state: WaitUntil = "load",
        timeout_ms: int = 30000,
    ) -> None:
        """Wait until the page reaches the requested load state."""
        ...

    async def reload(self, ignore_cache: bool = False, timeout_ms: int = 30000) -> None:
        """Reload the current page."""
        ...

    async def back(self, timeout_ms: int = 30000) -> bool:
        """Navigate back in session history; returns `False` if already at start."""
        ...

    async def forward(self, timeout_ms: int = 30000) -> bool:
        """Navigate forward in session history; returns `False` if already at end."""
        ...

    async def bring_to_front(self) -> None:
        """Activate and focus this tab."""
        ...

    async def close(self) -> None:
        """Close this tab via `Target.closeTarget`."""
        ...

    # Reading & evaluation
    async def evaluate(self, expression: str) -> Any:
        """Evaluate a JS expression (awaits Promises) and return its JSON-deserialized value."""
        ...

    async def content(self) -> str:
        """Return full `document.documentElement.outerHTML`."""
        ...

    async def title(self) -> str:
        """Return `document.title`."""
        ...

    async def url(self) -> str:
        """Return `window.location.href`."""
        ...

    async def current_url(self) -> str:
        """Alias for `url()` returning `window.location.href`."""
        ...

    async def exists(self, selector: str) -> bool:
        """Return `True` if any element matches the Smart Locator `selector`."""
        ...

    async def count(self, selector: str) -> int:
        """Count all elements matching the Smart Locator (including inside Shadow DOM)."""
        ...

    async def text(self, selector: str) -> Optional[str]:
        """Return trimmed text of the first element matching `selector`, or `None`."""
        ...

    async def text_all(self, selector: str) -> list[str]:
        """Return trimmed text strings for all elements matching `selector`."""
        ...

    async def attr(self, selector: str, name: str) -> Optional[str]:
        """Read attribute `name` from the first matched element."""
        ...

    async def attr_all(self, selector: str, name: str) -> list[Optional[str]]:
        """Read attribute `name` from all matched elements."""
        ...

    async def value(self, selector: str) -> Optional[str]:
        """Read `.value` of a matched form input/textarea/select."""
        ...

    async def html(self, selector: str) -> Optional[str]:
        """Return `innerHTML` of the first matched element."""
        ...

    # Waiting
    async def wait_for(
        self,
        selector: str,
        timeout_ms: int = 10000,
        visible: bool = False,
    ) -> None:
        """Wait for an element matching `selector` to appear (raises `TimeoutError` on expiry)."""
        ...

    async def wait_for_selector(
        self,
        selector: str,
        timeout_ms: int = 10000,
        visible: bool = False,
    ) -> None:
        """Alias for `wait_for()`."""
        ...

    async def wait_for_hidden(self, selector: str, timeout_ms: int = 10000) -> None:
        """Wait until `selector` is detached or hidden."""
        ...

    async def wait_for_text(self, text: str, timeout_ms: int = 10000) -> None:
        """Wait until `text` appears anywhere in `document.body.innerText`."""
        ...

    async def wait_for_url(self, contains: str, timeout_ms: int = 10000) -> str:
        """Wait until `location.href` contains `contains` and return the matching URL."""
        ...

    async def wait_for_function(
        self,
        expression: str,
        timeout_ms: int = 10000,
        poll_ms: int = 100,
    ) -> Any:
        """Poll `expression` until truthy and return its value."""
        ...

    # Mouse & Smart Element API
    async def click(
        self,
        selector: str,
        button: MouseButton = "left",
        click_count: int = 1,
        timeout_ms: int = 10000,
    ) -> None:
        """Wait for `selector`, move along a Cubic Bézier arc, and click."""
        ...

    async def double_click(self, selector: str, timeout_ms: int = 10000) -> None:
        """Double-click the matched element."""
        ...

    async def right_click(self, selector: str, timeout_ms: int = 10000) -> None:
        """Right-click the matched element."""
        ...

    async def click_text(
        self,
        text: str,
        exact: bool = False,
        timeout_ms: int = 10000,
    ) -> None:
        """Find the deepest visible element (including Shadow DOM) containing `text` and click it."""
        ...

    async def mouse_click(
        self,
        x: float,
        y: float,
        button: MouseButton = "left",
        click_count: int = 1,
    ) -> None:
        """Move to viewport coordinates `(x, y)` and click."""
        ...

    async def mouse_move(self, x: float, y: float) -> None:
        """Move cursor to `(x, y)` along a biological Cubic Bézier trajectory."""
        ...

    async def hover(self, selector: str, timeout_ms: int = 10000) -> None:
        """Hover over the matched element."""
        ...

    async def drag(
        self,
        from_selector: str,
        to_selector: str,
        timeout_ms: int = 10000,
    ) -> None:
        """Drag from `from_selector` to `to_selector`."""
        ...

    async def swipe(self, x1: float, y1: float, x2: float, y2: float) -> None:
        """Perform an S-curve swipe gesture from `(x1, y1)` to `(x2, y2)` (for slider captchas)."""
        ...

    async def human_type(
        self,
        selector: str,
        text: str,
        clear: bool = False,
        timeout_ms: int = 10000,
    ) -> None:
        """Type `text` into `selector` with biological keystroke cadence and pauses."""
        ...

    async def ele(self, query: str, timeout_ms: int = 10000) -> Optional[Element]:
        """Locate one element using Smart Locators and return a native `Element` handle (or `None`)."""
        ...

    async def eles(self, query: str, timeout_ms: int = 10000) -> list[Element]:
        """Locate all elements matching `query` across DOM and Shadow Roots."""
        ...

    # Keyboard
    async def focus(self, selector: str, timeout_ms: int = 10000) -> None:
        """Focus the matched element."""
        ...

    async def clear(self, selector: str, timeout_ms: int = 10000) -> None:
        """Clear an input/editable element through the native selection + Backspace pipeline."""
        ...

    async def send_keys(
        self,
        selector: str,
        text: str,
        clear: bool = False,
        delay_ms: int = 60,
        timeout_ms: int = 10000,
    ) -> None:
        """Focus `selector` and type `text` with per-key delay jitter."""
        ...

    async def type_text(self, text: str, delay_ms: int = 60) -> None:
        """Type into whichever element currently has focus."""
        ...

    async def fill(self, selector: str, text: str, timeout_ms: int = 10000) -> None:
        """Clear `selector` and insert `text` immediately via `Input.insertText`."""
        ...

    async def press_key(self, key: Union[SpecialKey, str]) -> None:
        """Dispatch a special key (`"Enter"`, `"Tab"`, `"Escape"`, `"Backspace"`, `"ArrowDown"`, ...)."""
        ...

    # Forms
    async def select_option(
        self,
        selector: str,
        value: str,
        timeout_ms: int = 10000,
    ) -> None:
        """Select a `<select>` option by `value` or visible label."""
        ...

    async def check(self, selector: str, timeout_ms: int = 10000) -> None:
        """Ensure a checkbox/radio is checked (falls back to clicking its `<label>` if hidden)."""
        ...

    async def uncheck(self, selector: str, timeout_ms: int = 10000) -> None:
        """Ensure a checkbox is unchecked."""
        ...

    async def upload_file(
        self,
        selector: str,
        files: Union[str, Sequence[str]],
        timeout_ms: int = 10000,
    ) -> None:
        """Attach one or more local files to an `<input type="file">` without OS dialogs."""
        ...

    # Scrolling
    async def scroll_down(self, pixels: float = 600.0) -> None:
        """Wheel-scroll down by `pixels`."""
        ...

    async def scroll_up(self, pixels: float = 600.0) -> None:
        """Wheel-scroll up by `pixels`."""
        ...

    async def scroll_to_bottom(
        self,
        step: float = 800.0,
        pause_ms: int = 600,
        max_rounds: int = 50,
    ) -> None:
        """Wheel-scroll until page height stabilizes or `max_rounds` is reached."""
        ...

    async def scroll_to_top(self) -> None:
        """Scroll immediately to the top of the document."""
        ...

    async def scroll_into_view(self, selector: str, timeout_ms: int = 10000) -> None:
        """Scroll the matched element into the center of the viewport."""
        ...

    # Emulation
    async def set_viewport(
        self,
        width: int,
        height: int,
        scale: float = 1.0,
        mobile: bool = False,
    ) -> None:
        """Override viewport dimensions and device scale factor."""
        ...

    async def clear_viewport(self) -> None:
        """Reset viewport overrides to window defaults."""
        ...

    async def set_user_agent(
        self,
        user_agent: str,
        accept_language: Optional[str] = None,
        platform: Optional[str] = None,
    ) -> None:
        """Override `User-Agent`, `Accept-Language`, and `navigator.platform`."""
        ...

    async def set_timezone(self, timezone_id: str) -> None:
        """Spoof IANA timezone (e.g. `"Asia/Ho_Chi_Minh"`, `"America/New_York"`)."""
        ...

    async def set_locale(self, locale: str) -> None:
        """Override browser locale (e.g. `"en-US"`, `"vi-VN"`)."""
        ...

    async def set_geolocation(
        self,
        latitude: float,
        longitude: float,
        accuracy: float = 50.0,
    ) -> None:
        """Grant geolocation permission and spoof coordinates."""
        ...

    # Network
    async def set_extra_headers(self, headers: dict[str, str]) -> None:
        """Attach extra HTTP headers to every outgoing request in this tab."""
        ...

    async def block_urls(self, patterns: Sequence[str]) -> None:
        """Block matching URL wildcard patterns (e.g. `["*.png", "*analytics*"]`)."""
        ...

    async def wait_for_response(
        self,
        url_pattern: str,
        timeout_ms: int = 30000,
    ) -> NetworkResponse:
        """Await an HTTP response whose URL contains `url_pattern` and return headers + decoded body."""
        ...

    async def set_cache_disabled(self, disabled: bool = True) -> None:
        """Disable or re-enable browser HTTP cache for this tab."""
        ...

    # Cookies & storage
    async def set_cookie(
        self,
        name: str,
        value: str,
        domain: Optional[str] = None,
        path: Optional[str] = None,
    ) -> None:
        """Set a single cookie quickly by `name` and `value`."""
        ...

    async def cookies(self, urls: Optional[Sequence[str]] = None) -> list[Cookie]:
        """Alias for `get_cookies()`."""
        ...

    async def get_cookies(self, urls: Optional[Sequence[str]] = None) -> list[Cookie]:
        """Retrieve browser cookies (optionally filtered by `urls`)."""
        ...

    async def set_cookies(self, cookies: Sequence[Union[Cookie, dict[str, Any]]]) -> None:
        """Inject a list of CDP cookie dictionaries."""
        ...

    async def delete_cookies(
        self,
        name: str,
        domain: Optional[str] = None,
        url: Optional[str] = None,
    ) -> None:
        """Delete cookies matching `name` (and optional `domain`/`url`)."""
        ...

    async def clear_cookies(self) -> None:
        """Clear all cookies in the browser profile."""
        ...

    async def save_cookies(self, path: str) -> int:
        """Serialize all browser cookies to a JSON file and return cookie count."""
        ...

    async def load_cookies(self, path: str) -> int:
        """Load and inject cookies from a JSON file and return cookie count."""
        ...

    @overload
    async def storage_get(self, key: None = None, session: bool = False) -> dict[str, str]:
        """Read all entries from `localStorage` (`session=False`) or `sessionStorage`."""
        ...

    @overload
    async def storage_get(self, key: str, session: bool = False) -> Optional[str]:
        """Read one key from `localStorage` (`session=False`) or `sessionStorage`."""
        ...

    @overload
    async def storage_get(
        self,
        key: Optional[str] = None,
        session: bool = False,
    ) -> Union[dict[str, str], Optional[str]]:
        """Read one key or all entries from `localStorage` (`session=False`) or `sessionStorage`."""
        ...

    async def storage_set(self, key: str, value: str, session: bool = False) -> None:
        """Write a key/value pair into `localStorage` or `sessionStorage`."""
        ...

    async def storage_clear(self, session: bool = False) -> None:
        """Clear `localStorage` or `sessionStorage`."""
        ...

    # Dialogs
    def auto_dialog(
        self,
        accept: Optional[bool] = True,
        prompt_text: Optional[str] = None,
    ) -> None:
        """Automatically handle `alert`/`confirm`/`prompt` dialogs; pass `None` to disable."""
        ...

    async def handle_dialog(
        self,
        accept: bool = True,
        prompt_text: Optional[str] = None,
    ) -> None:
        """Respond to a currently open JavaScript dialog."""
        ...

    # Capture & raw CDP
    async def screenshot(
        self,
        file_path: Optional[str] = None,
        full_page: bool = False,
        format: ImageFormat = "png",
        quality: Optional[int] = None,
        selector: Optional[str] = None,
    ) -> bytes:
        """Capture PNG/JPEG/WEBP screenshot of viewport, full page, or target element."""
        ...

    async def pdf(
        self,
        file_path: Optional[str] = None,
        landscape: bool = False,
    ) -> bytes:
        """Render the current page to PDF bytes (optionally saving to `file_path`)."""
        ...

    async def sleep(self, seconds: float) -> None:
        """Non-blocking Tokio sleep."""
        ...

    async def send(
        self,
        method: str,
        params: Optional[dict[str, Any]] = None,
    ) -> Any:
        """Send an arbitrary raw CDP command scoped to this tab's flattened `sessionId`."""
        ...

    @property
    def proxy(self) -> Optional[str]:
        """Proxy URL inherited from the parent `Browser` or `BrowserContext`, or `None`."""
        ...

    @property
    def context_id(self) -> Optional[str]:
        """Owning `BrowserContext` identifier, or `None` for the default context."""
        ...

    async def add_init_script(self, script: str) -> str:
        """Inject JavaScript via `Page.addScriptToEvaluateOnNewDocument` before any page script runs."""
        ...

    async def fetch(
        self,
        url: str,
        method: str = "GET",
        headers: Optional[dict[str, str]] = None,
        body: Optional[str] = None,
        json: Any = None,
        credentials: str = "include",
    ) -> FetchResponse:
        """Execute a real in-browser `fetch()` over Chromium's native TLS stack and cookie jar."""
        ...

    async def fingerprint(self) -> BrowserFingerprint:
        """Capture the live Chromium HTTP/TLS identity (`impersonate`, UA, `Sec-CH-UA*`, `header_order`, `proxy`)."""
        ...

    async def storage_state(self, path: Optional[str] = None) -> StorageState:
        """Export all CDP cookies (including `HttpOnly`) and live browser headers in `http_tls`/Playwright format."""
        ...

    @overload
    async def tls_session(self, async_mode: Literal[True] = True, **kwargs: Any) -> HttpTlsAsyncSession:
        """Create an `http_tls.AsyncSession` synced with this tab's TLS preset, headers, proxy, and cookies."""
        ...

    @overload
    async def tls_session(self, async_mode: Literal[False], **kwargs: Any) -> HttpTlsSession:
        """Create a synchronous `http_tls.Session` synced with this tab's TLS preset, headers, proxy, and cookies."""
        ...

    @overload
    async def tls_session(self, async_mode: bool = True, **kwargs: Any) -> Union[HttpTlsAsyncSession, HttpTlsSession]:
        """Create an `http_tls.AsyncSession` (or `Session`) synced with this tab's TLS preset, headers, proxy, and cookies."""
        ...

    async def sync_from_tls(
        self,
        session: Union[HttpTlsAsyncSession, HttpTlsSession, Any],
        default_url: Optional[str] = None,
    ) -> int:
        """Push updated cookies from an `http_tls` session back into this Chromium tab; returns count synced."""
        ...

    async def route(
        self,
        url_pattern: str,
        handler: RouteHandler,
        stage: RouteStage = "request",
    ) -> None:
        """Intercept requests or responses (`stage='request'|'response'`) matching `url_pattern` via CDP Fetch."""
        ...

    async def unroute(self, url_pattern: Optional[str] = None) -> None:
        """Remove registered route(s) for `url_pattern` (or all routes when `None`)."""
        ...

    async def record_har(self, include_bodies: bool = True) -> HarRecorder:
        """Start recording HAR 1.2 network transactions on this tab."""
        ...

    async def export_har(self, path: Optional[str] = None) -> dict[str, Any]:
        """Return recorded HAR 1.2 data (optionally writing pretty JSON to `path`)."""
        ...

    async def stop_har(self) -> dict[str, Any]:
        """Stop HAR 1.2 recording on this tab and return the captured HAR dictionary."""
        ...

    def listen_responses(self, url_pattern: Optional[str] = None) -> ResponseStream:
        """Open a live async stream of incoming HTTP responses matching `url_pattern`."""
        ...

    def listen_requests(self, url_pattern: Optional[str] = None) -> RequestStream:
        """Open a live async stream of outgoing HTTP requests matching `url_pattern`."""
        ...

    def locator(self, query: str) -> Locator:
        """Create a lazy, auto-retrying Smart Locator chain resilient to SPA DOM re-renders."""
        ...


class EventStream:
    """Low-level Tokio-backed filtered CDP event stream."""

    async def next(self, timeout_ms: Optional[int] = None) -> Optional[dict[str, Any]]: ...
    def close(self) -> None: ...


class Route:
    """Intercepted CDP Fetch request or response passed to a `tab.route()` handler."""

    request_id: str
    url: str
    method: str
    headers: dict[str, str]
    post_data: Optional[str]
    resource_type: str
    response_status: Optional[int]
    response_headers: dict[str, str]
    stage: RouteStage
    handled: bool

    async def response_body(self) -> bytes:
        """Read the real server response body bytes (`stage == 'response'`)."""
        ...

    async def response_text(self, encoding: str = "utf-8") -> str:
        """Decode the intercepted server response body as text."""
        ...

    async def response_json(self) -> Any:
        """Parse the intercepted server response body as JSON."""
        ...

    async def fetch(
        self,
        *,
        url: Optional[str] = None,
        method: Optional[str] = None,
        headers: Optional[dict[str, str]] = None,
        body: Optional[str] = None,
    ) -> FetchResponse:
        """Perform the real upstream HTTP request via `http_tls` (if installed) or `tab.fetch()`."""
        ...

    async def continue_(
        self,
        *,
        url: Optional[str] = None,
        method: Optional[str] = None,
        headers: Optional[dict[str, str]] = None,
        post_data: Optional[Union[str, bytes]] = None,
    ) -> None:
        """Continue the intercepted request (or response) with optional overrides."""
        ...

    async def continue_response(
        self,
        *,
        status: Optional[int] = None,
        headers: Optional[dict[str, str]] = None,
    ) -> None:
        """Continue an intercepted response (`stage == 'response'`) with optional status/header edits."""
        ...

    async def fulfill(
        self,
        *,
        status: int = 200,
        body: Union[str, bytes] = "",
        headers: Optional[dict[str, str]] = None,
        content_type: Optional[str] = None,
        json: Any = None,
    ) -> None:
        """Fulfill the intercepted request or replace the intercepted server response."""
        ...

    async def abort(self, error_reason: str = "Failed") -> None:
        """Abort the intercepted request with a Chromium network error reason."""
        ...


class HarRecorder:
    """Background HAR 1.2 network recorder attached to a `Tab`."""

    entries: list[dict[str, Any]]

    def start(self) -> None: ...
    async def stop(self) -> None: ...
    def to_har(self) -> dict[str, Any]: ...


RouteHandler = Callable[[Route], Union[None, Awaitable[None]]]


class StreamedRequest:
    """Outgoing HTTP request yielded by `tab.listen_requests()`."""

    request_id: str
    url: str
    method: str
    headers: dict[str, str]
    post_data: Optional[str]
    resource_type: str
    timestamp: float


class StreamedResponse:
    """Incoming HTTP response yielded by `tab.listen_responses()` with lazy body decoding."""

    request_id: str
    url: str
    status: int
    status_text: str
    headers: dict[str, str]
    mime_type: str
    remote_ip: str
    protocol: str
    from_cache: bool

    async def body(self) -> bytes:
        """Fetch raw response body bytes via `Network.getResponseBody`."""
        ...

    async def text(self, encoding: str = "utf-8") -> str:
        """Decode response body as text."""
        ...

    async def json(self) -> Any:
        """Parse response body as JSON."""
        ...


class RequestStream(AsyncIterator[StreamedRequest]):
    """Async stream of outgoing HTTP requests (`async with` + `async for`)."""

    async def next(self, timeout_ms: Optional[int] = None) -> Optional[StreamedRequest]: ...
    def close(self) -> None: ...
    def __aiter__(self) -> RequestStream: ...
    async def __anext__(self) -> StreamedRequest: ...
    async def __aenter__(self) -> RequestStream: ...
    async def __aexit__(self, *args: Any) -> bool: ...


class ResponseStream(AsyncIterator[StreamedResponse]):
    """Async stream of incoming HTTP responses (`async with` + `async for`)."""

    async def next(self, timeout_ms: Optional[int] = None) -> Optional[StreamedResponse]: ...
    def close(self) -> None: ...
    def __aiter__(self) -> ResponseStream: ...
    async def __anext__(self) -> StreamedResponse: ...
    async def __aenter__(self) -> ResponseStream: ...
    async def __aexit__(self, *args: Any) -> bool: ...


class Locator:
    """Playwright-style lazy, auto-retrying Smart Locator chain."""

    def locator(self, query: str) -> Locator:
        """Chain a descendant Smart Locator query (CSS, XPath, `text:`, `@attr`, open/closed Shadow DOM)."""
        ...

    def filter(
        self,
        *,
        has_text: Optional[str] = None,
        has_not_text: Optional[str] = None,
    ) -> Locator:
        """Filter matched elements at the current step by substring inclusion/exclusion."""
        ...

    @property
    def first(self) -> Locator:
        """Select the first match (`nth(0)`)."""
        ...

    @property
    def last(self) -> Locator:
        """Select the last match (`nth(-1)`)."""
        ...

    def nth(self, index: int) -> Locator:
        """Select the `index`-th match at the current step."""
        ...

    async def element(self, timeout_ms: int = 10000) -> Optional[Element]: ...
    async def all(self, timeout_ms: int = 10000) -> list[Element]: ...
    async def count(self) -> int: ...
    async def exists(self) -> bool: ...
    async def is_visible(self) -> bool: ...
    async def is_enabled(self) -> bool: ...
    async def is_checked(self) -> bool: ...
    async def wait_for(self, timeout_ms: int = 10000, state: LocatorState = "visible") -> None: ...
    async def wait_for_hidden(self, timeout_ms: int = 10000) -> None: ...
    async def click(
        self,
        button: MouseButton = "left",
        click_count: int = 1,
        timeout_ms: int = 10000,
    ) -> None: ...
    async def double_click(self, timeout_ms: int = 10000) -> None: ...
    async def right_click(self, timeout_ms: int = 10000) -> None: ...
    async def hover(self, timeout_ms: int = 10000) -> None: ...
    async def human_type(self, text: str, clear: bool = False, timeout_ms: int = 10000) -> None: ...
    async def fill(self, text: str, timeout_ms: int = 10000) -> None: ...
    async def clear(self, timeout_ms: int = 10000) -> None: ...
    async def check(self, timeout_ms: int = 10000) -> None: ...
    async def uncheck(self, timeout_ms: int = 10000) -> None: ...
    async def select_option(self, value: str, timeout_ms: int = 10000) -> None: ...
    async def press_key(self, key: Union[SpecialKey, str], timeout_ms: int = 10000) -> None: ...
    async def text(self, timeout_ms: int = 10000) -> str: ...
    async def attr(self, name: str, timeout_ms: int = 10000) -> Optional[str]: ...
    async def value(self, timeout_ms: int = 10000) -> Optional[str]: ...
    async def inner_html(self, timeout_ms: int = 10000) -> Optional[str]: ...
    async def outer_html(self, timeout_ms: int = 10000) -> str: ...
    async def scroll_into_view(self, timeout_ms: int = 10000) -> None: ...
    async def screenshot(self, file_path: Optional[str] = None, timeout_ms: int = 10000) -> bytes: ...


class BrowserContext:
    """Isolated incognito browser context (separate cookies, storage, and optional proxy) inside one Chromium process."""

    @property
    def context_id(self) -> str:
        """Chromium CDP `browserContextId`."""
        ...

    @property
    def proxy(self) -> Optional[str]:
        """Proxy URL bound specifically to this context, or `None`."""
        ...

    async def add_init_script(self, script: str) -> None:
        """Inject JavaScript before any page script runs for all current and future tabs in this context."""
        ...

    async def main_tab(self) -> Tab:
        """Return the primary active tab in this context (creating one if none exist)."""
        ...

    async def new_tab(self, url: str = "about:blank") -> Tab:
        """Open a new isolated tab inside this `BrowserContext`."""
        ...

    async def tabs(self) -> list[Tab]:
        """List all open page tabs belonging to this `BrowserContext`."""
        ...

    async def get(
        self,
        url: str,
        new_tab: bool = False,
        wait_until: WaitUntil = "load",
    ) -> Tab:
        """Navigate a tab in this context to `url`."""
        ...

    async def close(self) -> None:
        """Dispose this `BrowserContext` via `Target.disposeBrowserContext`, wiping its tabs, cookies, and storage."""
        ...

    async def __aenter__(self) -> BrowserContext: ...
    async def __aexit__(self, *args: Any) -> bool: ...


class Browser:
    """Chromium instance managed over a single flattened browser-level CDP WebSocket."""

    @property
    def port(self) -> int:
        """Loopback remote debugging TCP port assigned by the OS."""
        ...

    @property
    def user_data_dir(self) -> str:
        """Filesystem path of the active Chromium profile directory."""
        ...

    @property
    def ws_url(self) -> str:
        """Browser-level CDP WebSocket endpoint URL."""
        ...

    @property
    def proxy(self) -> Optional[str]:
        """Proxy URL passed to `best_driver.start(proxy=...)`, or `None`."""
        ...

    async def is_alive(self) -> bool:
        """Return `True` if the Chromium child process is still running."""
        ...

    async def main_tab(self) -> Tab:
        """Return the primary active tab (creating one if none exist)."""
        ...

    async def new_tab(self, url: str = "about:blank") -> Tab:
        """Open a new tab via `Target.createTarget` with evasion armed before navigation."""
        ...

    async def tabs(self) -> list[Tab]:
        """List all currently open page tabs via `Target.getTargets`."""
        ...

    async def get(
        self,
        url: str,
        new_tab: bool = False,
        wait_until: WaitUntil = "load",
    ) -> Tab:
        """Navigate the main tab (or a new tab when `new_tab=True`) to `url`."""
        ...

    async def new_context(
        self,
        proxy: Optional[str] = None,
        proxy_bypass_list: Optional[str] = None,
        user_agent: Optional[str] = None,
        accept_language: Optional[str] = None,
        platform: Optional[str] = None,
        viewport: Optional[ViewportSize] = None,
        locale: Optional[str] = None,
        timezone: Optional[str] = None,
        extra_headers: Optional[dict[str, str]] = None,
    ) -> BrowserContext:
        """Create an isolated incognito `BrowserContext` with separate cookies, storage, proxy, and emulation overrides."""
        ...

    async def stop(self) -> None:
        """Gracefully close Chromium (`Browser.close`), flushing profile state before terminating."""
        ...

    async def __aenter__(self) -> Browser: ...
    async def __aexit__(self, *args: Any) -> bool: ...


async def start(
    headless: bool = False,
    user_data_dir: Optional[str] = None,
    browser_path: Optional[str] = None,
    extra_args: Optional[Sequence[str]] = None,
    proxy: Optional[str] = None,
) -> Browser:
    """Launch Chromium over a single flattened browser-level CDP WebSocket.

    Args:
        headless: Run with `--headless=new` (auto-strips `HeadlessChrome` from UA & Sec-CH-UA).
        user_data_dir: Persistent profile directory; auto-cleaned temp directory when `None`.
        browser_path: Explicit path to Chrome/Brave/Edge/Chromium binary; auto-detected when `None`.
        extra_args: Additional Chromium command-line flags.
        proxy: Proxy URL (`"http://host:port"`, `"http://user:pass@host:port"`, `"socks5://host:port"`).
    """
    ...


@overload
async def tls_session(tab: Tab, async_mode: Literal[True] = True, **kwargs: Any) -> HttpTlsAsyncSession:
    """Create an `http_tls.AsyncSession` synchronized with `tab`."""
    ...


@overload
async def tls_session(tab: Tab, async_mode: Literal[False], **kwargs: Any) -> HttpTlsSession:
    """Create a synchronous `http_tls.Session` synchronized with `tab`."""
    ...


@overload
async def tls_session(
    tab: Tab,
    async_mode: bool = True,
    **kwargs: Any,
) -> Union[HttpTlsAsyncSession, HttpTlsSession]:
    """Create an `http_tls.AsyncSession` (or `Session`) synchronized with `tab`."""
    ...


async def sync_from_tls(
    tab: Tab,
    session: Union[HttpTlsAsyncSession, HttpTlsSession, Any],
    default_url: Optional[str] = None,
) -> int:
    """Push cookies from an `http_tls` session back into `tab`."""
    ...


__all__ = [
    "Browser",
    "BrowserContext",
    "BrowserFingerprint",
    "Cookie",
    "DownloadInfo",
    "Element",
    "ElementDict",
    "ElementRect",
    "EventStream",
    "FetchResponse",
    "FrameInfo",
    "HarRecorder",
    "ImageFormat",
    "Locator",
    "LocatorState",
    "MouseButton",
    "NetworkResponse",
    "RequestStream",
    "ResponseStream",
    "Route",
    "RouteHandler",
    "RouteStage",
    "SpecialKey",
    "StorageState",
    "StreamedRequest",
    "StreamedResponse",
    "Tab",
    "ViewportSize",
    "WaitUntil",
    "start",
    "sync_from_tls",
    "tls_session",
]
