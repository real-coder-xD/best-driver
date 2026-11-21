"""End-to-end verification of best-driver v0.2.0:
1. BrowserContext cookie/localStorage isolation & concurrent execution in 1 Chromium process
2. tab.route() request mocking (fulfill JSON), header injection (continue_), and aborting
3. tab.listen_responses() & tab.listen_requests() live async network streams
4. tab.locator() lazy auto-retrying chained locator across SPA DOM re-renders & closed Shadow DOM
"""

from __future__ import annotations

import asyncio
import os
import best_driver

SPA_HTML = """<!DOCTYPE html>
<html>
<head><title>v0.2.0 Locator & Network Test</title></head>
<body>
  <div id="spa-root"></div>
  <div id="closed-host"></div>
  <div id="log">idle</div>
  <script>
    window.__spaClicks = 0;
    window.__closedClicked = false;

    // Simulate React/Next.js re-renders replacing DOM nodes (forces stale backendNodeId)
    let gen = 0;
    window.renderSpa = function renderSpa() {
      gen++;
      const root = document.getElementById('spa-root');
      root.innerHTML =
        '<div class="card"><span>Card 0</span><button class="act">Btn 0</button></div>' +
        '<div class="card"><span>Card 1 (gen ' + gen + ')</span><button class="act" onclick="window.__spaClicks++; document.getElementById(\\'log\\').innerText=\\'clicked:\\'+window.__spaClicks">Target Action</button></div>';
    };
    window.renderSpa();

    // Closed shadow root host
    const ch = document.getElementById('closed-host');
    const cr = ch.attachShadow({ mode: 'closed' });
    cr.innerHTML = '<button id="closed-btn" onclick="window.__closedClicked = true">Closed Action</button>';
  </script>
</body>
</html>
"""


