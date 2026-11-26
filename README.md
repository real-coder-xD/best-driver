<div align="center">

<img src="https://raw.githubusercontent.com/real-coder-xD/best-driver/main/docs/assets/banner.jpg" alt="best-driver — Rust PyO3 + Tokio Undetected Chromium CDP Engine for Python" width="100%" />

# `best-driver`

**Undetected Rust-Powered Chromium Automation for Python**

*High-performance, GIL-free browser automation and web scraping built in pure Rust (`PyO3` + `Tokio`) over a single flattened Chrome DevTools Protocol (CDP) WebSocket.*

[![PyPI version](https://img.shields.io/pypi/v/best-driver?style=flat-square&color=blue)](https://pypi.org/project/best-driver/)
[![Python](https://img.shields.io/badge/python-3.8%2B-blue.svg?style=flat-square)](https://www.python.org/)
[![Typing](https://img.shields.io/badge/typing-PEP%20561%20typed-007acc.svg?style=flat-square)](https://peps.python.org/pep-0561/)
[![Rust](https://img.shields.io/badge/rust-2021%20%7C%20PyO3%200.30%20%7C%20Tokio-orange.svg?style=flat-square)](https://www.rust-lang.org/)

[![Sannysoft Headless](https://img.shields.io/badge/sannysoft%20headless-57%2F57%20passed-success.svg?style=flat-square)](https://bot.sannysoft.com)
[![Wheels](https://img.shields.io/badge/wheels-14%20platforms-brightgreen.svg?style=flat-square)](https://pypi.org/project/best-driver/#files)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg?style=flat-square)](https://github.com/real-coder-xD/best-driver/blob/main/LICENSE)

[Quickstart](#quickstart) · [http_tls Handoff](#synchronized-http_tls-session-handoff) · [Overview](#overview) · [Comparison](#best-driver-vs-patchright-nodriver-and-drissionpage) · [Installation](#installation) · [Locators & Shadow DOM](#smart-locators-and-shadow-dom) · [Elements](#element-handles) · [Iframes](#working-with-iframes)<br>
[Network & Downloads](#network-interception-and-downloads) · [Sessions & Lifecycle](#cookies-storage-profiles-and-proxies) · [Popups & Events](#popups-events-and-utilities) · [Input Physics](#biological-input-physics-and-forms) · [Stealth](#stealth-and-anti-detection-architecture) · [Errors & Typing](#error-handling-and-typing) · [API Reference](#api-reference) · [Architecture](#architecture-under-the-hood) · [FAQ](#frequently-asked-questions)

</div>

---

## Quickstart

### 1. Install

```bash
# Core browser automation engine (14 pre-built wheel targets, zero external runtime)
pip install best-driver

# Optional: include the synchronized Rust BoringSSL HTTP client bridge (http-tls)
pip install "best-driver[tls]"
```

### 2. Run Your First Undetected Browser

```python
import asyncio
import best_driver


async def main() -> None:
    async with await best_driver.start(headless=True) as browser:
        tab = await browser.get("https://bot.sannysoft.com", wait_until="networkidle")

        print("Title:", await tab.title())
        print("navigator.webdriver:", await tab.evaluate("navigator.webdriver"))  # False

        # Smart Locator (CSS, XPath //, text:, @attr=val — pierces open & closed Shadow DOM)
        heading = await tab.ele("h1")
        if heading is not None:
            print("Heading:", heading.tag, heading.text)

        await tab.screenshot("sannysoft.png", full_page=True)


if __name__ == "__main__":
    asyncio.run(main())
```

### 3. Hand Off the Live Session to `http_tls` (1 Line)

Open a page once in real Chromium, then switch to `http_tls` (`~200-346 req/s`) sharing the exact same proxy/IP family, `HTTP/2`, TLS ciphers/extensions, `User-Agent`, `Sec-CH-UA*`, 13-header wire order, and `HttpOnly` cookies:

```python
import asyncio
import best_driver


async def main() -> None:
    async with await best_driver.start(headless=True) as browser:
        tab = await browser.get("https://www.cloudflare.com/cdn-cgi/trace", wait_until="load")

        # 1 line: sync proxy/IPv6-IPv4, TLS/H2 preset, headers, header order & HttpOnly cookies
        async with await tab.tls_session() as tls:
            resp = await tls.get("https://www.cloudflare.com/cdn-cgi/trace")
            print(resp.status_code, resp.http_version)


if __name__ == "__main__":
    asyncio.run(main())
```

---

## Synchronized `http_tls` Session Handoff

Use `best-driver` for JavaScript rendering and browser session setup, then call `await tab.tls_session()` to run high-throughput API requests through `http_tls` (Rust BoringSSL `wreq`) without paying Chromium memory overhead on every request.

### What Gets Synchronized Automatically

| Layer | How `best-driver` Syncs It to `http_tls` |
| :--- | :--- |
| `▸` **Outbound IP / Proxy** | Inherits `browser.proxy` verbatim; when no proxy is set, inspects CDP `remoteIPAddress` and auto-binds `local_address="::"` (IPv6) or `"0.0.0.0"` (IPv4) so dual-stack hosts see the exact same IP. |
| `▸` **Transport Protocol** | Launches Chromium with `--disable-quic` by default so both Chromium and `http_tls` negotiate `HTTP/2` over TCP. |
| `▸` **TLS Ciphers & Extensions** | Auto-maps live browser version & OS (`chrome_157`, `chrome_149_macos`, `edge_148`) and disables `TLSTrustAnchorIDs` (`0xca34`) by default so all 15 ciphers (`8daaf6152771`) and 16 standard TLS extensions align. |
| `▸` **Akamai HTTP/2 & Header Order** | Enforces Chromium's exact `13`-header HTTP/2 wire sequence (`sec-ch-ua` ... `accept-encoding`, `accept-language`, `priority`). |
| `▸` **`User-Agent` & Client Hints** | Copies live `User-Agent`, `Sec-CH-UA`, `Sec-CH-UA-Mobile`, `Sec-CH-UA-Platform`, and `Accept-Language` verbatim. |
| `▸` **Cookies (Including `HttpOnly`)** | Injects all CDP cookies directly into `http_tls`'s domain-scoped Rust cookie jar (zero cross-domain cookie leakage), and pushes refreshed or host-only cookies back into Chromium via `await tab.sync_from_tls(tls)`. |

> **Tip for 100% Exact `JA4` Parity:** Modern Chrome 156+ hardcodes post-quantum `mldsa44/65/87` signature algorithms in `JA4_R`. If a strict endpoint pins the exact 12-char `JA4` suffix per session, point `browser_path` at a pinned Chrome for Testing `131-149` binary so `JA4` matches `http_tls` (`t13d1516h2_8daaf6152771_d8a2da3f94cd`) byte-for-byte.

### Concurrent Burst + Bi-Directional Cookie Sync

```python
import asyncio
import best_driver


async def main() -> None:
    async with await best_driver.start(
        headless=True,
        # proxy="http://user:pass@proxy.example.com:8080",
    ) as browser:
        tab = await browser.get("https://httpbin.org/cookies", wait_until="load")

        async with await tab.tls_session(async_mode=True) as tls:
            # Fire concurrent requests over the synced Rust BoringSSL session
            resps = await asyncio.gather(
                tls.get("https://httpbin.org/cookies"),
                tls.get("https://httpbin.org/cookies/set?refreshed=ok_2026"),
            )
            print("Statuses:", [r.status_code for r in resps])

            # Push newly issued cookies back into the live Chromium tab
            synced = await tab.sync_from_tls(tls)
            print("Synced back to Chromium:", synced)


if __name__ == "__main__":
    asyncio.run(main())
```

### Save Session to Disk and Reuse Without Reopening Chrome

```python
import asyncio
import best_driver
import http_tls


async def export_state() -> str:
    async with await best_driver.start(headless=True) as browser:
        tab = await browser.get("https://httpbin.org/cookies/set?token=abc123", wait_until="load")
        state = await tab.storage_state("session_state.json")
        return state["impersonate"]


preset = asyncio.run(export_state())

# Later: run directly in http_tls (sync or async) with zero browser RAM
with http_tls.Session(storage_state="session_state.json", impersonate=preset) as s:
    print(s.get("https://httpbin.org/cookies").json())
```

### Bridge Methods Cheat Sheet

| Method | Purpose |
| :--- | :--- |
| `await tab.fingerprint()` | Returns live `impersonate`, `user_agent`, `sec_ch_ua*`, `accept_language`, `headers`, `header_order`, `proxy`, `remote_ip`, `local_address`. |
| `await tab.storage_state(path=None)` | Exports all CDP cookies (including `HttpOnly`) + live browser headers in `http_tls` / Playwright JSON format. |
| `await tab.tls_session(async_mode=True, **kwargs)` | Returns an `http_tls.AsyncSession` (or sync `Session`) pre-configured with all 6 synchronized layers. |
| `await tab.sync_from_tls(tls, default_url=None)` | Pushes refreshed or host-only cookies from `http_tls` back into the live Chromium tab. |

---

## Overview

`best-driver` is an open-source, driverless Chromium automation engine for Python built as a native Rust extension (`cdylib`). It is designed as a fast, low-memory, stealth-first alternative to Playwright, Patchright, DrissionPage, nodriver, and Selenium.

Instead of wrapping `chromedriver` over HTTP or delegating commands to a Node.js subprocess, `best-driver` launches Chrome, Chromium, Brave, or Microsoft Edge directly and multiplexes all tabs, popups, and out-of-process iframes (OOPIFs) over a **single browser-level CDP WebSocket** using flattened sessions (`Target.attachToTarget` with `flatten: true`).

### Core Design Highlights

- `◈` **Single Flattened WebSocket Multiplexer**: One persistent Tokio WebSocket serves all tabs, popups, and OOPIFs via `sessionId` routing with zero HTTP client crates (`reqwest` and TLS are not in the dependency tree).
- `◈` **GIL-Free Native Async I/O**: JSON-RPC 2.0 framing, WebSocket I/O, Base64 decoding, concurrent `DOM.describeNode` fan-out (`join_all`), and cursor trajectory math run natively in Rust on Tokio outside the Python GIL.
- `◈` **Verified Stealth — Zero `Runtime.enable` Leak & 57/57 Sannysoft Headless**: Omits `Runtime.enable` so Cloudflare/DataDome `console.debug` traps return `False`, strips `HeadlessChrome` from `User-Agent` and `Sec-CH-UA`, patches strictly on DOM prototypes (`Object.getOwnPropertyNames(navigator) == []`), isolates canvas noise via offscreen cloning, and auto-arms popups and OOPIFs before their first JavaScript statement runs.
- `◈` **Zero-DOM-Mutation Element Handles**: `tab.ele()` and `tab.eles()` bind directly to Chromium's internal `backendNodeId`, leaving zero synthetic attributes for `MutationObserver` detectors.
- `◈` **Unified Smart Locators, Open & Closed Shadow DOM, and Iframes**: Resolves CSS, XPath (`//`), visible text (`text:` / `t:`), and attribute shorthands (`@name=q`) across both `mode: 'open'` and `mode: 'closed'` `#shadow-root` trees, plus same-origin, nested, `srcdoc`, closed-shadow-hosted, and OOPIF iframes.
- `◈` **Synchronized `http_tls` Handoff**: Built-in 1-line bridge (`tab.tls_session()`) to move seamlessly between real Chromium and Rust BoringSSL HTTP requests.
- `◈` **100% PEP 561 Typed**: Ships `best_driver/__init__.pyi` (100% parity with the compiled runtime) and `py.typed` for complete IDE autocomplete and static checking.

---

## best-driver vs Patchright, nodriver, and DrissionPage

Comparison against Python browser automation tools commonly used for anti-bot and Cloudflare workflows:

| Capability | best-driver | Patchright | nodriver | DrissionPage |
| :--- | :--- | :--- | :--- | :--- |
| Core runtime | Rust + Tokio (`cdylib`) | Node.js + Python | Python (`asyncio`) | Python (sync) |
| Transport | 1 flattened CDP WS | Flattened CDP (IPC) | Per-tab CDP WS | Per-tab CDP + HTTP |
| External runtime | `✓` None | `✗` Patched Node | `✓` None | `✓` None |
| GIL on I/O | `✓` Free (Tokio) | `◐` Cross-process IPC | `✗` GIL-bound | `✗` GIL-bound |
| Stealth strategy | No `Runtime.enable` + prototype JS + UA/CH + Auto-Attach + Bezier | Patched `Runtime.enable` + isolated worlds | Driverless CDP flags | Driverless CDP flags |
| Smart locators | `✓` Unified (`//`, `text:`, `@attr`, CSS) | `◐` Prefixed (`text=`, `xpath=`) | `◐` Basic CSS / text | `✓` Unified (`//`, `text:`, `@attr`, CSS) |
| Auto Shadow DOM | `✓` Open + Closed (CDP) | `◐` Open (CSS) | `✗` No | `◐` Open (JS) |
| Element handles | `✓` `backendNodeId` + lazy `locator()` | `✓` Locator handles | `◐` Raw CDP nodes | `◐` Node / locator refs |
| Isolated contexts & routing | `✓` `new_context()` + `route()` + streams | `✓` Built-in | `✗` Manual CDP | `◐` `listen` only |
| Popup & OOPIF stealth | `✓` Auto (`Target.setAutoAttach`) | `✓` Patched driver | `✗` Manual | `✗` Manual |
| Synced TLS HTTP handoff | `✓` Native `tab.tls_session()` (`http_tls`) | `✗` Manual | `✗` Manual | `◐` `SessionPage` (OpenSSL) |
| Iframe / Download / AJAX | `✓` Built-in | `✓` Built-in | `✗` Manual CDP | `✓` Built-in |
| Input physics | `✓` Cubic Bezier + `human_type` + `swipe` | `✗` Synthetic / instant | `✗` Linear / instant | `✗` Linear / instant |
| PEP 561 typing | `✓` Full (`py.typed`) | `✓` Full | `◐` Partial | `◐` Partial |

> Legend: `✓` Native / full support · `◐` Partial / indirect · `✗` Not built-in

### Measured Local Throughput (Windows 11, Headless Chrome)

| Benchmark | Measured Latency |
| :--- | ---: |
| `▸` Browser launch + flattened browser WebSocket ready | **~680 ms** |
| `▸` Opening **20 concurrent tabs** (`asyncio.gather`, 1 shared WS) | **~295 ms** |
| `▸` Executing **20 concurrent `tab.evaluate()` calls** across 20 tabs | **~5.4 ms** |
| `▸` 10 concurrent Cloudflare edge requests via `tab.tls_session()` | **45.8 ms (218 req/s)** |

---

## Installation

### Requirements

- Python `>= 3.8` (CPython 3.8-3.14 and PyPy supported).
- A locally installed Chromium-based browser: Google Chrome, Chromium, Brave, or Microsoft Edge.

### Pre-Built Wheels (14 Platforms)

Pre-built wheels are published to PyPI for **14 targets** (Windows x64/x86/ARM64, Linux manylinux x86_64/x86/aarch64/armv7, Musllinux x86_64/x86/aarch64/armv7, macOS Intel/Apple Silicon) plus source distribution:

```bash
pip install best-driver

# With synchronized Rust BoringSSL HTTP client (http-tls)
pip install "best-driver[tls]"
```

### Browser Auto-Discovery

When `browser_path=None`, `best_driver.start()` searches standard installation locations in order before falling back to `PATH` (`chrome`, `chromium`, `google-chrome`, `msedge`):

- `▸` **Windows**: `Program Files` / `Program Files (x86)` / `%LOCALAPPDATA%` for Google Chrome, Brave, and Microsoft Edge.
- `▸` **macOS**: `/Applications/Google Chrome.app`, `/Applications/Brave Browser.app`, `/Applications/Microsoft Edge.app`.
- `▸` **Linux**: `chrome`, `chromium`, `google-chrome`, `msedge` via `PATH`.

Pass `browser_path="/custom/path/to/chrome"` to target a specific binary (for example, a pinned Chrome for Testing build).

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

## Smart Locators and Shadow DOM

Every query-accepting method on `Tab` (`ele`, `eles`, `click`, `double_click`, `right_click`, `hover`, `drag`, `human_type`, `send_keys`, `fill`, `clear`, `focus`, `text`, `text_all`, `attr`, `attr_all`, `value`, `html`, `exists`, `count`, `wait_for`, `wait_for_selector`, `wait_for_hidden`, `select_option`, `check`, `uncheck`, `upload_file`, `scroll_into_view`, `screenshot`, `iframe`) and `Element` (`ele`, `eles`) uses one unified Smart Locator engine.

### Locator Syntax Rules

| Prefix / Pattern | Resolution Mode | Shadow DOM Piercing | Examples |
| :--- | :--- | :--- | :--- |
| `//`, `(//`, `.//` | XPath (`document.evaluate`) | Standard document XPath | `"//button[@type='submit']"`, `"(//li[@class='row'])[1]"`, `".//span"` |
| `text:` or `t:` | Deepest visible element whose `innerText`, `textContent`, `value`, or `aria-label` contains substring (case-insensitive) | Automatic open + closed (`mode: 'closed'`) via CDP fallback | `"text:Sign In"`, `"t:Confirm order"` |
| `@attr` or `@attr=value` | CSS attribute selector with automatic escaping of `\` and `"` | Automatic open + closed (`mode: 'closed'`) via CDP fallback | `"@name=email"`, `"@placeholder=Enter password"`, `"@disabled"` |
| Anything else | Standard CSS selector (`#id`, `.cls`, `tag`, `[attr]`, `[attr*=val]`, descendants) | Direct query first, then open JS walk, then CDP `pierce: true` for closed roots | `"#login-btn"`, `".card > a[href]"`, `"input[type='checkbox']"` |

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

# Locate inside open OR closed #shadow-root transparently (no frame or shadow switching needed)
shadow_button = await tab.ele("#button-inside-shadow-root")
if shadow_button:
    print(await shadow_button.outer_html())
    await shadow_button.click()

# Pierce an iframe hosted inside a closed #shadow-root (mode: 'closed')
widget_frame = await tab.iframe("iframe[src*='challenges']")
checkbox = await widget_frame.ele("input[type='checkbox']")
if checkbox:
    await checkbox.click()

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

### Lazy Auto-Retrying `Locator` Chains (`tab.locator()`)

On dynamic React, Vue, or Next.js pages that re-render DOM subtrees frequently, use `tab.locator(query)` instead of a snapshot `Element`. A `Locator` stores the chained query and re-resolves lazily on every action (piercing both open and closed `#shadow-root` trees), automatically retrying transient `StaleElement` detachments:

```python
target_btn = tab.locator("#feed").locator(".card").filter(has_text="Pro Plan").locator("button.buy")
await target_btn.wait_for(state="visible")
await target_btn.click()  # re-queries fresh & retries automatically if React re-renders mid-click

# Full form & state checks (works across open & closed Shadow DOM)
assert await tab.locator("#agree").is_checked() is False
await tab.locator("#agree").check()
await tab.locator("#tier").select_option("pro")

# Works identically across closed Shadow DOM
await tab.locator("#closed-host").locator("#closed-btn").click()
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

### Request & Response Interception, Mocking, `tab.fetch()`, and HAR 1.2 (`tab.route()`, `record_har()`)

Intercept outgoing requests (`stage="request"`) OR live server responses (`stage="response"`) via CDP `Fetch`, execute in-browser Chromium-TLS `tab.fetch()`, or export full **HAR 1.2** traces:

```python
async def mock_api(route):
    await route.fulfill(status=200, json={"ok": True, "source": "mocked"})

async def patch_live_response(route):
    data = await route.response_json()
    data["patched"] = True
    await route.fulfill(status=route.response_status or 200, json=data)

await tab.route("*/api/mocked*", mock_api)
await tab.route("*httpbin.org/json*", patch_live_response, stage="response")
await tab.route("*.png", lambda r: r.abort("BlockedByClient"))

# Execute a real in-browser fetch() over Chromium's exact TLS stack & cookie jar
res = await tab.fetch("https://httpbin.org/post", method="POST", json={"hello": "world"})
print(res["status"], res["body"])

# Record & export HAR 1.2 (drag-and-drop into Chrome DevTools Network tab)
await tab.record_har(include_bodies=True)
await tab.get("https://example.com")
await tab.export_har("trace.har")
await tab.stop_har()

# Remove one pattern or all routes
await tab.unroute("*.png")
await tab.unroute()
```

### Continuous Async Network Streams (`listen_requests()` & `listen_responses()`)

Stream every matching outgoing request or incoming response continuously using `async with` / `async for` (with lazy `await resp.body()`, `await resp.text()`, and `await resp.json()` decoding):

```python
async with tab.listen_responses("*/api/*") as stream:
    await tab.click("#load-feed")
    resp = await stream.next(timeout_ms=10000)
    if resp is not None:
        print(resp.status, resp.url, await resp.json())
```

### Sniffing a Single AJAX / JSON Response (`wait_for_response`)

`tab.wait_for_response(url_pattern, timeout_ms=30000)` subscribes to the tab's filtered CDP event stream, matches `Network.responseReceived` against either a substring or a `*`-wildcard glob (e.g. `"*/api/v1/items*"`), waits on the same stream for `Network.loadingFinished` (or `Network.loadingFailed`) for that `requestId` so large payloads finish transferring, and calls `Network.getResponseBody` (automatically decoding UTF-8 base64 payloads).

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

# Symmetric Playwright-format storage_state (cookies + active-origin localStorage)
# Every file-path API accepts both str and pathlib.Path (os.PathLike)
from pathlib import Path
state_file = Path("auth_state.json")
await tab.storage_state(state_file)
restored = await tab.load_storage_state(state_file)  # {"cookies": N, "local_storage": M}
```

### Isolated Incognito Contexts (`browser.new_context()`)

Run multiple fully isolated sessions (separate cookies, `localStorage`, optional per-context authenticated proxy with automatic WebRTC UDP leak protection, viewport, locale, timezone, extra headers, and `add_init_script`) inside **one shared Chromium process and one WebSocket** via CDP `Target.createBrowserContext`:

```python
async with await best_driver.start(headless=True) as browser:
    async with (
        await browser.new_context(
            proxy="http://user1:pass1@proxy1:8080",
            viewport={"width": 1280, "height": 720},
            timezone="Asia/Tokyo",
        ) as ctx_a,
        await browser.new_context(proxy="http://user2:pass2@proxy2:8080") as ctx_b,
    ):
        await ctx_a.add_init_script("window.__env = 'ctx_a';")
        tab_a, tab_b = await asyncio.gather(
            ctx_a.get("https://example.com"),
            ctx_b.get("https://example.com"),
        )
        # Cookies, HttpOnly tokens, localStorage, and proxies are 100% isolated
        # tab_a.cookies(), tab_a.storage_state(), and tab_a.tls_session() scope strictly to ctx_a!
```

### Persistent User Profiles and Graceful Shutdown

When `user_data_dir` is omitted, `best-driver` creates a temporary profile directory and removes it automatically via RAII (`tempfile::TempDir`) when the browser stops.

When `user_data_dir="./my_profile"` is provided, `browser.stop()` (and `async with` exit) first sends `Browser.close` and waits up to 1.5 seconds for Chromium to exit cleanly so SQLite databases (`Cookies`, `Local Storage`, `Preferences`) flush cleanly to disk before falling back to process termination.

```python
async with await best_driver.start(headless=True, user_data_dir="./my_profile") as browser:
    assert await browser.is_alive() is True
    tab = await browser.get("https://example.com")
```

### Keep-Open, Re-Attach (`connect()`), and Safe Profile Cleanup (`cleanup()`)

Leave Chromium open after your script finishes (e.g. interactive headful login or debugging), attach to any already-running Chrome/Edge/Brave instance over CDP without taking process ownership, or safely sweep stale temporary profiles without risking live bot instances:

```python
import asyncio
import best_driver


async def keep_open_and_reattach() -> None:
    # 1. Launch and keep Chromium alive even after `async with` exits
    async with await best_driver.start(headless=False, keep_open=True) as browser:
        port = browser.port
        await browser.get("https://example.com")
        # Or detach dynamically mid-run: browser.detach()

    # 2. Re-attach later by port, HTTP URL, host:port, or ws://... URL
    #    stop() / async with exit closes the CDP WebSocket cleanly without killing Chrome
    async with await best_driver.connect(port) as attached:
        tab = await attached.main_tab()
        print("Re-attached:", await tab.title())
        # Optionally wait until the user closes the window manually:
        # await attached.wait_until_closed()


# 3. Conflict-safe cleanup of stale `best_driver_profile_*` temp directories
#    Never touches personal Chrome profiles or browsers owned by live Python scripts.
#    Uses Windows atomic rename staging + POSIX SingletonLock/PPID tree checks.
report = best_driver.cleanup(
    kill_orphans=False,      # Set True to terminate true orphan Chromium trees (dead parent PID)
    min_age_seconds=60.0,    # Skip profiles touched in the last 60s
    dry_run=False,           # Set True to preview without deleting
)
print(report["removed_dirs"], report["freed_bytes"], report["skipped_in_use"])
```

### Authenticated HTTP and SOCKS5 Proxies

Inline proxy credentials (`http://user:pass@host:port`) are split automatically before launching Chromium; authentication challenges are answered natively via CDP `Fetch.authRequired` without requiring browser extensions:

```python
browser = await best_driver.start(
    headless=True,
    proxy="http://username:password@proxy.example.com:8080",
)
```

For handing off the active session (including proxy, IP family, cookies, and TLS/HTTP2 headers) to `http_tls`, see [Synchronized `http_tls` Session Handoff](#synchronized-http_tls-session-handoff).

---

## Popups, Events, and Utilities

### Race-Free Popup Capture (`expect_popup` & `listen_popups`)

Capture `window.open()` and `<a target="_blank">` popups with stealth evasion, `BrowserContext` proxy/emulation, and init scripts pre-armed before the first JavaScript instruction runs (zero `Runtime.enable` overhead):

```python
# Playwright-style context manager (recommended)
async with tab.expect_popup(timeout_ms=15000) as popup_info:
    await tab.click("a[target='_blank']")
popup = await popup_info.value
print("Popup URL:", await popup.url())

# Or stream multiple popups continuously
async with tab.listen_popups() as popups:
    async for p in popups:
        print("Opened:", await p.url())
        break
```

### Opt-In Console, Page Errors, and Renderer Crash Monitoring

`listen_console()` and `listen_page_errors()` use reference-counted `Runtime.enable` / `Runtime.disable` scoped strictly to the active block so default stealth is restored automatically on exit. `listen_crashes()` and `tab.is_alive()` never touch `Runtime.enable`:

```python
# Capture warnings & errors, then auto-disable Runtime on exit
async with tab.listen_console(types=["warning", "error"]) as console_stream:
    await tab.evaluate("console.warn('rate limit approaching')")
    msg = await console_stream.next(timeout_ms=5000)
    print(msg.type, msg.text, msg.url, msg.line)

async with tab.listen_page_errors() as err_stream:
    err = await err_stream.next(timeout_ms=5000)
    if err:
        print(err.name, err.message)

# Detect renderer crashes in long-running 24/7 workers
async with tab.listen_crashes() as crashes:
    alive = await tab.is_alive()
```

### Permissions, OS Window Bounds, and Auto-Dialogs

```python
# Context-scoped permissions (Playwright aliases supported)
await tab.grant_permissions(["clipboard-read", "clipboard-write", "notifications"])

# Real OS window control for headful sessions
bounds = await tab.get_window_bounds()
await tab.set_window_bounds(width=1280, height=800, left=80, top=60)
await tab.set_window_bounds(state="maximized")

# Automatic JavaScript alert/confirm/prompt handling (pass None to disable)
tab.auto_dialog(accept=True, prompt_text="confirmed")
```

---

## Biological Input Physics and Forms

- **Cubic Bezier Cursor Movement**: `mouse_move()`, `mouse_click()`, `click()`, `hover()`, and `Element.click()` move along a randomized 4-point Cubic Bezier arc with smoothstep acceleration/deceleration easing, sub-pixel micro-tremors, and slight off-center target landing inside the element's bounding box.
- **Biological Typing (`human_type`)**: Dispatches real `keyDown`/`keyUp` CDP events with variable hold durations and natural inter-key pauses at spaces, punctuation (`.`, `,`, `!`, `?`), and word boundaries. Supports `\n` to press Enter.
- **Instant Input (`fill`)**: Clears via native selection + Backspace and inserts text immediately via `Input.insertText` (fires React/Vue `input` events).
- **Slider Captcha Gestures (`swipe`)**: Smooth S-curve drag with perpendicular micro-jitter while holding `buttons: 1`.
- **Special Keys & Modifier Chords (`press_key`)**: Supports single keys (`Enter`, `Tab`, `Escape`, `Backspace`, `Delete`, `Insert`, `Space`, arrow/nav keys, `F1`–`F12`) and `+`-separated chords (`"Control+a"`, `"Control+c"`, `"Control+v"`, `"Ctrl+Shift+z"`, `"Shift+Enter"`, `"Meta+a"`) with native Blink editing commands (`selectAll`, `copy`, `cut`, `paste`, `undo`, `redo`).
- **Forms, Closed Shadow DOM & `pathlib.Path`**: `select_option()`, `check()` / `uncheck()`, `upload_file()`, `fill()`, `human_type()`, `wait_for()`, `attr()`, `value()`, `html()`, `hover()`, and `screenshot(selector=...)` all fall back seamlessly into `#shadow-root (closed)` trees; every file-path parameter accepts `str` or `pathlib.Path` (`os.PathLike`).

```python
from pathlib import Path

await tab.human_type("@name=query", "rust cdp driver\n", clear=True)
await tab.press_key("Control+a")
await tab.press_key("Control+c")
await tab.select_option("#country", "United States")
await tab.check("#accept-terms")
await tab.upload_file("input[type='file']", [Path("report.pdf"), Path("summary.png")])
await tab.swipe(120.0, 340.0, 380.0, 342.0)
```

---

## Stealth and Anti-Detection Architecture

### Seven Defense Layers

1. **Zero `Runtime.enable` CDP Leak**: `Tab::new()` intentionally omits `Runtime.enable`. All script evaluations (`Runtime.evaluate`, `Runtime.callFunctionOn`), iframe isolated worlds (`Page.createIsolatedWorld`), and handle cleanup (`Runtime.releaseObject`) operate without activating V8's `Runtime` inspector domain, so `console.debug(new Error())` `Error.stack` getter and object-property serialization traps used by Cloudflare Turnstile and DataDome evaluate to `False`.
2. **Headless UA and Client Hints Alignment**: At launch, `BrowserCore` queries `Browser.getVersion`. Whenever `HeadlessChrome/` is present, `Emulation.setUserAgentOverride` is applied across all sessions with synchronized `userAgent` and `userAgentMetadata` (`brands`, `fullVersionList`, `platform`, `platformVersion`, `architecture`, `bitness`) matching the real installed Chrome version and host OS.
3. **Pre-Execution Auto-Attach for Popups and OOPIFs**: The browser connection runs `Target.setAutoAttach(autoAttach=True, waitForDebuggerOnStart=True, flatten=True)` supervised by a background Tokio task synchronized with per-target `tokio::sync::OnceCell`. Every newly spawned `window.open()` popup or cross-origin OOPIF receives `evasion.js` and the UA override before its first JavaScript instruction runs, then resumes unconditionally via `Runtime.runIfWaitingForDebugger`.
4. **Prototype-Only Native Patching (`src/evasion.js`)**:
   - `Navigator.prototype.webdriver` getter returns `false` with authentic descriptor (`'webdriver' in Navigator.prototype === true`, `configurable: true`, `enumerable: true`).
   - All patched getters and methods live on their real prototypes (`Navigator.prototype`, `Permissions.prototype`, `SpeechSynthesis.prototype`, `MediaDevices.prototype`) using non-constructable concise method syntax (`'prototype' in fn === false`) masked via `Function.prototype.toString` to return `function <name>() { [native code] }`.
   - `Object.getOwnPropertyNames(navigator)` and `Object.getOwnPropertyNames(navigator.permissions)` remain empty (`[]`).
5. **Zero-DOM-Mutation Locators & Closed Shadow DOM Piercing**: `Element` handles bind directly to Chromium's internal `backendNodeId` rather than injecting DOM attributes. When a selector targets nodes inside a `#shadow-root (closed)`, `best-driver` falls back to CDP `DOM.getDocument(pierce=True)` / `DOM.describeNode(pierce=True)` without injecting wrapper scripts into page context.
6. **Safe Canvas, Audio, WebGL, and Environment Consistency**:
   - WebGL `UNMASKED_VENDOR_WEBGL` / `UNMASKED_RENDERER_WEBGL` spoofed to `Google Inc. (NVIDIA)` / `ANGLE (NVIDIA, NVIDIA GeForce RTX 4070 Direct3D11 vs_5_0 ps_5_0, D3D11)`.
   - Canvas noise uses a per-session shift applied via an offscreen clone in `toDataURL`, skipping transparent pixels (`alpha === 0`) and small probe canvases (`< 16px`) so live canvases are never mutated and `TRANSPARENT_PIXEL` tests stay `[0, 0, 0, 0]`.
   - AudioContext applies a tiny once-per-buffer salt tracked in a `WeakSet`.
   - Provides realistic fallback entries for `SpeechSynthesis.getVoices()`, `MediaDevices.enumerateDevices()`, `navigator.connection` (`4g`), `PluginArray` / `MimeTypeArray`, `window.chrome`, and `outerWidth` / `outerHeight` window frame offsets when headless omits them.
7. **Biological Input Physics**: Cubic Bezier cursor arcs, micro-tremors, variable keystroke cadence, and S-curve swipe physics.

### Verified Headless Benchmark (`bot.sannysoft.com`)

Verified in `--headless=new` mode (`headless=True`) with strict CSS-class inspection (`failed` and `warn` counts across all 57 rows):

| Check Group | Observed Result | Status |
| :--- | :--- | :---: |
| `User Agent (Old)` & `HEADCHR_UA` | `Chrome/<host-version>` (no `HeadlessChrome`) | `✓` Passed |
| `WebDriver (New)` & `WebDriver Advanced` | `missing (passed)` / `passed` | `✓` Passed |
| `Chrome (New)` & `Permissions (New)` | `present (passed)` / `default` | `✓` Passed |
| `Plugins Length` & `PluginArray` type | `5` / `passed` | `✓` Passed |
| `Languages` & `CHR_MEMORY` | `en-US,en` / `ok` | `✓` Passed |
| `WebGL Vendor` & `WebGL Renderer` | `Google Inc. (NVIDIA)` / `RTX 4070 Direct3D11` | `✓` Passed |
| `TRANSPARENT_PIXEL` & `Broken Image Dimensions` | `ok` (`[0, 0, 0, 0]`) / `16x16` | `✓` Passed |
| `HEADCHR_*`, `PHANTOM_*`, `SELENIUM_DRIVER`, `CHR_DEBUG_TOOLS` | `ok` (0 failures) | `✓` Passed |
| `Object.getOwnPropertyNames(navigator)` | `[]` | `✓` Passed |
| `MutationObserver` during `ele()` / `eles()` | `[]` | `✓` Passed |
| `Runtime.enable` `console.debug(err)` `Error.stack` & object getter traps | `False` / `False` | `✓` Passed |
| **Total Sannysoft Headless Score** | **57 / 57 passed (0 failed, 0 warnings)** | **`✓` Passed** |

In addition to `bot.sannysoft.com`, live headless verification confirmed DuckDuckGo search submission via `@name=q` + `human_type` and clean page load on `nowsecure.nl` without triggering an interstitial challenge.

### Known Limitations on Stock Chromium

`best-driver` intentionally runs on stock, unmodified Chromium binaries so users do not have to maintain custom browser builds. While `Runtime.enable` leaks and closed Shadow DOM barriers are eliminated at the CDP layer, browser-binary-level signals (such as OS font inventory mismatches on minimal Linux containers, datacenter IP reputation, or TLS/JA4 fingerprints) still depend on your host environment and proxy layer rather than a custom-compiled engine like Camoufox.

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

### PEP 561 Static Typing & IDE Autocomplete (VS Code & PyCharm)

The package ships with `py.typed`, [`python/best_driver/__init__.pyi`](https://github.com/real-coder-xD/best-driver/blob/main/python/best_driver/__init__.pyi), and [`python/best_driver/best_driver.pyi`](https://github.com/real-coder-xD/best-driver/blob/main/python/best_driver/best_driver.pyi) (sibling stub for the compiled Rust `.pyd`/`.so` extension), exporting full signatures, hover docstrings, `@overload`s (for `tls_session()` and `storage_get()`), `Literal`s (`WaitUntil`, `MouseButton`, `ImageFormat`, `RouteStage`, `LocatorState`, `WindowState`, `PermissionName`, `SpecialKey`), and `TypedDict`s (`ElementRect`, `ElementDict`, `Cookie`, `FetchResponse`, `NetworkResponse`, `FrameInfo`, `DownloadInfo`, `BrowserFingerprint`, `StorageState`, `LoadedStorageState`, `WindowBounds`, `ViewportSize`, `CleanupReport`).

- **VS Code (Pylance / Pyright)**: Works out of the box with zero configuration — select the Python interpreter where `best-driver` is installed (`Ctrl+Shift+P` -> `Python: Select Interpreter`). After upgrading `best-driver`, run `Developer: Reload Window` (or `Python: Restart Language Server`) if Pylance caches an older stub.
- **PyCharm**: Works out of the box via `py.typed` + `best_driver.pyi` (no binary skeleton guesswork on the Rust extension). Ensure the project interpreter points to the virtualenv containing `best-driver`; after upgrading, run `File -> Invalidate Caches...` only if PyCharm retains stale stubs.
- **Mypy / Pyright CLI**: Recognized automatically as a PEP 561 typed package (`Typing :: Typed`).

---

## API Reference

### Top-Level Functions

#### `best_driver.start(...)`

```python
async def start(
    headless: bool = False,
    user_data_dir: Optional[PathLike] = None,
    browser_path: Optional[PathLike] = None,
    extra_args: Optional[Sequence[str]] = None,
    proxy: Optional[str] = None,
    keep_open: bool = False,
    start_url: Optional[str] = None,
) -> Browser
```

Launches Chromium on an OS-assigned loopback debugging port, connects a single browser-level CDP WebSocket, strips `HeadlessChrome` UA/Client Hints if running headless, and arms `Target.setAutoAttach` for popups and OOPIFs. By default `start_url=None` skips appending `about:blank` (so Chromium opens its native New Tab page, respects persistent profile restore, and avoids duplicate blank tabs when URLs or `--no-startup-window` are passed); pass `start_url="about:blank"` or `start_url="https://..."` to open a specific page on launch. Set `keep_open=True` to leave Chromium running independently after `stop()`, `async with`, or Python exit.

#### `best_driver.connect(...)`

```python
async def connect(
    endpoint: Union[int, str],
    proxy: Optional[str] = None,
) -> Browser
```

Attaches over flattened CDP to an already-running Chromium instance (`9222`, `"http://127.0.0.1:9222"`, `"host:port"`, or `"ws://..."`) without taking process ownership (`stop()` disconnects cleanly without killing the external browser).

#### `best_driver.cleanup(...)`

```python
def cleanup(
    *,
    kill_orphans: bool = False,
    min_age_seconds: float = 0.0,
    exclude_dirs: Optional[Sequence[str]] = None,
    dry_run: bool = False,
    temp_dir: Optional[str] = None,
) -> CleanupReport
```

Safely removes stale `best_driver_profile_*` temporary directories (and optionally terminates true orphan Chromium processes whose parent launcher PID is dead when `kill_orphans=True`). Active browsers owned by live scripts and personal Chrome profiles are never touched.

---

### `Browser`

| Member | Signature / Type | Description |
| :--- | :--- | :--- |
| `port` | `int` (property) | Ephemeral loopback TCP debugging port (`127.0.0.1:<port>`). |
| `user_data_dir` | `str` (property) | Active Chromium profile directory path. |
| `ws_url` | `str` (property) | Browser-level CDP WebSocket URL shared by all flattened sessions. |
| `proxy` | `Optional[str]` (property) | Proxy URL passed to `best_driver.start(proxy=...)`, or `None`. |
| `is_alive()` | `async () -> bool` | Returns `True` if the Chromium process / CDP WebSocket is still running. |
| `detach()` | `() -> None` | Detaches Chromium from `stop()` / `Drop` cleanup so the window stays open after Python exits. |
| `wait_until_closed()` | `async (poll_ms: int = 250) -> None` | Awaits until the Chromium window is closed or the CDP WebSocket disconnects. |
| `main_tab()` | `async () -> Tab` | Returns the primary active page tab (creating one if none exist). |
| `new_tab()` | `async (url: str = "about:blank") -> Tab` | Opens a new tab via `Target.createTarget` with evasion armed before navigation. |
| `tabs()` | `async () -> list[Tab]` | Lists all open `page` targets via `Target.getTargets`. |
| `get()` | `async (url: str, new_tab: bool = False, wait_until: WaitUntil = "load") -> Tab` | Navigates the main tab (or a new tab if `new_tab=True`) to `url`. |
| `new_context()` | `async (proxy=None, ..., viewport=None, locale=None, timezone=None, extra_headers=None) -> BrowserContext` | Creates an isolated incognito context with separate cookies, storage, proxy, and emulation overrides. |
| `stop()` | `async () -> None` | Sends graceful `Browser.close` (up to 1.5s wait), then kills and reaps the child process (or disconnects cleanly when detached). |
| Context manager | `async with await best_driver.start() as browser:` | Automatically calls `await browser.stop()` on block exit. |

### `BrowserContext`

| Member | Signature / Type | Description |
| :--- | :--- | :--- |
| `context_id`, `proxy` | `str` / `Optional[str]` (properties) | CDP `browserContextId` and optional per-context proxy URL. |
| `add_init_script()` | `async (script: str) -> None` | Injects JavaScript before page load for all current and future tabs in this context. |
| `main_tab()` / `new_tab()` / `tabs()` / `get()` | Same signatures as `Browser` | Opens and manages isolated tabs scoped strictly to this context. |
| `close()` / `async with` | `async () -> None` | Disposes context via `Target.disposeBrowserContext`, wiping its tabs, cookies, and storage. |

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
| `frame_id`, `proxy`, `context_id` | `Optional[str]` (properties) | In-process iframe ID, inherited proxy URL, and owning `BrowserContext` ID. |
| `add_init_script()` | `async (script: str) -> str` | Injects JS via `Page.addScriptToEvaluateOnNewDocument` before any page script executes. |
| `fetch()` | `async (url, method="GET", headers=None, body=None, json=None, credentials="include") -> FetchResponse` | Executes a real in-browser `fetch()` over Chromium's native TLS stack and cookie jar. |
| `fingerprint()` | `async () -> BrowserFingerprint` | Captures live `impersonate`, `user_agent`, `sec_ch_ua*`, `accept_language`, `headers`, `header_order`, and `proxy`. |
| `storage_state()` / `load_storage_state()` | `async (path=None) -> StorageState` / `async (state_or_path) -> dict[str, int]` | Exports and restores context-scoped CDP cookies (`HttpOnly`) plus active-origin `localStorage` in Playwright / `http_tls` format. |
| `tls_session()` | `async (async_mode: bool = True, **kwargs) -> HttpTlsAsyncSession \| HttpTlsSession` | Creates a typed `http_tls.AsyncSession` (or `Session`) synced with this tab's fingerprint, proxy, headers, and cookies (supports `http-tls>=1.3.0` ECH/HTTP/3/DoH kwargs). |
| `sync_from_tls()` | `async (session, default_url: Optional[str] = None) -> int` | Pushes updated cookies from an `http_tls` session back into this Chromium tab. |
| `locator()` | `(query: str) -> Locator` | Returns a lazy, auto-retrying chained `Locator` (`filter`, `first`, `last`, `nth`, `wait_for`, forms, mixed-comma Smart Locators). |
| `route()` / `unroute()` | `async (url_pattern, handler, stage="request")` / `async (url_pattern=None)` | Intercepts requests or responses (`fulfill`, `continue_`, `continue_response`, `abort`, `fetch`). |
| `record_har()` / `export_har()` / `stop_har()` | `async (...) -> HarRecorder \| dict` | Records and exports full HAR 1.2 network traces for this tab. |
| `listen_requests()` / `listen_responses()` | `(url_pattern: Optional[str] = None) -> RequestStream \| ResponseStream` | Opens a continuous live async stream of matching HTTP requests or responses. |
| `expect_popup()` / `listen_popups()` | `(timeout_ms: int = 30000) -> PopupExpectation` / `() -> PopupStream` | Race-free popup capture (`window.open` / `target="_blank"`) inheriting owning `BrowserContext` and pre-armed stealth. |
| `listen_console()` / `listen_page_errors()` | `(types=None) -> ConsoleStream` / `() -> PageErrorStream` | Opt-in refcounted `Runtime.enable` streams that automatically call `Runtime.disable` on exit to restore stealth. |
| `listen_crashes()` / `is_alive()` | `() -> CrashStream` / `async () -> bool` | Stealth-safe `Inspector.targetCrashed` stream and live target/renderer health check. |
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
| `press_key()` | `async (key: str) -> None` | Dispatches a single key or modifier chord (`"Enter"`, `"F5"`, `"Control+a"`, `"Ctrl+Shift+Z"`, `"Shift+Enter"`). |
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
| `get_window_bounds()` / `set_window_bounds()` | `async () -> dict` / `async (width=None, height=None, left=None, top=None, state=None) -> None` | Reads or updates real OS window geometry and state (`normal`, `minimized`, `maximized`, `fullscreen`). |
| `set_user_agent()` / `set_timezone()` / `set_locale()` / `set_geolocation()` | `async (...) -> None` | Emulation overrides for UA, timezone, locale, and coordinates. |
| `grant_permissions()` | `async (permissions: Sequence[str], origin: Optional[str] = None) -> None` | Grants context-scoped browser permissions (`notifications`, `clipboard-read`, `clipboard-write`, `camera`, `microphone`, `geolocation`). |
| `auto_dialog()` / `handle_dialog()` | `(accept=True, prompt_text=None)` / `async (...)` | Synchronous auto-handler toggle or one-shot JS dialog response. |
| `screenshot()` | `async (file_path=None, full_page=False, format="png", quality=None, selector=None) -> bytes` | PNG/JPEG/WEBP capture of viewport, full document, or element clip. |
| `pdf()` | `async (file_path=None, landscape=False) -> bytes` | Vector PDF export with background graphics enabled. |
| `sleep()` | `async (seconds: float) -> None` | Non-blocking Tokio sleep. |
| `send()` | `async (method: str, params: Optional[dict[str, Any]] = None) -> Any` | Raw CDP command scoped to this tab's flattened `sessionId`. |

---

## Architecture Under the Hood

```text
┌────────────────────────────────────────────────────────────┐
│                Python Application (PEP 561)                │
│        await browser.get("https://...")  +  .pyi           │
└─────────────────────────────┬──────────────────────────────┘
                              │ PyO3 0.30 (direct serde Value FFI)
                              ▼
┌────────────────────────────────────────────────────────────┐
│               Rust Core (best_driver.cdylib)               │
│  ┌──────────────────────────────────────────────────────┐  │
│  │              Tokio Asynchronous Runtime              │  │
│  │  ▸ Single Browser-Level WebSocket (Zero reqwest)     │  │
│  │  ▸ Flattened SessionId Multiplexer (Target API)      │  │
│  │  ▸ Zero Runtime.enable CDP Leak Architecture         │  │
│  │  ▸ OnceCell Auto-Attach Stealth (Popups & OOPIFs)    │  │
│  │  ▸ Native CDP backendNodeId + Closed Shadow Piercing │  │
│  │  ▸ Cubic Bezier Mouse & Biological Typing Engine     │  │
│  └──────────────────────────┬───────────────────────────┘  │
└─────────────────────────────┼──────────────────────────────┘
                              │ Single CDP WebSocket (flatten: true)
                              ▼
┌────────────────────────────────────────────────────────────┐
│                      Chromium Engine                       │
│         (Google Chrome / Brave / Edge / Chromium)          │
│     ▸ Target.attachToTarget + Target.setAutoAttach         │
│     ▸ Page.addScriptToEvaluateOnNewDocument (Evasion)      │
│     ▸ Input / Network / Fetch / DOM (pierce) / Emulation   │
└────────────────────────────────────────────────────────────┘
```

### Rust Source Layout

| File | Responsibility |
| :--- | :--- |
| `src/cdp.rs` | Single browser-level `CdpConnection` WebSocket multiplexer, lightweight `CdpSession` per `sessionId`, and session-filtered `SessionEventRx`. |
| `src/browser.rs` | Chromium process launcher, pure-Tokio `TcpStream` `/json/version` discovery (`Content-Length` framed), headless UA/Client Hints builder, graceful shutdown + RAII temp profile cleanup. |
| `src/tab.rs` | Per-tab and per-iframe controller (without `Runtime.enable`): navigation, waiting, native `backendNodeId` element ops, closed Shadow DOM fallback, iframe scoping, download manager, network sniffer, proxy auth, cookies, storage, and capture. |
| `src/locator.rs` | Smart Locator JS codegen (`sel_expr`, `sel_all_expr`, `child_sel_fn`, `child_sel_all_fn`) plus pure-Rust `find_pierced_backend_node_ids` walker for `#shadow-root (closed)`. |
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
Both open (`mode: 'open'`) and closed (`mode: 'closed'`) `#shadow-root` trees are pierced automatically by Smart Locator queries (fast JS path first, with native CDP `DOM.getDocument(pierce=True)` fallback for closed roots); `tab.iframe(selector)` returns a full `Tab` controller for same-origin, nested, `srcdoc`, closed-shadow-hosted, and cross-origin OOPIF iframes with automatic coordinate translation; `tab.wait_for_download(dir)` captures downloaded files with full metadata; and `tab.wait_for_response(pattern)` waits for `Network.loadingFinished` and returns headers plus the UTF-8 decoded response body.

### Does `best-driver` require `chromedriver` or Node.js?
No. It requires only Python `>= 3.8` and any locally installed Chromium-based browser (Chrome, Brave, Edge, or Chromium).

---

## Changelog and License

- `▸` Release history: [CHANGELOG.md](https://github.com/real-coder-xD/best-driver/blob/main/CHANGELOG.md)
- `▸` License: MIT — see [LICENSE](https://github.com/real-coder-xD/best-driver/blob/main/LICENSE)
