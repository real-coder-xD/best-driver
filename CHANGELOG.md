# Changelog

All notable changes to `best-driver` are documented in this file.

## [0.3.0] - 2026-11-21

### Fixed
- **`BrowserContext` Cookie Scoping (`Storage.getCookies` / `setCookies` / `clearCookies`)**: `Tab` now retains its owning `browser_context_id` and passes `browserContextId` to CDP `Storage.*` cookie commands, fixing the `0`-cookie bug when calling `tab.cookies()`, `tab.storage_state()`, `tab.tls_session()`, or `tab.clear_cookies()` inside an isolated `BrowserContext`.

### Added
- **Response-Stage Interception (`tab.route(..., stage="response")`) & `route.fetch()`**: Intercept and modify live server responses (`route.response_body()`, `response_text()`, `response_json()`, `continue_response()`, `fulfill()`) before page JavaScript receives them.
- **Native In-Browser `tab.fetch()`**: Executes `fetch()` directly through Chromium's real TLS stack, HTTP/2, and cookie jar (supporting `method`, `headers`, `body`, `json`, `credentials`) for ultra-strict JA4-pinned endpoints.
- **HAR 1.2 Recording (`tab.record_har()`, `tab.export_har()`, `tab.stop_har()`)**: Captures full HAR 1.2 network traces with optional response bodies directly from `Tab`.
- **Per-Context Emulation & Init Scripts**: Extended `browser.new_context()` with `user_agent`, `accept_language`, `platform`, `viewport`, `locale`, `timezone`, and `extra_headers`, plus `tab.add_init_script()` and `ctx.add_init_script()` (`Page.addScriptToEvaluateOnNewDocument`) and `tab.context_id`.
- **Playwright-Grade `Locator` & `Element` Forms**: Added `loc.filter(has_text=..., has_not_text=...)`, `loc.wait_for(state="visible"|"attached"|"hidden"|"detached")`, and Shadow-DOM-safe `is_visible()`, `is_enabled()`, `is_checked()`, `check()`, `uncheck()`, `select_option()`, and `press_key()`.
- **Stealth & Dev Polish**: Auto-applies `--force-webrtc-ip-handling-policy=disable_non_proxied_udp` when a proxy is configured to prevent WebRTC UDP IP leaks; cleared `worldName` (`""`) in `Page.createIsolatedWorld`; added `[profile.dev] debug = 0` to keep local build caches compact.

## [0.2.0] - 2026-11-21

