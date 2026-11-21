# best-driver — Undetected Rust-Powered Chromium Automation for Python

**High-performance, GIL-free Python browser automation and web scraping built in pure Rust (`PyO3` + `Tokio`) over a single flattened Chrome DevTools Protocol (CDP) WebSocket.**

[![PyPI version](https://img.shields.io/pypi/v/best-driver?style=flat-square&color=blue)](https://pypi.org/project/best-driver/)
[![Python](https://img.shields.io/badge/python-3.8%2B-blue.svg?style=flat-square)](https://www.python.org/)
[![Typing](https://img.shields.io/badge/typing-PEP%20561%20typed-007acc.svg?style=flat-square)](https://peps.python.org/pep-0561/)
[![Rust](https://img.shields.io/badge/rust-2021%20%7C%20PyO3%200.30%20%7C%20Tokio-orange.svg?style=flat-square)](https://www.rust-lang.org/)
[![Sannysoft Headless](https://img.shields.io/badge/sannysoft%20headless-57%2F57%20passed-success.svg?style=flat-square)](https://bot.sannysoft.com)
[![Wheels](https://img.shields.io/badge/wheels-14%20platforms-brightgreen.svg?style=flat-square)](https://pypi.org/project/best-driver/#files)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg?style=flat-square)](https://github.com/real-coder-xD/best-driver/blob/main/LICENSE)

[Overview](#overview) | [Comparison](#best-driver-vs-playwright-selenium-drissionpage-and-nodriver) | [Installation](#installation) | [Quickstart](#quickstart) | [Smart Locators & Shadow DOM](#smart-locators-and-shadow-dom) | [Element Handles](#element-handles) | [Iframes](#working-with-iframes) | [Network & Downloads](#network-interception-and-downloads) | [Sessions & Proxies](#cookies-storage-profiles-and-proxies) | [Input Physics](#biological-input-physics-and-forms) | [Stealth Architecture](#stealth-and-anti-detection-architecture) | [Error Handling & Typing](#error-handling-and-typing) | [API Reference](#api-reference) | [Architecture](#architecture-under-the-hood) | [Migration Guide](#migration-guide) | [FAQ](#frequently-asked-questions)

---

## Overview

`best-driver` is an open-source, driverless Chromium automation engine for Python built as a native Rust extension (`cdylib`). It is designed as a fast, low-memory, stealth-first alternative to Playwright, Selenium, DrissionPage, nodriver, and undetected-chromedriver.

Instead of wrapping `chromedriver` over HTTP or delegating commands to a Node.js subprocess, `best-driver` launches Chrome, Chromium, Brave, or Microsoft Edge directly and multiplexes all tabs, popups, and out-of-process iframes (OOPIFs) over a **single browser-level CDP WebSocket** using flattened sessions (`Target.attachToTarget` with `flatten: true`).

### Core Design Highlights

- **Single Flattened WebSocket Multiplexer**: One persistent Tokio WebSocket serves all tabs, popups, and OOPIFs via `sessionId` routing. Target discovery and creation use `Target.getTargets` and `Target.createTarget` directly over WebSocket with zero HTTP client crates (`reqwest` and TLS are not in the dependency tree).
- **GIL-Free Native Async I/O**: JSON-RPC 2.0 framing, WebSocket I/O, Base64 decoding, concurrent `DOM.describeNode` fan-out (`join_all`), and cursor trajectory calculations run natively in Rust on Tokio without holding the Python GIL.
- **Direct FFI Serialization**: Python primitives (`dict`, `list`, `int`, `float`, `bool`, `str`, `None`) map directly into `serde_json::Value` in Rust without intermediate `json.dumps` allocations.
- **Headless Stealth Verified at 57/57 on Sannysoft (0 failures, 0 warnings)**: Automatically strips `HeadlessChrome` from HTTP `User-Agent` and `Sec-CH-UA` client hints, patches strictly on DOM prototypes (`Object.getOwnPropertyNames(navigator) == []`), isolates canvas noise via offscreen cloning so transparent probe pixels stay intact, and auto-arms popups and OOPIFs before their first JavaScript statement runs.
- **Zero-DOM-Mutation Element Handles**: `tab.ele()` and `tab.eles()` bind directly to Chromium's internal `backendNodeId` (`DOM.describeNode` + `DOM.resolveNode`), avoiding synthetic DOM attributes that `MutationObserver` anti-bot scripts flag.
- **Unified Smart Locators, Shadow DOM & Iframes**: Every selector method resolves CSS, XPath (`//`), visible text (`text:` / `t:`), and attribute shorthands (`@name=q`) while automatically traversing open `#shadow-root` subtrees; `tab.iframe()` provides full control over same-origin, nested, `srcdoc`, and cross-origin OOPIF frames.
- **100% PEP 561 Typed**: Ships `best_driver/__init__.pyi` (117/117 public symbols verified against the compiled runtime) and `py.typed` for complete autocomplete and static checking in VSCode/Pylance, PyCharm, Mypy, and Pyright.

---

## best-driver vs Playwright, Selenium, DrissionPage, and nodriver

Comparison of architectural design and built-in capabilities (only factual architectural properties and locally verified `best-driver` benchmarks are listed):

| Capability | best-driver (v0.1.4) | DrissionPage | Playwright (Python) | nodriver | Selenium / undetected-chromedriver |
| :--- | :--- | :--- | :--- | :--- | :--- |
| Core runtime | Rust + Tokio (native `cdylib` extension) | Python (sync) | Node.js subprocess + IPC | Python (asyncio) | Python + `chromedriver` binary |
| Transport protocol | Single flattened CDP WebSocket | Per-tab CDP + HTTP | Flattened CDP via Node IPC | Per-tab CDP WebSocket | W3C WebDriver HTTP |
| External driver dependency | None | None | Bundled Node.js driver | None | `chromedriver` binary |
| Python GIL impact on I/O | None (native Tokio threads) | GIL-bound | Cross-process IPC pipe | GIL-bound | GIL-bound |
| Smart locators (`//`, `text:`, `@attr`, CSS) | Built-in, unified across all methods | Built-in, unified | Prefixed (`text=`, `xpath=`) | Basic CSS | Explicit `By.*` enums |
| Automatic open Shadow DOM piercing | Yes (all locator methods) | Yes | Partial (CSS selectors) | No | Manual JS traversal |
| Element handle DOM mutation | None (native CDP `backendNodeId`) | Node / locator refs | Internal handles | Raw CDP nodes | WebDriver element IDs |
| Iframe support | Same-origin, nested, `srcdoc`, OOPIF | Built-in | `frame_locator` | Manual | `switch_to.frame` |
| Popup & OOPIF pre-execution stealth | Built-in (`Target.setAutoAttach`) | Manual | Not stealth-focused | Manual | None |
| Network response body capture | `tab.wait_for_response()` | `listen.wait()` | `page.wait_for_response()` | Manual CDP events | External proxy required |
| Download manager | `tab.wait_for_download()` | Built-in | `page.expect_download()` | Manual | Browser prefs workaround |
| Built-in headless stealth | 57/57 Sannysoft headless (0 fail, 0 warn) | Basic CDP evasion | Disabled by default | Basic CDP evasion | Binary-patched ChromeDriver (`uc`) |
| Cursor & typing physics | Cubic Bezier + biological cadence | Linear / instant | Synthetic / instant | Linear / instant | None |
| PEP 561 type stubs (`.pyi`) | Full (117/117 symbols + `py.typed`) | Partial | Full | Partial | Full |

### Measured Local Throughput (`best-driver` v0.1.4, Windows 11, Headless Chrome)

- Browser launch + flattened browser WebSocket ready: **~680 ms**
- Opening **20 concurrent tabs** (`asyncio.gather` over 1 shared WebSocket): **~295 ms**
- Executing **20 concurrent `tab.evaluate()` calls** across all 20 tabs: **~5.4 ms**

---

## Installation

### Requirements

- Python `>= 3.8` (CPython 3.8-3.14 and PyPy supported).
- A locally installed Chromium-based browser: Google Chrome, Chromium, Brave, or Microsoft Edge.

### Install Pre-Built Wheels (Recommended)

Pre-built wheels are published to PyPI for **14 targets** (Windows x64/x86/ARM64, Linux manylinux x86_64/x86/aarch64/armv7, Musllinux x86_64/x86/aarch64/armv7, macOS Intel/Apple Silicon) plus source distribution:

```bash
pip install best-driver
```

### Browser Auto-Discovery

When `browser_path=None`, `best_driver.start()` searches standard installation locations in order before falling back to `PATH` (`chrome`, `chromium`, `google-chrome`, `msedge`):

- **Windows**: `Program Files` / `Program Files (x86)` / `%LOCALAPPDATA%` for Google Chrome, Brave, and Microsoft Edge.
- **macOS**: `/Applications/Google Chrome.app`, `/Applications/Brave Browser.app`, `/Applications/Microsoft Edge.app`.
- **Linux**: `chrome`, `chromium`, `google-chrome`, `msedge` via `PATH`.

Pass `browser_path="/custom/path/to/chrome"` to target a specific binary.

### Build from Source

Requires the Rust toolchain (`cargo`) and `maturin` (no OpenSSL or C system libraries needed):

```bash
git clone https://github.com/real-coder-xD/best-driver.git
cd best-driver
pip install maturin
maturin build --release
pip install --force-reinstall target/wheels/*.whl
```

---

## Quickstart

```python
import asyncio
import best_driver


async def main() -> None:
    async with await best_driver.start(headless=True) as browser:
        tab = await browser.get("https://bot.sannysoft.com", wait_until="networkidle")

        print("Title:", await tab.title())
        print("URL:", await tab.url())
        print("navigator.webdriver:", await tab.evaluate("navigator.webdriver"))  # False
        print("Has HeadlessChrome UA:", "HeadlessChrome" in await tab.evaluate("navigator.userAgent"))  # False

        # Smart locator returning a native CDP Element handle
        heading = await tab.ele("h1")
        if heading is not None:
            print("Heading:", heading.tag, heading.text, heading.rect)

        await tab.screenshot("sannysoft.png", full_page=True)


if __name__ == "__main__":
    asyncio.run(main())
```

---

## Smart Locators and Shadow DOM

Every query-accepting method on `Tab` (`ele`, `eles`, `click`, `double_click`, `right_click`, `hover`, `drag`, `human_type`, `send_keys`, `fill`, `clear`, `focus`, `text`, `text_all`, `attr`, `attr_all`, `value`, `html`, `exists`, `count`, `wait_for`, `wait_for_selector`, `wait_for_hidden`, `select_option`, `check`, `uncheck`, `upload_file`, `scroll_into_view`, `screenshot`, `iframe`) and `Element` (`ele`, `eles`) uses one unified Smart Locator engine.

### Locator Syntax Rules

| Prefix / Pattern | Resolution Mode | Shadow DOM Piercing | Examples |
| :--- | :--- | :--- | :--- |
| `//`, `(//`, `.//` | XPath (`document.evaluate`) | Standard document XPath | `"//button[@type='submit']"`, `"(//li[@class='row'])[1]"`, `".//span"` |
| `text:` or `t:` | Deepest visible element whose `innerText`, `textContent`, `value`, or `aria-label` contains substring (case-insensitive) | Automatic recursive walk | `"text:Sign In"`, `"t:Confirm order"` |
| `@attr` or `@attr=value` | CSS attribute selector with automatic escaping of `\` and `"` | Automatic deep traversal | `"@name=email"`, `"@placeholder=Enter password"`, `"@disabled"` |
| Anything else | Standard CSS selector | Direct query first, then recursive `#shadow-root` traversal | `"#login-btn"`, `".card > a[href]"`, `"input[type='email']"` |

### Example

```python
# Locate by text
await tab.click("text:Sign In")
await tab.click("t:Continue")

# Locate by attribute shorthand (quotes optional; special chars escaped automatically)
await tab.human_type("@placeholder=Enter username", "operator_01", clear=True)
await tab.click("@data-testid=submit-button")

# Locate by XPath
first_title = await tab.text("(//article//h2)[1]")

# Locate inside open #shadow-root transparently (no frame or shadow switching needed)
shadow_button = await tab.ele("#button-inside-shadow-root")
if shadow_button:
    print(await shadow_button.outer_html())
    await shadow_button.click()

# Multi-element extraction also pierces Shadow DOM
total = await tab.count(".shadow-item")
labels = await tab.text_all(".shadow-item")
```

---

## Element Handles

`tab.ele(query, timeout_ms=10000)` returns `Optional[Element]` and `tab.eles(query, timeout_ms=10000)` returns `list[Element]`.

### How `Element` Works Internally

1. Evaluates the Smart Locator with `returnByValue: false` to obtain a V8 `objectId`.
2. Calls `DOM.describeNode` to resolve Chromium's internal integer `backendNodeId`, captures snapshot metadata (`tag`, `text`, `value`, `id`, `class`, `rect`), and immediately frees the temporary V8 handle via `Runtime.releaseObject`.
3. Subsequent actions on `Element` resolve `backendNodeId` via `DOM.resolveNode`, execute via `Runtime.callFunctionOn`, and unconditionally release the handle on every exit path.
4. For `tab.eles()`, all matched nodes are described concurrently with `futures_util::future::join_all`.

Because `best-driver` never stamps custom attributes onto page nodes, `MutationObserver` anti-bot listeners record zero DOM mutations.

### Properties and Methods

```python
card = await tab.ele(".product-card")
if card is not None:
    # Fast snapshot properties (captured at resolution time)
    print(card.tag)              # "div"
    print(card.text)             # trimmed innerText
    print(card.id)               # element id or None
    print(card.class_name)       # class string (SVG-safe) or None
    print(card.value)            # form value or None
    print(card.rect)             # {"x": ..., "y": ..., "width": ..., "height": ...}
    print(card.backend_node_id)  # Chromium internal backendNodeId

    # Backward-compatible dict access
    print(card["tag"], card.get("id", "none"), card.to_dict())

    # Live DOM queries & chained descendant lookups (CSS, XPath, text:, @attr, Shadow DOM)
    title_el = await card.ele(".title")
    tags = await card.eles(".badge")
    href = await card.attr("data-href")
    markup = await card.outer_html()

    # Element-scoped actions
    await card.scroll_into_view()
    await card.screenshot("card.png")
    await card.hover()
    await card.click()
```

---

## Working with Iframes

`tab.iframe(selector, timeout_ms=10000)` returns a full `Tab` controller scoped to the target `<iframe>`. It handles both Chromium frame models automatically:

1. **Out-of-Process IFrames (OOPIFs / cross-origin isolated targets)**: Detects the matching `iframe` target via `Target.getTargets`, attaches using `Target.attachToTarget(flatten=True)` on the existing browser WebSocket, and returns a dedicated flattened session `Tab`.
2. **In-Process IFrames (same-origin, `srcdoc`, `data:`, or non-isolated cross-origin)**: Resolves the frame's `frameId` via `DOM.describeNode`, runs JS evaluations inside `Page.createIsolatedWorld({ frameId, grantUniveralAccess: true })`, and accumulates exact top-level viewport offsets from `DOM.getFrameOwner` (including borders and padding) across arbitrary nesting depths so mouse movements and clicks land on the exact pixel.

```python
# Inspect the entire frame tree
for f in await tab.frames():
    print(f["id"], f["parent_id"], f["name"], f["url"])

# Interact inside an iframe using the exact same Tab / Element API
frame = await tab.iframe("#checkout-frame")
await frame.human_type("@name=card_number", "4242424242424242")
pay_btn = await frame.ele("text:Pay Now")
if pay_btn:
    await pay_btn.click()

# Nested iframe-in-iframe works recursively
inner = await frame.iframe("#three-ds-frame")
code_text = await inner.text("#challenge-code")
```

---

## Network Interception and Downloads

### Important Note on PyO3 Async Futures

Async methods in `best-driver` are backed by `pyo3_async_runtimes::tokio::future_into_py`. Calling a method like `tab.wait_for_response(...)` or `tab.wait_for_download(...)` returns an `asyncio.Future` that **starts executing on Tokio immediately** — you do not need (and should not pass it to) `asyncio.create_task()`. Store the future, trigger the browser action, then `await` the future.

### Sniffing AJAX / JSON Responses (`wait_for_response`)

`tab.wait_for_response(url_pattern, timeout_ms=30000)` subscribes to the tab's filtered CDP event stream, matches `Network.responseReceived` where `url` contains `url_pattern`, waits on the same stream for `Network.loadingFinished` (or `Network.loadingFailed`) for that `requestId` so large payloads finish transferring, and calls `Network.getResponseBody` (automatically decoding UTF-8 base64 payloads).

```python
import json

resp_fut = tab.wait_for_response("/api/v1/items", timeout_ms=15000)
await tab.click("#load-more")
resp = await resp_fut

print(resp["status"], resp["mime_type"], resp["url"])
payload = json.loads(resp["body"])
```

Returned `NetworkResponse` keys: `url`, `status`, `headers`, `mime_type`, `body`, `base64_encoded`.

### Request Blocking, Custom Headers, and Cache Control

```python
# Block heavy assets and trackers using CDP wildcard patterns
await tab.block_urls(["*.png", "*.jpg", "*.woff2", "*google-analytics*"])

# Attach extra headers to every request in this tab
await tab.set_extra_headers({"X-Custom-Header": "1", "Accept-Language": "en-US,en;q=0.9"})

# Disable HTTP cache
await tab.set_cache_disabled(True)
```

### Automated File Downloads (`wait_for_download`)

`tab.wait_for_download(download_dir, timeout_ms=30000)` creates `download_dir` if missing, subscribes to browser download events before enabling `Browser.setDownloadBehavior(behavior="allow", eventsEnabled=True)`, and resolves once `downloadProgress` reaches `state == "completed"`.

```python
dl_fut = tab.wait_for_download("./downloads", timeout_ms=30000)
await tab.sleep(0.1)
await tab.click("text:Export CSV")
info = await dl_fut

print(info["filename"], info["total_bytes"], info["path"])
# {"guid": "...", "url": "...", "filename": "report.csv", "path": "/abs/path/report.csv", "total_bytes": 4096, "state": "completed"}
```

If you only want to configure the directory ahead of time without waiting immediately, call `abs_path = await tab.set_download_dir("./downloads")`.

---

## Cookies, Storage, Profiles, and Proxies

### Cookies and Web Storage

```python
# Quick single cookie injection
await tab.set_cookie("session_id", "abc123", domain="example.com", path="/")

# Inspect cookies (tab.cookies() is an alias for tab.get_cookies())
all_cookies = await tab.cookies()
domain_cookies = await tab.get_cookies(["https://example.com"])

# Persist and restore cookie jars across sessions
count = await tab.save_cookies("session_cookies.json")
loaded = await tab.load_cookies("session_cookies.json")

await tab.delete_cookies("session_id")
await tab.clear_cookies()

# localStorage (session=False) and sessionStorage (session=True)
await tab.storage_set("theme", "dark", session=False)
val = await tab.storage_get("theme", session=False)
all_items = await tab.storage_get(None, session=False)
await tab.storage_clear(session=False)
```

### Persistent User Profiles and Graceful Shutdown

When `user_data_dir` is omitted, `best-driver` creates a temporary profile directory and removes it automatically via RAII (`tempfile::TempDir`) when the browser stops.

When `user_data_dir="./my_profile"` is provided, `browser.stop()` (and `async with` exit) first sends `Browser.close` and waits up to 1.5 seconds for Chromium to exit cleanly so SQLite databases (`Cookies`, `Local Storage`, `Preferences`) flush cleanly to disk before falling back to process termination.

```python
async with await best_driver.start(headless=True, user_data_dir="./my_profile") as browser:
    assert await browser.is_alive() is True
    tab = await browser.get("https://example.com")
```

### Authenticated HTTP and SOCKS5 Proxies

Inline proxy credentials (`http://user:pass@host:port`) are split automatically before launching Chromium; authentication challenges are answered natively via CDP `Fetch.authRequired` without requiring browser extensions:

```python
browser = await best_driver.start(
    headless=True,
    proxy="http://username:password@proxy.example.com:8080",
)
```

---

## Biological Input Physics and Forms

- **Cubic Bezier Cursor Movement**: `mouse_move()`, `mouse_click()`, `click()`, `hover()`, and `Element.click()` move along a randomized 4-point Cubic Bezier arc with smoothstep acceleration/deceleration easing, sub-pixel micro-tremors, and slight off-center target landing inside the element's bounding box.
- **Biological Typing (`human_type`)**: Dispatches real `keyDown`/`keyUp` CDP events with variable hold durations and natural inter-key pauses at spaces, punctuation (`.`, `,`, `!`, `?`), and word boundaries. Supports `\n` to press Enter.
- **Instant Input (`fill`)**: Clears via native selection + Backspace and inserts text immediately via `Input.insertText` (fires React/Vue `input` events).
- **Slider Captcha Gestures (`swipe`)**: Smooth S-curve drag with perpendicular micro-jitter while holding `buttons: 1`.
- **Special Keys (`press_key`)**: Supports `Enter` / `Return`, `Tab`, `Escape` / `Esc`, `Backspace`, `Delete` / `Del`, `Space`, `ArrowUp` / `Up`, `ArrowDown` / `Down`, `ArrowLeft` / `Left`, `ArrowRight` / `Right`, `Home`, `End`, `PageUp`, `PageDown` (case-insensitive).
- **Forms & File Uploads**: `select_option()` matches `<option>` by `value` or visible text; `check()` / `uncheck()` click only when state differs and fall back to associated `<label>` elements for custom-styled hidden inputs; `upload_file()` attaches one or multiple absolute paths via `DOM.setFileInputFiles` without triggering OS file dialogs.

```python
await tab.human_type("@name=query", "rust cdp driver\n", clear=True)
await tab.select_option("#country", "United States")
await tab.check("#accept-terms")
await tab.upload_file("input[type='file']", ["report.pdf", "summary.png"])
await tab.swipe(120.0, 340.0, 380.0, 342.0)
```

---

## Stealth and Anti-Detection Architecture

### Six Defense Layers

1. **Headless UA and Client Hints Alignment**: At launch, `BrowserCore` queries `Browser.getVersion`. Whenever `HeadlessChrome/` is present, `Emulation.setUserAgentOverride` is applied across all sessions with synchronized `userAgent` and `userAgentMetadata` (`brands`, `fullVersionList`, `platform`, `platformVersion`, `architecture`, `bitness`) matching the real installed Chrome version and host OS.
2. **Pre-Execution Auto-Attach for Popups and OOPIFs**: The browser connection runs `Target.setAutoAttach(autoAttach=True, waitForDebuggerOnStart=True, flatten=True)` supervised by a background Tokio task synchronized with per-target `tokio::sync::OnceCell`. Every newly spawned `window.open()` popup or cross-origin OOPIF receives `evasion.js` and the UA override before its first JavaScript instruction runs, then resumes unconditionally via `Runtime.runIfWaitingForDebugger`.
3. **Prototype-Only Native Patching (`src/evasion.js`)**:
   - `Navigator.prototype.webdriver` getter returns `false` with authentic descriptor (`'webdriver' in Navigator.prototype === true`, `configurable: true`, `enumerable: true`).
   - All patched getters and methods live on their real prototypes (`Navigator.prototype`, `Permissions.prototype`, `SpeechSynthesis.prototype`, `MediaDevices.prototype`) using non-constructable concise method syntax (`'prototype' in fn === false`) masked via `Function.prototype.toString` to return `function <name>() { [native code] }`.
   - `Object.getOwnPropertyNames(navigator)` and `Object.getOwnPropertyNames(navigator.permissions)` remain empty (`[]`).
4. **Zero-DOM-Mutation Locators**: `Element` handles use Chromium's internal `backendNodeId` rather than injecting DOM attributes.
5. **Safe Canvas, Audio, WebGL, and Environment Consistency**:
   - WebGL `UNMASKED_VENDOR_WEBGL` / `UNMASKED_RENDERER_WEBGL` spoofed to `Google Inc. (NVIDIA)` / `ANGLE (NVIDIA, NVIDIA GeForce RTX 4070 Direct3D11 vs_5_0 ps_5_0, D3D11)`.
   - Canvas noise uses a per-session shift applied via an offscreen clone in `toDataURL`, skipping transparent pixels (`alpha === 0`) and small probe canvases (`< 16px`) so live canvases are never mutated and `TRANSPARENT_PIXEL` tests stay `[0, 0, 0, 0]`.
   - AudioContext applies a tiny once-per-buffer salt tracked in a `WeakSet`.
   - Provides realistic fallback entries for `SpeechSynthesis.getVoices()`, `MediaDevices.enumerateDevices()`, `navigator.connection` (`4g`), `PluginArray` / `MimeTypeArray`, `window.chrome`, and `outerWidth` / `outerHeight` window frame offsets when headless omits them.
6. **Biological Input Physics**: Cubic Bezier cursor arcs, micro-tremors, variable keystroke cadence, and S-curve swipe physics.

### Verified Headless Benchmark (`bot.sannysoft.com`)

Verified in `--headless=new` mode (`headless=True`) with strict CSS-class inspection (`failed` and `warn` counts across all 57 rows):

| Check Group | Observed Result | Status |
| :--- | :--- | :--- |
| `User Agent (Old)` & `HEADCHR_UA` | `Chrome/<host-version>` (no `HeadlessChrome`) | Passed |
| `WebDriver (New)` & `WebDriver Advanced` | `missing (passed)` / `passed` | Passed |
| `Chrome (New)` & `Permissions (New)` | `present (passed)` / `default` | Passed |
| `Plugins Length` & `PluginArray` type | `5` / `passed` | Passed |
| `Languages` & `CHR_MEMORY` | `en-US,en` / `ok` | Passed |
| `WebGL Vendor` & `WebGL Renderer` | `Google Inc. (NVIDIA)` / `RTX 4070 Direct3D11` | Passed |
| `TRANSPARENT_PIXEL` & `Broken Image Dimensions` | `ok` (`[0, 0, 0, 0]`) / `16x16` | Passed |
| `HEADCHR_*`, `PHANTOM_*`, `SELENIUM_DRIVER`, `CHR_DEBUG_TOOLS` | `ok` (0 failures) | Passed |
| `Object.getOwnPropertyNames(navigator)` | `[]` | Passed |
| `MutationObserver` during `ele()` / `eles()` | `[]` | Passed |
| **Total Sannysoft Headless Score** | **57 / 57 passed (0 failed, 0 warnings)** | **Passed** |

In addition to `bot.sannysoft.com`, live headless verification confirmed DuckDuckGo search submission via `@name=q` + `human_type` and clean page load on `nowsecure.nl` without triggering an interstitial challenge.

### Known Limitations on Stock Chromium

`best-driver` intentionally runs on stock, unmodified Chromium binaries so users do not have to maintain custom browser builds. Because `Tab::new()` enables the standard CDP `Runtime` domain (`Runtime.enable`), enterprise WAFs that probe V8 `Runtime.enable` console serialization side-channels (such as advanced Kasada or Akamai configurations) can still detect stock CDP sessions.

---

## Error Handling and Typing

### Typed Python Exceptions

Rust `DriverError` variants map to standard Python exception classes:

| Rust `DriverError` Variant | Python Exception | Typical Cause |
| :--- | :--- | :--- |
| `DriverError::Timeout` | `TimeoutError` | `wait_for`, `wait_for_response`, `wait_for_download`, CDP 60s RPC timeout, or browser launch timeout |
| `DriverError::InvalidInput` | `ValueError` | Unsupported `press_key` name, malformed cookies list, non-`<iframe>` passed to `tab.iframe()` |
| `BrowserClosed`, `Launch`, `Connection`, `Cdp`, `JsException`, `NotFound`, `StaleElement`, `Io` | `RuntimeError` | Closed browser, Chromium binary not found, JS exception, stale `backendNodeId`, file I/O failure |

```python
try:
    await tab.wait_for("#results", timeout_ms=5000)
except TimeoutError:
    print("Results did not appear within 5 seconds")
```

### PEP 561 Static Typing

The package includes `py.typed` and [`python/best_driver/__init__.pyi`](https://github.com/real-coder-xD/best-driver/blob/main/python/best_driver/__init__.pyi), exporting `Browser`, `Tab`, `Element`, `start`, and typed dictionary aliases (`ElementRect`, `NetworkResponse`, `FrameInfo`, `DownloadInfo`, `WaitUntil`, `MouseButton`, `ImageFormat`). Static type checkers (Mypy, Pyright, Pylance) pick up full signatures and hover docstrings automatically with no extra configuration.

---

## API Reference

### Top-Level Function

#### `best_driver.start(...)`

```python
async def start(
    headless: bool = False,
    user_data_dir: Optional[str] = None,
    browser_path: Optional[str] = None,
    extra_args: Optional[Sequence[str]] = None,
    proxy: Optional[str] = None,
) -> Browser
```

Launches Chromium on an OS-assigned loopback debugging port, connects a single browser-level CDP WebSocket, strips `HeadlessChrome` UA/Client Hints if running headless, and arms `Target.setAutoAttach` for popups and OOPIFs.

---

### `Browser`

| Member | Signature / Type | Description |
| :--- | :--- | :--- |
| `port` | `int` (property) | Ephemeral loopback TCP debugging port (`127.0.0.1:<port>`). |
| `user_data_dir` | `str` (property) | Active Chromium profile directory path. |
| `ws_url` | `str` (property) | Browser-level CDP WebSocket URL shared by all flattened sessions. |
| `is_alive()` | `async () -> bool` | Returns `True` if the Chromium child process is still running. |
| `main_tab()` | `async () -> Tab` | Returns the primary active page tab (creating one if none exist). |
| `new_tab()` | `async (url: str = "about:blank") -> Tab` | Opens a new tab via `Target.createTarget` with evasion armed before navigation. |
| `tabs()` | `async () -> list[Tab]` | Lists all open `page` targets via `Target.getTargets`. |
| `get()` | `async (url: str, new_tab: bool = False, wait_until: WaitUntil = "load") -> Tab` | Navigates the main tab (or a new tab if `new_tab=True`) to `url`. |
| `stop()` | `async () -> None` | Sends graceful `Browser.close` (up to 1.5s wait), then kills and reaps the child process. |
| Context manager | `async with await best_driver.start() as browser:` | Automatically calls `await browser.stop()` on block exit. |

---

### `Element`

| Member | Signature / Type | Description |
| :--- | :--- | :--- |
| `backend_node_id` | `int` (property) | Chromium internal `backendNodeId`. |
| `tag`, `text` | `str` (properties) | Lowercase HTML tag name and captured trimmed text. |
| `value`, `id`, `class_name` | `Optional[str]` (properties) | Captured control value, `id`, and SVG-safe `class` string. |
| `rect` | `ElementRect` (property) | Captured viewport rect (`x`, `y`, `width`, `height`). |
| `el[key]` / `get()` / `to_dict()` | `(key, default=None) -> Any` / `() -> dict` | Dictionary-compatible metadata access. |
| `click()` | `async (button: MouseButton = "left", click_count: int = 1) -> None` | Scrolls into view, moves via Cubic Bezier, and clicks this node. |
| `hover()` | `async () -> None` | Moves cursor to this node via Cubic Bezier. |
| `human_type()` | `async (text: str, clear: bool = False) -> None` | Focuses (optionally clears) and types with biological cadence. |
| `clear()` / `fill()` | `async () -> None` / `async (text: str) -> None` | Clears or fast-fills via `Input.insertText`. |
| `attr()` | `async (name: str) -> Optional[str]` | Reads a live attribute value from this node. |
| `inner_html()` / `outer_html()` | `async () -> Optional[str]` / `async () -> str` | Reads live inner or outer HTML (works inside Shadow DOM). |
| `scroll_into_view()` | `async () -> None` | Centers this node in the viewport. |
| `screenshot()` | `async (file_path: Optional[str] = None) -> bytes` | Captures PNG clipped to this node; saves to disk if `file_path` is given. |
| `ele()` / `eles()` | `async (query: str) -> Optional[Element]` / `list[Element]` | Queries descendants relative to this node (CSS, XPath, `text:`, `@attr`, Shadow DOM). |

---

### `Tab`

#### Identifiers, Iframes, and Downloads
| Method / Property | Signature | Description |
| :--- | :--- | :--- |
| `target_id`, `session_id` | `str` (properties) | Target ID and flattened CDP `sessionId`. |
| `frame_id` | `Optional[str]` (property) | Frame ID if scoped to an in-process iframe, else `None`. |
| `iframe()` | `async (selector: str, timeout_ms: int = 10000) -> Tab` | Returns a `Tab` scoped to the matched same-origin, nested, or OOPIF `<iframe>`. |
| `frames()` | `async () -> list[FrameInfo]` | Flattened list of frames (`id`, `parent_id`, `name`, `url`) from `Page.getFrameTree`. |
| `set_download_dir()` | `async (download_dir: str) -> str` | Configures download folder and returns its canonical absolute path. |
| `wait_for_download()` | `async (download_dir: str, timeout_ms: int = 30000) -> DownloadInfo` | Configures download dir and awaits completion of the next downloaded file. |

#### Navigation
| Method | Signature | Description |
| :--- | :--- | :--- |
| `get()` | `async (url: str, timeout_ms: Optional[int] = None, wait_until: WaitUntil = "load") -> None` | Navigates to `url` (`wait_until`: `"load"`, `"domcontentloaded"`, `"networkidle"`, `"none"`, `"commit"`). |
| `wait_for_load()` | `async (state: WaitUntil = "load", timeout_ms: int = 30000) -> None` | Waits for the specified document/network load state. |
| `reload()` | `async (ignore_cache: bool = False, timeout_ms: int = 30000) -> None` | Reloads the current page. |
| `back()` / `forward()` | `async (timeout_ms: int = 30000) -> bool` | Steps backward or forward in history (`False` at boundary). |
| `bring_to_front()` / `close()` | `async () -> None` | Activates or closes the tab via `Target.closeTarget`. |

#### Reading, Evaluation, and Smart Element Queries
| Method | Signature | Description |
| :--- | :--- | :--- |
| `evaluate()` | `async (expression: str) -> Any` | Evaluates JS (awaits Promises) and returns JSON-deserialized result. |
| `content()` / `title()` / `url()` / `current_url()` | `async () -> str` | Outer HTML, document title, or current `window.location.href`. |
| `ele()` / `eles()` | `async (query: str, timeout_ms: int = 10000) -> Optional[Element]` / `list[Element]` | Resolves one or all matching elements as native `Element` handles. |
| `exists()` / `count()` | `async (selector: str) -> bool` / `int` | Checks presence or counts all matches across DOM and Shadow Roots. |
| `text()` / `text_all()` | `async (selector: str) -> Optional[str]` / `list[str]` | Trimmed text of first or all matched elements. |
| `attr()` / `attr_all()` | `async (selector: str, name: str) -> Optional[str]` / `list[Optional[str]]` | Attribute value of first or all matched elements. |
| `value()` / `html()` | `async (selector: str) -> Optional[str]` | Form control `.value` or element `innerHTML`. |

#### Waiting
| Method | Signature | Description |
| :--- | :--- | :--- |
| `wait_for()` / `wait_for_selector()` | `async (selector: str, timeout_ms: int = 10000, visible: bool = False) -> None` | Waits for element presence (and optionally visibility). |
| `wait_for_hidden()` | `async (selector: str, timeout_ms: int = 10000) -> None` | Waits until element is detached or hidden. |
| `wait_for_text()` | `async (text: str, timeout_ms: int = 10000) -> None` | Waits until `document.body.innerText` contains `text`. |
| `wait_for_url()` | `async (contains: str, timeout_ms: int = 10000) -> str` | Waits until `location.href` contains substring and returns URL. |
| `wait_for_function()` | `async (expression: str, timeout_ms: int = 10000, poll_ms: int = 100) -> Any` | Polls JS expression until truthy and returns its value. |

#### Mouse, Keyboard, Forms, and Scrolling
| Method | Signature | Description |
| :--- | :--- | :--- |
| `click()` | `async (selector: str, button: MouseButton = "left", click_count: int = 1, timeout_ms: int = 10000) -> None` | Waits for visible element, moves via Cubic Bezier, and clicks. |
| `double_click()` / `right_click()` | `async (selector: str, timeout_ms: int = 10000) -> None` | Left double-click or single right-click. |
| `click_text()` | `async (text: str, exact: bool = False, timeout_ms: int = 10000) -> None` | Finds deepest visible element (including Shadow DOM) matching `text` and clicks. |
| `mouse_move()` / `mouse_click()` | `async (x: float, y: float, ...) -> None` | Coordinate-based Bezier move and click. |
| `hover()` / `drag()` | `async (selector, ...) -> None` | Hover over element or drag from `from_selector` to `to_selector`. |
| `swipe()` | `async (x1: float, y1: float, x2: float, y2: float) -> None` | Smooth S-curve drag gesture for sliders. |
| `human_type()` | `async (selector: str, text: str, clear: bool = False, timeout_ms: int = 10000) -> None` | Biological cadence typing with word and punctuation pauses. |
| `send_keys()` / `type_text()` | `async (..., delay_ms: int = 60, ...) -> None` | Character-by-character typing with jitter. |
| `fill()` / `focus()` / `clear()` | `async (selector: str, ...) -> None` | Fast paste fill, focus, or select-all + Backspace clear. |
| `press_key()` | `async (key: str) -> None` | Dispatches a named special key event pair. |
| `select_option()` / `check()` / `uncheck()` | `async (selector: str, ...) -> None` | Dropdown selection and checkbox/radio toggling. |
| `upload_file()` | `async (selector: str, files: Union[str, Sequence[str]], timeout_ms: int = 10000) -> None` | Sets local file path(s) on `<input type="file">`. |
| `scroll_down()` / `scroll_up()` | `async (pixels: float = 600.0) -> None` | Dispatches incremental wheel ticks. |
| `scroll_to_bottom()` | `async (step: float = 800.0, pause_ms: int = 600, max_rounds: int = 50) -> None` | Wheel-scrolls infinite feeds until height stops growing. |
| `scroll_to_top()` / `scroll_into_view()` | `async (...) -> None` | Scrolls to top of page or centers a target element. |

#### Network, Cookies, Storage, Emulation, Dialogs, Capture, and Raw CDP
| Method | Signature | Description |
| :--- | :--- | :--- |
| `wait_for_response()` | `async (url_pattern: str, timeout_ms: int = 30000) -> NetworkResponse` | Captures matching HTTP response headers and decoded body after transfer completes. |
| `set_extra_headers()` / `block_urls()` / `set_cache_disabled()` | `async (...) -> None` | Extra HTTP headers, URL wildcard blocking, and cache toggling. |
| `set_cookie()` / `cookies()` / `get_cookies()` / `set_cookies()` | `async (...) -> ...` | Single-cookie helper, bulk read/write cookie APIs. |
| `delete_cookies()` / `clear_cookies()` / `save_cookies()` / `load_cookies()` | `async (...) -> ...` | Delete, clear, and JSON file cookie persistence. |
| `storage_get()` / `storage_set()` / `storage_clear()` | `async (..., session: bool = False) -> ...` | `localStorage` and `sessionStorage` manipulation. |
| `set_viewport()` / `clear_viewport()` | `async (width, height, scale=1.0, mobile=False) -> None` | Device viewport metrics override and reset. |
| `set_user_agent()` / `set_timezone()` / `set_locale()` / `set_geolocation()` | `async (...) -> None` | Emulation overrides for UA, timezone, locale, and coordinates. |
| `auto_dialog()` / `handle_dialog()` | `(accept=True, prompt_text=None)` / `async (...)` | Synchronous auto-handler toggle or one-shot JS dialog response. |
| `screenshot()` | `async (file_path=None, full_page=False, format="png", quality=None, selector=None) -> bytes` | PNG/JPEG/WEBP capture of viewport, full document, or element clip. |
| `pdf()` | `async (file_path=None, landscape=False) -> bytes` | Vector PDF export with background graphics enabled. |
| `sleep()` | `async (seconds: float) -> None` | Non-blocking Tokio sleep. |
| `send()` | `async (method: str, params: Optional[dict[str, Any]] = None) -> Any` | Raw CDP command scoped to this tab's flattened `sessionId`. |

---

## Architecture Under the Hood

```text
+----------------------------------------------------------+
|                Python Application (PEP 561)              |
|        await browser.get("https://...")  + .pyi          |
+----------------------------+-----------------------------+
                             | PyO3 0.30 (direct serde Value FFI)
+----------------------------v-----------------------------+
|               Rust Core (best_driver.cdylib)             |
|  +----------------------------------------------------+  |
|  |             Tokio Asynchronous Runtime             |  |
|  |  - Single Browser-Level WebSocket (Zero reqwest)   |  |
|  |  - Flattened SessionId Multiplexer (Target API)    |  |
|  |  - OnceCell Auto-Attach Stealth (Popups & OOPIFs)  |  |
|  |  - Native CDP backendNodeId Element Handles        |  |
|  |  - Cubic Bezier Mouse & Biological Typing Engine   |  |
|  +-------------------------+--------------------------+  |
+----------------------------+-----------------------------+
                             | Single CDP WebSocket (flatten: true)
+----------------------------v-----------------------------+
|                     Chromium Engine                      |
|     (Google Chrome / Brave / Edge / Chromium)            |
|     - Target.attachToTarget + Target.setAutoAttach       |
|     - Page.addScriptToEvaluateOnNewDocument (Evasion)    |
|     - Input / Network / Fetch / DOM / Emulation          |
+----------------------------------------------------------+
```

### Rust Source Layout

| File | Responsibility |
| :--- | :--- |
| `src/cdp.rs` | Single browser-level `CdpConnection` WebSocket multiplexer, lightweight `CdpSession` per `sessionId`, and session-filtered `SessionEventRx`. |
| `src/browser.rs` | Chromium process launcher, pure-Tokio `TcpStream` `/json/version` discovery (`Content-Length` framed), headless UA/Client Hints builder, graceful shutdown + RAII temp profile cleanup. |
| `src/tab.rs` | Per-tab and per-iframe controller: navigation, waiting, native `backendNodeId` element ops, iframe scoping, download manager, network sniffer, proxy auth, cookies, storage, and capture. |
| `src/locator.rs` | Smart Locator JS codegen (`sel_expr`, `sel_all_expr`, `child_sel_fn`, `child_sel_all_fn`) with attribute escaping and Shadow DOM traversal. |
| `src/input.rs` | Cubic Bezier cursor trajectory math, S-curve swipe generator, nanosecond jitter, and special keyboard table. |
| `src/error.rs` | Typed `DriverError` enum mapped to Python `TimeoutError`, `ValueError`, and `RuntimeError`. |
| `src/evasion.js` | Prototype-only anti-detection script injected via `Page.addScriptToEvaluateOnNewDocument`. |
| `src/lib.rs` | `BrowserCore` auto-attach supervisor (`OnceCell` race-free arming) and PyO3 bindings (`Browser`, `Tab`, `Element`, `start`). |
| `python/best_driver/` | Mixed package shim (`__init__.py`), complete PEP 561 stub (`__init__.pyi`), and `py.typed` marker. |

---

## Migration Guide

### From Selenium

```python
# Selenium
# driver.get("https://example.com")
# box = WebDriverWait(driver, 10).until(EC.presence_of_element_located((By.NAME, "q")))
# box.send_keys("rust cdp")
# driver.find_element(By.XPATH, "//button[contains(text(),'Search')]").click()

# best-driver
tab = await browser.get("https://example.com")
await tab.human_type("@name=q", "rust cdp")
await tab.click("text:Search")
```

### From Playwright

```python
# Playwright
# async with async_playwright() as p:
#     browser = await p.chromium.launch(headless=True)
#     page = await browser.new_page()
#     await page.goto("https://example.com")
#     await page.frame_locator("#frame").locator("button").click()

# best-driver
async with await best_driver.start(headless=True) as browser:
    tab = await browser.get("https://example.com")
    frame = await tab.iframe("#frame")
    await frame.click("button")
```

### From DrissionPage

```python
# DrissionPage (sync Python)
# page = ChromiumPage()
# page.get("https://example.com")
# card = page.ele(".product-card")
# title = card.ele(".title").text

# best-driver (GIL-free async Rust)
async with await best_driver.start(headless=True) as browser:
    tab = await browser.get("https://example.com")
    card = await tab.ele(".product-card")
    if card:
        title_el = await card.ele(".title")
        title = title_el.text if title_el else ""
```

---

## Frequently Asked Questions

### What is the fastest undetected alternative to Selenium and Playwright in Python?
`best-driver` runs its entire CDP WebSocket multiplexer, JSON-RPC 2.0 parser, and Base64 decoder natively in Rust + Tokio outside the Python GIL. Unlike Selenium (which sends HTTP requests through `chromedriver`) and Playwright (which runs a separate Node.js server over IPC), `best-driver` uses a single flattened browser-level WebSocket and passes **57/57 Sannysoft checks in headless mode** out of the box.

### Does `best-driver` leak `HeadlessChrome` in headless mode?
No. On startup, `best-driver` reads `Browser.getVersion` and automatically applies `Emulation.setUserAgentOverride` with matching `userAgent` and `userAgentMetadata` (`Sec-CH-UA` brands and full version list), preserving the real installed Chromium version and host OS platform while removing `HeadlessChrome`.

### Does `best-driver` mutate the DOM when calling `ele()` or `eles()`?
No. `tab.ele()` and `tab.eles()` bind directly to Chromium's internal `backendNodeId` via `DOM.describeNode` and operate through `DOM.resolveNode` + `Runtime.callFunctionOn`, releasing every V8 `objectId` on all paths. `MutationObserver` detectors observe zero attribute mutations.

### How does `best-driver` handle Shadow DOM, cross-origin iframes, downloads, and AJAX JSON responses?
Open `#shadow-root` trees are pierced automatically by all Smart Locator queries; `tab.iframe(selector)` returns a full `Tab` controller for same-origin, nested, `srcdoc`, and cross-origin OOPIF iframes with automatic coordinate translation; `tab.wait_for_download(dir)` captures downloaded files with full metadata; and `tab.wait_for_response(pattern)` waits for `Network.loadingFinished` and returns headers plus the UTF-8 decoded response body.

### Does `best-driver` require `chromedriver` or Node.js?
No. It requires only Python `>= 3.8` and any locally installed Chromium-based browser (Chrome, Brave, Edge, or Chromium).

---

## Changelog and License

- Release history: [CHANGELOG.md](https://github.com/real-coder-xD/best-driver/blob/main/CHANGELOG.md)
- License: MIT — see [LICENSE](https://github.com/real-coder-xD/best-driver/blob/main/LICENSE)
