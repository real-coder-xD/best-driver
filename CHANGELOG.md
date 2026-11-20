# Changelog

All notable changes to `best-driver` are documented in this file.

## [0.1.3] - 2026-11-20

### Architecture & Performance
- **Flattened Target Session Multiplexing (`Target.attachToTarget`, `flatten: true`)**: All tabs, popups, and OOPIF iframes now share a single browser-level CDP WebSocket (`CdpConnection`) routed by `sessionId` with isolated per-session event streams.
- **Removed `reqwest` / HTTP Client Dependencies**: Eliminated >35 transitive crates by switching target discovery/creation to `Target.getTargets` / `Target.createTarget` and loopback `/json/version` discovery to a pure-Tokio `TcpStream` reader with `Content-Length` framing.
- **Modular Codebase & Typed Errors**: Split monolithic controller into `error.rs`, `cdp.rs`, `browser.rs`, `locator.rs`, `input.rs`, `tab.rs`, and `lib.rs`. Mapped `DriverError` natively to Python `TimeoutError`, `ValueError`, and `RuntimeError`.
- **Direct FFI Serialization**: Refactored `py_to_value` to convert Python primitives (`dict`, `list`, `int`, `float`, `bool`, `str`, `None`) directly to `serde_json::Value` without `json.dumps` roundtrips.
- **Graceful Shutdown**: `Browser.stop()` now sends `Browser.close` and waits briefly so persistent `user_data_dir` profiles flush SQLite/Cookies cleanly before termination; added `Browser.is_alive()`.

### Stealth & Anti-Detection (57/57 Sannysoft Headless — 0 Fail, 0 Warn)
- **Headless UA & Client Hints Alignment**: Automatically strips `HeadlessChrome/` from HTTP `User-Agent`, `navigator.userAgent`, and `Sec-CH-UA` / `navigator.userAgentData` metadata via `Emulation.setUserAgentOverride`.
- **Auto-Attach Pre-Execution Stealth**: `Target.setAutoAttach` + race-free `OnceCell` injects evasion scripts and UA overrides into popups (`window.open` / `target=_blank`) and cross-origin OOPIF iframes before their first JS instruction runs.
- **Zero-DOM-Mutation Element Handles**: `tab.ele()` and `tab.eles()` bind directly to Chromium's native `backendNodeId` (`DOM.describeNode` / `DOM.resolveNode`) with guaranteed `Runtime.releaseObject` cleanup — leaving zero attributes for `MutationObserver` probes.
- **Prototype-Only Patching**: Zero own properties leaked on `navigator` (`Object.getOwnPropertyNames(navigator) == []`); all patched methods/getters are non-constructable concise methods returning `[native code]`.
- **Safe Canvas & Audio Micro-Noise**: Offscreen-cloned canvas perturbation that preserves transparent probe pixels (`TRANSPARENT_PIXEL` passes cleanly) plus once-per-buffer AudioContext noise and WebGL RTX 4070 spoofing.
- **Biological Input Physics**: Cubic Bézier mouse trajectories with ease-in-out acceleration and micro-tremors, `tab.human_type()` biological cadence, and `tab.swipe()` S-curve drag gestures.

### Developer Experience & Features
- **DrissionPage-Style Smart Locators & Shadow DOM**: Unified resolution for CSS, XPath (`//`), visible text (`text:` / `t:`), and attributes (`@attr=val`) with automatic open `#shadow-root` traversal across `ele`, `eles`, `click`, `count`, `text_all`, `attr_all`, `click_text`, and `wait_for`.
- **Object-Oriented `Element` Chaining**: `el.click()`, `el.hover()`, `el.human_type()`, `el.attr()`, `el.inner_html()`, `el.outer_html()`, `el.screenshot()`, `el.ele()`, `el.eles()`, plus backward-compatible dict subscripting (`el["tag"]`, `el.get()`).
- **Iframe Controller (`tab.iframe()`, `tab.frames()`)**: Returns a full `Tab`-compatible controller scoped to same-origin, `srcdoc`, nested, or cross-origin OOPIF iframes with automatic viewport coordinate translation.
- **Download Manager (`tab.set_download_dir()`, `tab.wait_for_download()`)**: Configures Chromium download behavior and awaits completion with `{guid, url, filename, path, total_bytes, state}`.
- **Network Response Sniffer (`tab.wait_for_response()`)**: Waits for `Network.responseReceived` + `Network.loadingFinished` and extracts headers + UTF-8 decoded body.
- **PEP 561 Type Stubs**: Shipped complete `best_driver/__init__.pyi` (100% parity with runtime) + `py.typed` marker for full IDE autocomplete and Mypy/Pyright support.
- **CI Quality Gate**: Added `cargo clippy --all-targets -- -D warnings` and `cargo test --lib` (9 unit tests) before wheel releases.

## [0.1.1] - 2026-11-19
- Added full `README.md` project description and repository URLs to PyPI package metadata.
- Switched GitHub Actions release pipeline to PyPI Trusted Publishing (OIDC).

## [0.1.0] - 2026-11-19
- Initial pure-Rust PyO3 + Tokio Chromium CDP driver release with 14-platform wheels.