### Network Control, Isolated Concurrency & Auto-Retrying Locators
- **Isolated `BrowserContext` (`browser.new_context(proxy=None, proxy_bypass_list=None)`)**: Creates fully isolated incognito browser contexts via CDP `Target.createBrowserContext` inside a single Chromium process and single WebSocket — with independent cookies, `localStorage`, and optional per-context authenticated proxies (`ctx_tab.tls_session()` automatically inherits the context's proxy). Supports `async with` automatic disposal via `Target.disposeBrowserContext`.
- **Full Request Routing (`tab.route()`, `tab.unroute()`)**: Intercepts requests matching glob or substring patterns via CDP `Fetch`, supporting both sync and `async def` handlers with `route.fulfill(status, body, json, headers)`, `route.continue_(url, method, headers, post_data)`, and `route.abort(error_reason)` while preserving proxy auth (`Fetch.authRequired`) and auto-continuing unhandled pauses.
- **Continuous Async Network Streams (`tab.listen_requests()`, `tab.listen_responses()`)**: Backed by a Rust Tokio-filtered `EventStream`, yields `StreamedRequest` and `StreamedResponse` (with lazy `await resp.body()` / `text()` / `json()`) via `async with` / `async for` / `await stream.next(timeout_ms)`.
- **Lazy Auto-Retrying `Locator` (`tab.locator(query)`)**: Playwright-style chained Smart Locators (`locator()`, `.first`, `.last`, `.nth(i)`) that re-resolve fresh on every interaction (including across open and closed `#shadow-root` trees) and automatically retry transient `StaleElement` errors on re-rendering React/Vue/Next.js SPAs.
- **Full PEP 561 Stub Parity & Verification**: Added complete `.pyi` stubs across `Browser`, `BrowserContext`, `Tab`, `Element`, `EventStream`, `Locator`, `Route`, `RequestStream`, `ResponseStream`, `StreamedRequest`, and `StreamedResponse`, plus end-to-end verification in `examples/test_v020.py`.

## [0.1.8] - 2026-11-21

### Static Typing & IDE Autocomplete (PEP 561)
- **Typed `tls_session()` Overloads**: Added `@overload` annotations in `python/best_driver/__init__.pyi` and `python/best_driver/tls.py` binding `async_mode=True` to `http_tls.AsyncSession` and `async_mode=False` to `http_tls.Session`, enabling full IDE autocomplete on both `tls.*` and `resp.*` (`status_code`, `text`, `json`, `headers`, `cookies`, `http_version`, `challenge`, etc.).
- **Strict `TypedDict` & `Literal` Return Types**: Added `Cookie`, `ElementDict`, and `SpecialKey` types; upgraded `Tab.cookies()`, `Tab.get_cookies()`, `StorageState.cookies`, `Element.to_dict()`, `Tab.press_key()`, and `@overload` typing on `Tab.storage_get()` (`key=None -> dict[str, str]`, `key: str -> Optional[str]`).

## [0.1.7] - 2026-11-21

### Documentation
- Restructured `README.md` for 30-second developer onboarding: moved `Quickstart` (core browser + 1-line `http_tls` handoff) to the top, promoted `Synchronized http_tls Session Handoff` to a dedicated top-level section & TOC entry with a 6-layer parity table, `asyncio.gather` concurrent burst example, `storage_state.json` save/reload guide, 4-method cheat sheet, and Chrome 131–149 `JA4` pinning note.

## [0.1.6] - 2026-11-21

### Synchronized `http_tls` Fingerprint & Session Handoff (`best-driver[tls]`)
- **One-Line Session Bridge (`tab.tls_session()`, `tab.sync_from_tls()`)**: Added optional extra `pip install "best-driver[tls]"` (`http-tls>=0.6.2`) and `python/best_driver/tls.py` to hand off active Chromium sessions to `http_tls.AsyncSession` / `Session` while keeping outbound proxy/IP family, TLS/HTTP2 preset, `User-Agent`, `Sec-CH-UA*`, 13-header HTTP/2 wire order, and domain-scoped `HttpOnly` cookies aligned.
- **Automatic Dual-Stack IPv6/IPv4 Parity**: `Tab` now records the active document's `remoteIPAddress` from CDP `Network.responseReceived` and derives `local_address` (`"::"` for IPv6, `"0.0.0.0"` for IPv4 when no proxy is configured) so `http_tls` exits from the exact same IP as Chromium on dual-stack networks.
- **Default TLS/HTTP2 Alignment (`--disable-quic` + `TLSTrustAnchorIDs`)**: Enabled `--disable-quic` and `TLSTrustAnchorIDs` in Chromium launch flags (with safe `--disable-features=` merging for `extra_args`) and aligned `Sec-CH-UA` GREASE brand order so Chromium and `http_tls` match on `HTTP/2`, all 15 TLS cipher suites (`8daaf6152771`), all 16 standard TLS extensions, Akamai HTTP/2 (`1:65536;2:0;4:6291456;6:262144|15663105|0|m,a,s,p`), and 13/13 HTTP/2 wire header order out of the box.
- **New Public APIs & PEP 561 Stubs**: Exposed `browser.proxy`, `tab.proxy`, `await tab.fingerprint()`, `await tab.storage_state(path=None)`, `await tab.tls_session(async_mode=True, **kwargs)`, `await tab.sync_from_tls(session, default_url=None)`, `BrowserFingerprint`, and `StorageState` with 100% `.pyi` parity.
- **Verification Scripts**: Added `examples/test_tls_sync.py` and `examples/test_real_handoff.py` (including `--multi-cf` 6-origin Cloudflare edge suite and `--url`/`--proxy`/`--headful` custom CLI testing).

## [0.1.5] - 2026-11-21

### Stealth & Closed Shadow DOM
- **Eliminated `Runtime.enable` CDP Leak**: Removed `Runtime.enable` from `Tab::new()` while keeping `Runtime.evaluate`, `Runtime.callFunctionOn`, `Runtime.getProperties`, `Runtime.releaseObject`, and `Page.createIsolatedWorld` fully operational. Verified against Cloudflare/DataDome `console.debug(new Error())` `Error.stack` getter and object-property getter traps (both `False`).
- **Native CDP Closed Shadow DOM Piercing (`mode: 'closed'`)**: Added pure-Rust `find_pierced_backend_node_ids()` tree walker in `src/locator.rs` (supporting `#id`, `.class`, `tag`, `[attr]`, `[attr=val]`, `[attr*=val]`, `[attr^=val]`, `[attr$=val]`, `@attr=val`, descendant chains, and `text:`/`t:`) backed by automatic `DOM.getDocument(pierce=True)` / `DOM.describeNode(pierce=True)` fallback across `ele`, `eles`, `ele_child`, `eles_child`, `click`, `exists`, `count`, `text`, and `iframe` (including iframes nested inside closed shadow roots).
- **Expanded Verification Suite**: Added unit test `pierced_matcher_finds_nodes_inside_closed_shadow_root` (10/10 Rust unit tests) and TEST SUITE 6 in `examples/test_v013.py` covering `Runtime.enable` traps, closed shadow checkbox clicking, and closed-shadow-hosted iframe traversal.

### Documentation
- Refined `README.md` comparison table to focus on anti-bot / Cloudflare-bypass tools (`best-driver`, `Patchright`, `nodriver`, `DrissionPage`) in a compact 5-column PyPI-safe layout with minimal typographic icons (`◈`, `▸`, `✓`, `◐`, `✗`), centered hero header, and Unicode box-drawing architecture diagram.

## [0.1.4] - 2026-11-21

### Documentation
- Rewrote `README.md` in clean, detailed technical English with zero emoji spam, full API Reference matching all 117 PEP 561 `.pyi` symbols, Smart Locator syntax table, native `backendNodeId` / iframe / download guides, migration snippets (Selenium, Playwright, DrissionPage), measured concurrency throughput, and SEO/GEO FAQ.

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
