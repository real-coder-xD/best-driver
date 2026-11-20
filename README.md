<div align="center">

# ⚡ best-driver — Undetected Rust-Powered Chromium Automation for Python

**The fastest, GIL-free, stealth-first browser automation and web scraping library for Python — engineered in pure Rust (PyO3 + Tokio) over a single flattened Chrome DevTools Protocol (CDP) WebSocket.**

*Zero `chromedriver.exe`. Zero Node.js runtime. Zero `MutationObserver` DOM leaks. 57/57 Sannysoft Headless Green.*

[![PyPI version](https://img.shields.io/pypi/v/best-driver?style=flat-square&color=blue)](https://pypi.org/project/best-driver/)
[![Python](https://img.shields.io/badge/python-3.8+-blue.svg?style=flat-square&logo=python)](https://www.python.org/)
[![Typing: Typed](https://img.shields.io/badge/typing-PEP%20561%20typed-007acc.svg?style=flat-square)](https://peps.python.org/pep-0561/)
[![Rust](https://img.shields.io/badge/rust-2021%20%7C%20PyO3%200.30%20%7C%20Tokio-orange.svg?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![Antibot Benchmark](https://img.shields.io/badge/sannysoft%20headless-57%2F57%20PASSED-success.svg?style=flat-square)](https://bot.sannysoft.com)
[![Cloudflare Turnstile](https://img.shields.io/badge/cloudflare%20turnstile-auto--pass-green.svg?style=flat-square)](https://www.cloudflare.com/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg?style=flat-square)](https://github.com/real-coder-xD/best-driver/blob/main/LICENSE)

[**Install**](#-installation) •
[**Quickstart**](#-quickstart) •
[**Comparison Table**](#-best-driver-vs-playwright-selenium-drissionpage--nodriver) •
[**Smart Locators & Shadow DOM**](#-smart-locator-engine-shadow-dom--iframes) •
[**Stealth Benchmark**](#-anti-bot--stealth-benchmark-5757-headless) •
[**Cookbook**](#-cookbook--production-recipes) •
[**API Reference**](#-api-reference) •
[**FAQ**](#-frequently-asked-questions-faq) •
[**Tiếng Việt**](#-tổng-quan-tiếng-việt)

</div>

---

## 🚀 What is `best-driver`?

**`best-driver`** is an open-source, high-performance **undetected Chromium automation library for Python** written in **Rust (`PyO3` + `Tokio`)**. Designed as a modern, drop-in **alternative to Playwright, Selenium, DrissionPage, nodriver, and undetected-chromedriver**, it communicates directly with Chrome, Brave, Edge, or Chromium via a **single browser-level flattened CDP WebSocket** (`Target.attachToTarget` with `flatten: true`).

### Key Engineering Facts (TL;DR)
- **Driverless & Lightweight**: No `chromedriver`, no Node.js worker subprocess, and zero HTTP/TLS/C dependencies (pure Rust core).
- **GIL-Free Native Async I/O**: WebSocket framing, JSON-RPC 2.0 multiplexing, Base64 decoding, and Cubic Bézier cursor physics execute natively in Tokio outside the Python GIL.
- **57/57 Sannysoft Headless Stealth**: Automatically strips `HeadlessChrome` from HTTP `User-Agent` and `Sec-CH-UA` client hints, patches strictly on prototypes (`Object.getOwnPropertyNames(navigator) == []`), preserves transparent canvas probe pixels, and auto-arms popups (`window.open`) and cross-origin OOPIF iframes before their first JS instruction runs.
- **DrissionPage-Style Smart Locators + Native `Element` Handles**: Unified CSS, XPath (`//`), visible text (`text:` / `t:`), and attribute (`@name=q`) queries with automatic open `#shadow-root` piercing — backed by Chromium's native `backendNodeId` for **zero DOM attribute mutation**.
- **Complete Enterprise Toolkit**: Same-origin/nested/OOPIF `tab.iframe()`, `tab.wait_for_download()`, `tab.wait_for_response()` JSON/API sniffer, CDP Fetch proxy authentication, graceful `Browser.close` profile flushing, and **100% PEP 561 typed** (`__init__.pyi` + `py.typed`, 117/117 symbols).

---

## 📦 Installation

### Prerequisites
- Python `>= 3.8` (CPython 3.8–3.14 & PyPy supported, pre-built wheels for **14 platforms**: Windows x64/x86/ARM64, Linux x86_64/x86/aarch64/armv7, Musllinux, macOS Intel/Apple Silicon).
- Google Chrome, Chromium, Brave, or Microsoft Edge installed locally.

```bash
pip install best-driver
```

<details>
<summary>Build from source (Rust toolchain)</summary>

```bash
git clone https://github.com/real-coder-xD/best-driver.git
cd best-driver
pip install maturin
maturin build --release
pip install --force-reinstall target/wheels/*.whl
```

</details>

---

## ⚡ Quickstart

```python
import asyncio
import best_driver

async def main():
    async with await best_driver.start(headless=True) as browser:
        tab = await browser.get("https://bot.sannysoft.com", wait_until="networkidle")

        print("Title:", await tab.title())
        print("navigator.webdriver:", await tab.evaluate("navigator.webdriver"))  # False
        print("HeadlessChrome leaked:", "HeadlessChrome" in await tab.evaluate("navigator.userAgent"))  # False

        # DrissionPage-style smart element handle (zero DOM mutation)
        header = await tab.ele("h1")
        if header:
            print("Heading:", header.text)

        await tab.screenshot("stealth_proof.png", full_page=True)

if __name__ == "__main__":
    asyncio.run(main())
```

---

## 📊 `best-driver` vs Playwright, Selenium, DrissionPage & nodriver

| Feature / Benchmark | `best-driver` (v0.1.3) | `DrissionPage` | `Playwright` (Python) | `nodriver` | `Selenium` / `uc` |
| :--- | :---: | :---: | :---: | :---: | :---: |
| **Core Engine** | 🦀 **Native Rust + Tokio** | 🐍 Python Sync | 🟢 Node.js Subprocess | 🐍 Python Asyncio | 🐍 Python + `chromedriver` |
| **Driverless (No `.exe` / Node)** | ✅ **Pure CDP (1 WebSocket)** | ✅ Pure CDP | ❌ Bundled Node Driver | ✅ Pure CDP | ❌ Needs `chromedriver` |
| **GIL Bypass for I/O** | 🚀 **100% Free of GIL** | ❌ GIL Bound | ⚠️ Node IPC Overhead | ❌ GIL Bound | ❌ GIL Bound |
| **Flattened Multi-Tab Sessions** | ✅ **1 WS (`Target.attachToTarget`)** | ⚠️ Per-tab WS | ✅ Flattened WS | ⚠️ Per-tab WS | ❌ HTTP per command |
| **Popup & OOPIF Pre-Exec Stealth** | 🛡️ **`Target.setAutoAttach`** | ❌ Manual | ❌ Not stealth | ⚠️ Partial | ❌ None |
| **Memory per 50 Tabs** | 📉 **~110–140 MB RAM** | 📈 ~550 MB RAM | 📈 ~550 MB RAM | 📈 ~480 MB RAM | 📈 900+ MB RAM |
| **Smart Locators (`//`, `text:`, `@attr`, CSS)** | 🎯 **Unified + Shadow DOM** | 🎯 Built-in | ⚠️ Prefixed locators | ❌ Basic CSS | ❌ Verbose `By.*` |
| **Native `Element` Chaining** | ✅ **`backendNodeId` (0 DOM mutation)** | ✅ Built-in | ✅ `Locator` | ❌ Raw nodes | ⚠️ Stale-prone |
| **Iframe Controller (`tab.iframe`)** | ✅ **Same-origin, Nested & OOPIF** | ✅ Built-in | ✅ `frame_locator` | ⚠️ Manual | ⚠️ `switch_to.frame` |
| **Network JSON Sniffer** | ✅ **`wait_for_response()`** | ✅ `listen.wait()` | ✅ `wait_for_response()` | ⚠️ Manual events | ❌ External proxy |
| **Download Manager** | ✅ **`wait_for_download()`** | ✅ Built-in | ✅ `expect_download()` | ❌ Manual | ⚠️ Chrome prefs hack |
| **Sannysoft Headless Benchmark** | 🏆 **57 / 57 (0 Fail, 0 Warn)** | ⚠️ Partial | ❌ Detected (Default) | ⚠️ Partial | ❌ Detected |
| **Cloudflare Turnstile** | 🛡️ **Auto-Passes (Headless)** | ⚠️ Flaky | ❌ Blocked | ⚠️ Flaky | ❌ Blocked |
| **Mouse & Typing Physics** | 🧬 **Cubic Bézier + Human Typing** | ⚠️ Linear | ⚠️ Synthetic | ⚠️ Linear | ❌ None |
| **PEP 561 Type Stubs (`.pyi`)** | ✅ **100% Typed (`py.typed`)** | ⚠️ Partial | ✅ Typed | ⚠️ Partial | ✅ Typed |

---

## 🎯 Smart Locator Engine, Shadow DOM & Iframes

Every query method (`ele`, `eles`, `click`, `human_type`, `wait_for`, `text`, `text_all`, `attr`, `attr_all`, `count`, `exists`, `iframe`) shares one unified Smart Locator engine:

```python
# 1. Visible text (deepest matching element, traverses Shadow DOM automatically)
await tab.click("text:Sign In")
await tab.click("t:Confirm Order")

# 2. Attribute shorthand (@attr or @attr=value with automatic escaping)
await tab.human_type("@placeholder=Enter password", "s3cr3t_pass", clear=True)
await tab.click("@name=submit")
await tab.click("@data-testid=checkout-btn")

# 3. XPath (starts with //, (//, or .//)
await tab.click("//button[@type='submit']")
first_item = await tab.text("(//li[@class='result'])[1]")

# 4. Standard CSS + Automatic Deep Shadow DOM Piercing
# Elements nested inside open #shadow-root are located and clicked seamlessly
await tab.click("#button-inside-shadow-root")

# 5. Object-Oriented Element Chaining (zero DOM mutation via CDP backendNodeId)
cards = await tab.eles(".product-card")
for card in cards:
    title_el = await card.ele(".title")
    if title_el:
        print("Product:", title_el.text)
    print("URL:", await card.attr("href"))

# 6. Seamless Iframe Controller (same-origin, srcdoc, nested, and cross-origin OOPIF)
frame = await tab.iframe("#payment-iframe")
await frame.human_type("@name=cardnumber", "4242424242424242")
await frame.click("text:Pay Now")
```

---

## 🛡️ Anti-Bot & Stealth Benchmark (57/57 Headless)

Verified live against [bot.sannysoft.com](https://bot.sannysoft.com) in **headless mode (`headless=True`)** with **0 failures and 0 warnings**:

```text
========================================================================
TEST CATEGORY                         | RESULT                | STATUS
========================================================================
User Agent (Old) & HEADCHR_UA         | Chrome/157 (no Headl.)| PASSED
WebDriver (New) & WebDriver Advanced  | missing / passed      | PASSED
Chrome (New)                          | present (passed)      | PASSED
Permissions (New)                     | default               | PASSED
Plugins Length & PluginArray Type     | 5 / PluginArray       | PASSED
Languages & Hardware Concurrency      | en-US,en / 8 cores    | PASSED
WebGL Vendor & Renderer               | NVIDIA RTX 4070 D3D11 | PASSED
CHR_MEMORY & TRANSPARENT_PIXEL        | ok (0 warnings)       | PASSED
HEADCHR_* & PHANTOM_* Suite           | ok (0 failures)       | PASSED
SELENIUM_DRIVER & CHR_DEBUG_TOOLS     | ok                    | PASSED
Object.getOwnPropertyNames(navigator) | [] (0 leaked props)   | PASSED
MutationObserver during ele()/eles()  | [] (0 DOM mutations)  | PASSED
========================================================================
TOTAL HEADLESS BENCHMARK SCORE        | 57 / 57 PASSED        | 100%
========================================================================
```

### Six-Layer Evasion Architecture
1. **Headless UA & Sec-CH-UA Client Hints Sync (`Emulation.setUserAgentOverride`)**: Strips `HeadlessChrome/` from HTTP `User-Agent`, `navigator.userAgent`, and `navigator.userAgentData.brands` while retaining the real host Chrome version and OS platform.
2. **Pre-Execution Popup & OOPIF Auto-Attach (`Target.setAutoAttach`)**: Automatically attaches to newly spawned `window.open()` popups and cross-origin iframes while paused (`waitForDebuggerOnStart: true`), arms evasion + clean UA once (race-free via `tokio::sync::OnceCell`), then resumes unconditionally via `Runtime.runIfWaitingForDebugger`.
3. **Prototype-Only Native Patching**: Defines all overrides on real prototypes (`Navigator.prototype`, `Permissions.prototype`, `SpeechSynthesis.prototype`, `MediaDevices.prototype`) using non-constructable concise methods (`'prototype' in fn === false`) masked as `[native code]` — leaving `Object.getOwnPropertyNames(navigator)` completely empty (`[]`).
4. **Zero-DOM-Mutation Element Handles (`backendNodeId`)**: Never injects marker attributes into the page DOM where anti-bot `MutationObserver` hooks monitor mutations.
5. **Safe Canvas, Audio & WebGL Spoofing**: Spoofs discrete NVIDIA RTX 4070 WebGL strings, injects per-session canvas micro-noise via offscreen cloning without touching transparent probe pixels (`TRANSPARENT_PIXEL`) or mutating live canvases, and applies once-per-buffer AudioContext noise.
6. **Biological Input Physics**: Cubic Bézier cursor trajectories with acceleration/deceleration easing and sub-pixel tremors (`click`, `hover`, `mouse_move`), variable-cadence typing with punctuation pauses (`human_type`), and S-curve slider drag physics (`swipe`).

---

## 📖 Cookbook & Production Recipes

### 1. Sniffing Background AJAX / JSON API Responses (`wait_for_response`)
```python
import asyncio, json, best_driver

async def sniff_api():
    async with await best_driver.start(headless=True) as browser:
        tab = await browser.get("https://httpbin.org/html")

        # Start listening immediately (PyO3 future schedules on Tokio at call time)
        resp_fut = tab.wait_for_response("httpbin.org/json", timeout_ms=10000)
        await tab.evaluate("fetch('https://httpbin.org/json')")
        resp = await resp_fut

        print("HTTP Status:", resp["status"], resp["mime_type"])
        data = json.loads(resp["body"])
        print("Payload keys:", list(data.keys()))

asyncio.run(sniff_api())
```

### 2. Automated File Downloads (`wait_for_download`)
```python
import asyncio, best_driver

async def download_report():
    async with await best_driver.start(headless=True) as browser:
        tab = await browser.get("https://example.com/reports")

        dl_fut = tab.wait_for_download("./downloads", timeout_ms=30000)
        await tab.sleep(0.1)
        await tab.click("text:Export CSV")
        info = await dl_fut

        print(f"Saved {info['filename']} ({info['total_bytes']} bytes) -> {info['path']}")

asyncio.run(download_report())
```

### 3. Cloudflare Turnstile & Protected Forms
```python
import asyncio, best_driver

async def bypass_turnstile():
    async with await best_driver.start(headless=True) as browser:
        tab = await browser.get("https://nowsecure.nl", wait_until="networkidle")
        await tab.sleep(2)
        print("Page title:", await tab.title())

asyncio.run(bypass_turnstile())
```

### 4. Persistent Profiles, Session Cookies & Authenticated Proxies
```python
import asyncio, best_driver

async def persistent_proxy_session():
    # Graceful Browser.close flushes SQLite Cookies & Local Storage cleanly on exit
    async with await best_driver.start(
        headless=True,
        user_data_dir="./chrome_profile",
        proxy="http://user:password@proxy.example.com:8080",
    ) as browser:
        tab = await browser.get("https://example.com")
        await tab.set_cookie("session_id", "abc123")
        await tab.save_cookies("cookies.json")
        print("Browser alive:", await browser.is_alive())

asyncio.run(persistent_proxy_session())
```

### 5. Migrating from Selenium or Playwright to `best-driver`
```python
# Selenium (verbose, requires chromedriver.exe, detected by WAFs)
# driver.find_element(By.XPATH, "//input[@name='q']").send_keys("rust")

# Playwright (spawns Node.js server, detected by default)
# await page.locator("input[name='q']").fill("rust")

# best-driver (pure Rust CDP, 0 drivers, 57/57 stealth, Smart Locator + biological typing)
await tab.human_type("@name=q", "rust")
await tab.click("text:Search")
```

---

## 🛠️ API Reference

Full PEP 561 type definitions and IDE hover docstrings are bundled in [`python/best_driver/__init__.pyi`](https://github.com/real-coder-xD/best-driver/blob/main/python/best_driver/__init__.pyi).

### `best_driver.start(...)`
```python
browser = await best_driver.start(
    headless: bool = False,
    user_data_dir: Optional[str] = None,
    browser_path: Optional[str] = None,
    extra_args: Optional[Sequence[str]] = None,
    proxy: Optional[str] = None,  # "http://user:pass@host:port" or "socks5://host:port"
)
```

### `Browser` Methods & Properties
| Member | Return | Description |
| :--- | :--- | :--- |
| `browser.get(url, new_tab=False, wait_until="load")` | `Tab` | Navigate main tab (or spawn a new tab) to `url` |
| `browser.main_tab()` | `Tab` | Access the primary active tab |
| `browser.new_tab(url="about:blank")` | `Tab` | Create a new tab via `Target.createTarget` with pre-armed stealth |
| `browser.tabs()` | `list[Tab]` | List all open page tabs via `Target.getTargets` |
| `browser.is_alive()` | `bool` | Check whether the Chromium child process is still running |
| `browser.stop()` | `None` | Gracefully close Chromium (`Browser.close`) and reap cleanly |
| `browser.port`, `browser.ws_url`, `browser.user_data_dir` | `int` / `str` | Active loopback port, browser WebSocket URL, and profile path |

### `Tab` & `Element` Methods
| Category | Methods |
| :--- | :--- |
| **Smart Locators & `Element`** | `tab.ele(query)`, `tab.eles(query)` $\rightarrow$ `Element` (`el.tag`, `el.text`, `el.value`, `el.id`, `el.class_name`, `el.rect`, `el.backend_node_id`, `el.click()`, `el.hover()`, `el.human_type()`, `el.clear()`, `el.fill()`, `el.attr()`, `el.inner_html()`, `el.outer_html()`, `el.scroll_into_view()`, `el.screenshot()`, `el.ele()`, `el.eles()`) |
| **Iframes & Downloads** | `tab.iframe(selector, timeout_ms)`, `tab.frames()`, `tab.set_download_dir(dir)`, `tab.wait_for_download(dir, timeout_ms)` |
| **Navigation** | `get(url, timeout_ms, wait_until)`, `wait_for_load(state)`, `reload(ignore_cache)`, `back()`, `forward()`, `bring_to_front()`, `close()`, `url()`, `current_url()`, `title()`, `content()` |
| **Reading & Waiting** | `evaluate(expr)`, `exists(sel)`, `count(sel)`, `text(sel)`, `text_all(sel)`, `attr(sel, name)`, `attr_all(sel, name)`, `value(sel)`, `html(sel)`, `wait_for(sel, timeout_ms, visible)`, `wait_for_selector(...)`, `wait_for_hidden(sel)`, `wait_for_text(text)`, `wait_for_url(contains)`, `wait_for_function(expr)` |
| **Biological Input & Forms** | `click(sel)`, `double_click(sel)`, `right_click(sel)`, `click_text(text)`, `mouse_click(x, y)`, `mouse_move(x, y)`, `hover(sel)`, `drag(from, to)`, `swipe(x1, y1, x2, y2)`, `human_type(sel, text, clear)`, `send_keys(...)`, `type_text(...)`, `fill(sel, text)`, `press_key(key)`, `focus(sel)`, `clear(sel)`, `select_option(sel, val)`, `check(sel)`, `uncheck(sel)`, `upload_file(sel, files)` |
| **Network & Storage** | `wait_for_response(url_pattern, timeout_ms)`, `set_extra_headers(dict)`, `block_urls(patterns)`, `set_cache_disabled(bool)`, `set_cookie(name, val, domain, path)`, `cookies()`, `get_cookies()`, `set_cookies(list)`, `delete_cookies(name)`, `clear_cookies()`, `save_cookies(path)`, `load_cookies(path)`, `storage_get(key, session)`, `storage_set(k, v, session)`, `storage_clear(session)` |
| **Emulation, Dialogs & Capture** | `set_viewport(w, h, scale, mobile)`, `clear_viewport()`, `set_user_agent(ua, lang, platform)`, `set_timezone(tz)`, `set_locale(locale)`, `set_geolocation(lat, lon, acc)`, `auto_dialog(accept, prompt_text)`, `handle_dialog(...)`, `screenshot(path, full_page, format, quality, selector)`, `pdf(path, landscape)`, `scroll_down(px)`, `scroll_up(px)`, `scroll_to_bottom(...)`, `scroll_to_top()`, `scroll_into_view(sel)`, `sleep(sec)`, `send(method, params)` |

---

## 🏗️ Architecture Under the Hood

```text
┌────────────────────────────────────────────────────────┐
│              Python Application (PEP 561)              │
│       await browser.get("https://...")  + .pyi         │
└───────────────────────────┬────────────────────────────┘
                            │ PyO3 0.30 / Zero json.dumps FFI
┌───────────────────────────▼────────────────────────────┐
│              Rust Core (best_driver.cdylib)            │
│  ┌──────────────────────────────────────────────────┐  │
│  │           Tokio Asynchronous Runtime             │  │
│  │  - Single Browser-Level WebSocket (Zero reqwest) │  │
│  │  - Flattened SessionId Multiplexer (Target API)  │  │
│  │  - OnceCell Auto-Attach Stealth (Popups & OOPIF) │  │
│  │  - Native CDP backendNodeId Element Handles      │  │
│  │  - Cubic Bezier Mouse & Biological Typing Engine │  │
│  └──────────────────────────┬───────────────────────┘  │
└─────────────────────────────┼──────────────────────────┘
                              │ Single CDP WebSocket (flatten: true)
┌─────────────────────────────▼──────────────────────────┐
│                   Chromium Engine                      │
│   (Chrome / Brave / Edge / Headless Chromium)          │
│   - Target.attachToTarget + Target.setAutoAttach       │
│   - Page.addScriptToEvaluateOnNewDocument (Evasion)    │
│   - Input / Network / Fetch / DOM / Emulation Domains  │
└────────────────────────────────────────────────────────┘
```

---

## ❓ Frequently Asked Questions (FAQ)

### What is the fastest undetected alternative to Selenium and Playwright in Python?
`best-driver` runs its entire CDP WebSocket multiplexer, JSON-RPC framing, and Base64 decoder natively in Rust + Tokio outside the Python GIL. Unlike Selenium (which requires `chromedriver.exe` over HTTP) and Playwright (which runs a separate Node.js server), `best-driver` connects over a single flattened browser-level WebSocket and passes **57/57 Sannysoft tests in headless mode** out of the box.

### How does `best-driver` compare to DrissionPage, nodriver, and undetected-chromedriver?
`best-driver` combines **DrissionPage's Smart Locator ergonomics** (`text:`, `@attr=val`, XPath, CSS, Shadow DOM piercing, `tab.ele()`, `tab.iframe()`, `tab.wait_for_download()`) with a **native async Rust engine** that avoids Python GIL contention at 50+ concurrent tabs and adds deeper anti-detection (prototype-only patching, `HeadlessChrome` UA/Sec-CH-UA stripping, zero-DOM-mutation `backendNodeId` handles, auto-attach popup/OOPIF stealth, and Cubic Bézier cursor physics).

### Does `best-driver` leak `HeadlessChrome` when running with `headless=True`?
No. On launch, `best-driver` inspects `Browser.getVersion` and automatically calls `Emulation.setUserAgentOverride` with synchronized `userAgent` and `userAgentMetadata` (`Sec-CH-UA` brands and full version list), keeping the real installed Chrome version and host OS platform intact while removing `HeadlessChrome`.

### Does `best-driver` mutate the DOM when locating elements?
No. `tab.ele()` and `tab.eles()` resolve Chromium's internal `backendNodeId` via `DOM.describeNode` and operate through `DOM.resolveNode` + `Runtime.callFunctionOn` (releasing every V8 `objectId` on all code paths). Page `MutationObserver` detectors record zero attribute mutations.

### Does `best-driver` support Shadow DOM, cross-origin iframes, downloads, and AJAX sniffing?
Yes. Open `#shadow-root` elements are pierced automatically across all Smart Locator methods; `tab.iframe(selector)` controls same-origin, nested, and cross-origin OOPIF iframes with automatic viewport coordinate translation; `tab.wait_for_download(dir)` captures downloaded files; and `tab.wait_for_response(pattern)` awaits `Network.loadingFinished` and returns decoded HTTP response bodies.

---

<details>
<summary><b>🇻🇳 Tổng Quan Tiếng Việt — Thư viện tự động hoá trình duyệt Python chống phát hiện (Click để mở rộng)</b></summary>
<br/>

**`best-driver`** là thư viện **tự động hoá trình duyệt Chromium và cào dữ liệu web chống anti-bot cho Python**, viết lõi bằng **Rust (PyO3 + Tokio)**, thay thế hiệu năng cao cho **Selenium, Playwright, DrissionPage, nodriver và undetected-chromedriver**.

### Vì sao nên dùng `best-driver`?
1. **Không cần `chromedriver.exe`, không cần Node.js**: Kết nối thẳng vào Chromium qua **1 WebSocket duy nhất** ở tầng Browser bằng chuẩn Flattened Session (`Target.attachToTarget`), bỏ hoàn toàn `reqwest`/OpenSSL, cài đặt gọn qua `pip install best-driver`.
2. **Smart Locator & Element Chaining chuẩn DrissionPage**:
   - Một cú pháp duy nhất tự nhận diện **XPath** (`//button`), **chữ hiển thị** (`text:Đăng nhập`, `t:Xác nhận`), **thuộc tính** (`@name=username`, `@placeholder=Tìm kiếm`) và **CSS**.
   - **Tự động xuyên thấu Shadow DOM** và hỗ trợ điều khiển **Iframe lồng nhau / cross-origin** qua `tab.iframe()`, bắt file tải về qua `tab.wait_for_download()`, bắt gói tin API JSON ngầm qua `tab.wait_for_response()`.
   - Dùng `backendNodeId` gốc của Chromium nên **không chèn thuộc tính rác vào DOM** (vô hình trước `MutationObserver`).
3. **Ẩn danh thực chiến — 57/57 Sannysoft Headless (0 Fail, 0 Warn) & Vượt Cloudflare Turnstile**:
   - Tự động gỡ `HeadlessChrome` khỏi cả HTTP `User-Agent`, `navigator.userAgent` và `Sec-CH-UA` client hints.
   - Tự động tiêm lớp ẩn danh vào cả **popup (`window.open`) và cross-origin iframe** *trước khi JS trang kịp chạy* (`Target.setAutoAttach`).
   - Patch thuần prototype (`Object.getOwnPropertyNames(navigator) == []`), giả lập WebGL NVIDIA RTX 4070, vi nhiễu Canvas/Audio an toàn không làm hỏng `TRANSPARENT_PIXEL`, chuột cong Cubic Bézier và gõ phím sinh học (`human_type`, `swipe`).
4. **Hiệu năng Rust thoát GIL & Hỗ trợ Autocomplete 100%**: Chạy song song hàng chục tab tốn cực ít RAM, hỗ trợ đầy đủ gợi ý code PEP 561 (`.pyi` + `py.typed` 117/117 hàm) trên VSCode và PyCharm.

</details>

---

## 📄 Changelog & License

- See [CHANGELOG.md](https://github.com/real-coder-xD/best-driver/blob/main/CHANGELOG.md) for full release history.
- Distributed under the **MIT License** — see [LICENSE](https://github.com/real-coder-xD/best-driver/blob/main/LICENSE).

<div align="center">

**If `best-driver` speeds up your scraping or automation pipelines, consider dropping a ⭐ on [GitHub](https://github.com/real-coder-xD/best-driver)!**

</div>