async def main() -> None:
    print("=" * 70)
    print("BEST-DRIVER v0.2.0 AUDIT: BrowserContext, Route, Streams & Locator")
    print("=" * 70)

    spa_path = os.path.abspath("tmp_spa_v020.html")
    with open(spa_path, "w", encoding="utf-8") as f:
        f.write(SPA_HTML)
    spa_url = f"file:///{spa_path.replace(os.sep, '/')}"

    try:
        async with await best_driver.start(headless=True) as browser:
            # ── SUITE 1: Isolated BrowserContext Concurrency ──────────────────
            print("\n--- SUITE 1: Isolated BrowserContext (Cookies + localStorage) ---")
            async with await browser.new_context() as ctx_a, await browser.new_context() as ctx_b:
                assert ctx_a.context_id != ctx_b.context_id
                tab_a, tab_b = await asyncio.gather(
                    ctx_a.get("https://httpbin.org/cookies", wait_until="load"),
                    ctx_b.get("https://httpbin.org/cookies", wait_until="load"),
                )
                await asyncio.gather(
                    tab_a.set_cookie("owner", "context_A", domain="httpbin.org", path="/"),
                    tab_b.set_cookie("owner", "context_B", domain="httpbin.org", path="/"),
                    tab_a.storage_set("who", "A"),
                    tab_b.storage_set("who", "B"),
                )
                await asyncio.gather(
                    tab_a.get("https://httpbin.org/cookies", wait_until="load"),
                    tab_b.get("https://httpbin.org/cookies", wait_until="load"),
                )
                ca = (await tab_a.evaluate("JSON.parse(document.body.innerText)"))["cookies"]
                cb = (await tab_b.evaluate("JSON.parse(document.body.innerText)"))["cookies"]
                la = await tab_a.storage_get("who")
                lb = await tab_b.storage_get("who")
                print(f"  [CTX A] cookies={ca} localStorage.who={la}")
                print(f"  [CTX B] cookies={cb} localStorage.who={lb}")
                assert ca == {"owner": "context_A"}, f"Context A leaked: {ca}"
                assert cb == {"owner": "context_B"}, f"Context B leaked: {cb}"
                assert la == "A" and lb == "B"
                assert len(await ctx_a.tabs()) == 1 and len(await ctx_b.tabs()) == 1

                # Regression test: tab.cookies(), tab.storage_state(), and tab.tls_session() INSIDE BrowserContext!
                await tab_a.set_cookies([
                    {
                        "name": "ctx_a_httponly",
                        "value": "secret_alpha_99",
                        "domain": "httpbin.org",
                        "path": "/",
                        "secure": True,
                        "httpOnly": True,
                    }
                ])
                raw_ca = await tab_a.cookies()
                raw_cb = await tab_b.cookies()
                names_a = {c["name"]: c["value"] for c in raw_ca}
                names_b = {c["name"]: c["value"] for c in raw_cb}
                print(f"  [CTX A tab.cookies()] {names_a}")
                print(f"  [CTX B tab.cookies()] {names_b}")
                assert names_a.get("owner") == "context_A"
                assert names_a.get("ctx_a_httponly") == "secret_alpha_99"
                assert "ctx_a_httponly" not in names_b and names_b.get("owner") == "context_B"

                # Verify tab_a.tls_session() inside BrowserContext carries Context A's cookies (incl. HttpOnly)!
                async with await tab_a.tls_session() as tls_a:
                    echo_a = (await tls_a.get("https://httpbin.org/cookies")).json()["cookies"]
                    print(f"  [CTX A -> http_tls] {echo_a}")
                    assert echo_a.get("owner") == "context_A"
                    assert echo_a.get("ctx_a_httponly") == "secret_alpha_99"
                async with await tab_b.tls_session() as tls_b:
                    echo_b = (await tls_b.get("https://httpbin.org/cookies")).json()["cookies"]
                    print(f"  [CTX B -> http_tls] {echo_b}")
                    assert echo_b == {"owner": "context_B"}

                # Verify clear_cookies() inside Context A does NOT wipe Context B!
                await tab_a.clear_cookies()
                assert await tab_a.cookies() == []
                assert any(c["name"] == "owner" and c["value"] == "context_B" for c in await tab_b.cookies())
            assert await browser.is_alive() is True
            print("[PASS] SUITE 1: 2 isolated BrowserContexts + tab.cookies() + tls_session() + clear_cookies() verified!")

            # ── SUITE 2: tab.route() Mock JSON, Header Inject, Abort ──────────
            print("\n--- SUITE 2: tab.route() Intercept / Mock / Modify / Abort ---")
            tab = await browser.main_tab()
            await tab.get("https://httpbin.org/html", wait_until="load")

            # 2a. Mock JSON response via async handler
            async def mock_api(route: best_driver.Route) -> None:
                await route.fulfill(
                    status=200,
                    json={"mocked": True, "engine": "best-driver-v0.2.0"},
                    headers={"Access-Control-Allow-Origin": "*"},
                )

            # 2b. Modify outgoing request headers via sync-or-async handler
            async def inject_header(route: best_driver.Route) -> None:
                hdrs = dict(route.headers)
                hdrs["X-Best-Driver-Route"] = "intercepted-ok"
                await route.continue_(headers=hdrs)

            # 2c. Abort blocked asset
            async def abort_blocked(route: best_driver.Route) -> None:
                await route.abort("BlockedByClient")

            await tab.route("*/api/mocked*", mock_api)
            await tab.route("*httpbin.org/headers*", inject_header)
            await tab.route("*.blocked-asset*", abort_blocked)

            mocked_res = await tab.evaluate(
                "fetch('https://httpbin.org/api/mocked').then(r => r.json())"
            )
            print(f"  [ROUTE fulfill] -> {mocked_res}")
            assert mocked_res == {"mocked": True, "engine": "best-driver-v0.2.0"}

            hdr_res = await tab.evaluate(
                "fetch('https://httpbin.org/headers').then(r => r.json())"
            )
            echoed = hdr_res.get("headers", {}).get("X-Best-Driver-Route")
            print(f"  [ROUTE continue_ headers] X-Best-Driver-Route={echoed!r}")
            assert echoed == "intercepted-ok"

            aborted_err = await tab.evaluate(
                "fetch('https://httpbin.org/foo.blocked-asset').then(() => 'OK').catch(e => String(e))"
            )
            print(f"  [ROUTE abort] -> {aborted_err}")
            assert "Failed to fetch" in aborted_err or "TypeError" in aborted_err

            await tab.unroute()
            print("[PASS] SUITE 2: tab.route() fulfill / header modify / abort / unroute verified!")

            # ── SUITE 3: Live Network Streams (listen_requests & listen_responses) ─
            print("\n--- SUITE 3: tab.listen_requests() & tab.listen_responses() ---")
            async with tab.listen_requests("*httpbin.org/json*") as req_stream, \
                       tab.listen_responses("*httpbin.org/json*") as resp_stream:
                await tab.evaluate("void fetch('https://httpbin.org/json')")
                req_ev = await req_stream.next(timeout_ms=10000)
                resp_ev = await resp_stream.next(timeout_ms=10000)
                assert req_ev is not None and req_ev.method == "GET"
                assert resp_ev is not None and resp_ev.status == 200
                body_json = await resp_ev.json()
                print(f"  [STREAM req ] {req_ev}")
                print(f"  [STREAM resp] {resp_ev} | keys={list(body_json.keys())}")
                assert "slideshow" in body_json
            print("[PASS] SUITE 3: Live async request & response streams verified!")

            # ── SUITE 4: Lazy Auto-Retrying Locator on Re-Rendering SPA & Closed Shadow ─
            print("\n--- SUITE 4: tab.locator() Lazy Auto-Retry (SPA + Closed Shadow) ---")
            await tab.get(spa_url, wait_until="load")
            target_btn = tab.locator("#spa-root").locator(".card").nth(1).locator("button.act")
            assert await tab.locator("#spa-root").locator(".card").count() == 2
            await target_btn.click()
            # Force full SPA DOM replacement — any snapshot Element handle is now detached/stale
            await tab.evaluate("window.renderSpa()")
            await target_btn.click()
            clicks = await tab.evaluate("window.__spaClicks")
            print(f"  [LOCATOR SPA] 2 clicks across full DOM replacement -> clicks={clicks}")
            assert clicks == 2

            closed_loc = tab.locator("#closed-host").locator("#closed-btn")
            assert await closed_loc.exists() is True
            await closed_loc.click()
            assert await tab.evaluate("window.__closedClicked") is True
            print("  [LOCATOR CLOSED SHADOW] pierced & clicked cleanly!")
            print("[PASS] SUITE 4: Lazy auto-retrying chained Locator verified!")

        print("\n>>> ALL V0.2.0 AUDIT TESTS PASSED (100%) <<<")
    finally:
        if os.path.exists(spa_path):
            os.remove(spa_path)


if __name__ == "__main__":
    asyncio.run(main())
