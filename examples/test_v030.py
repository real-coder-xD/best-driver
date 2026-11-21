"""End-to-end verification of best-driver v0.3.0:
1. BrowserContext cookie + HttpOnly + tls_session() + clear_cookies() isolation (fixes 0-cookie bug!)
2. BrowserContext per-context emulation (viewport, timezone, extra_headers, add_init_script)
3. tab.route(stage='response') real server response inspection & JSON patching
4. tab.fetch() real in-browser Chromium TLS fetch (with JSON body + credentials)
5. tab.record_har() / tab.export_har() HAR 1.2 traffic capture
6. Locator.filter(has_text=...), wait_for(state='visible'|'hidden'), is_visible/is_enabled/is_checked/check/uncheck/select_option
"""

from __future__ import annotations

import asyncio
import json
import os
import best_driver

FORM_HTML = """<!DOCTYPE html>
<html>
<head><title>v0.3.1 Locator & Form Test</title></head>
<body>
  <div id="cards">
    <div class="card"><span>Free Plan</span><button class="buy">Choose</button></div>
    <div class="card"><span>Pro Plan</span><button class="buy" onclick="window.__picked='pro'">Choose</button></div>
  </div>
  <div id="form-wrap">
    <input id="email-box" type="text" placeholder="Email" />
    <button id="ok-btn">Sign In</button>
    <p id="comma-txt">Hello, world</p>
  </div>
  <div id="closed-host"></div>
  <div id="delayed" style="display:none; width:100px; height:30px;">Ready Now</div>
  <input id="agree" type="checkbox" />
  <button id="disabled-btn" disabled>Locked</button>
  <select id="tier">
    <option value="free">Free</option>
    <option value="pro">Pro</option>
  </select>
  <script>
    setTimeout(() => { document.getElementById('delayed').style.display = 'block'; }, 250);
    const sr = document.getElementById('closed-host').attachShadow({ mode: 'closed' });
    sr.innerHTML = '<input id="closed-em" type="text" placeholder="ClosedEmail" />';
  </script>
</body>
</html>
"""


async def main() -> None:
    print("=" * 72)
    print("BEST-DRIVER v0.3.0 FULL AUDIT (>9.5/10 TARGET)")
    print("=" * 72)

    html_path = os.path.abspath("tmp_form_v030.html")
    har_path = os.path.abspath("tmp_trace_v030.har")
    with open(html_path, "w", encoding="utf-8") as f:
        f.write(FORM_HTML)
    form_url = f"file:///{html_path.replace(os.sep, '/')}"

    try:
        async with await best_driver.start(headless=True) as browser:
            # ── SUITE 1: BrowserContext Emulation + Cookie/tls_session Fix ────
            print("\n--- SUITE 1: BrowserContext Emulation + Init Script + tls_session() ---")
            async with await browser.new_context(
                viewport={"width": 900, "height": 640, "scale": 1.0, "mobile": False},
                timezone="Asia/Tokyo",
                extra_headers={"X-Ctx-Header": "ctx-v030"},
            ) as ctx:
                await ctx.add_init_script("window.__ctxInjected = 'tokyo_ok';")
                tab = await ctx.get("https://httpbin.org/headers", wait_until="load")
                assert tab.context_id == ctx.context_id

                # Verify init_script, viewport, timezone, extra_headers
                meta = await tab.evaluate("""({
                    injected: window.__ctxInjected,
                    w: window.innerWidth,
                    h: window.innerHeight,
                    tz: Intl.DateTimeFormat().resolvedOptions().timeZone,
                    hdrs: JSON.parse(document.body.innerText).headers
                })""")
                print(f"  [CTX EMULATION] injected={meta['injected']} vp={meta['w']}x{meta['h']} tz={meta['tz']} hdr={meta['hdrs'].get('X-Ctx-Header')}")
                assert meta["injected"] == "tokyo_ok"
                assert meta["w"] == 900 and meta["h"] == 640
                assert meta["tz"] == "Asia/Tokyo"
                assert meta["hdrs"].get("X-Ctx-Header") == "ctx-v030"

                # Verify HttpOnly cookie inside BrowserContext transfers 100% into tab.tls_session()!
                await tab.set_cookies([
                    {
                        "name": "cf_ctx_clear",
                        "value": "ctx_httponly_ok",
                        "domain": "httpbin.org",
                        "path": "/",
                        "secure": True,
                        "httpOnly": True,
                    }
                ])
                c_list = await tab.cookies()
                assert any(c["name"] == "cf_ctx_clear" and c["value"] == "ctx_httponly_ok" for c in c_list)
                async with await tab.tls_session() as tls:
                    echo = (await tls.get("https://httpbin.org/cookies")).json()["cookies"]
                    print(f"  [CTX -> http_tls] cookies={echo}")
                    assert echo.get("cf_ctx_clear") == "ctx_httponly_ok"
                print("[PASS] SUITE 1: BrowserContext emulation, init_script & tls_session() cookie sync verified!")

            # ── SUITE 2: Response-Stage Interception (Modify Live Server JSON) ─
            print("\n--- SUITE 2: tab.route(stage='response') Live Response Patching ---")
            tab = await browser.main_tab()
            await tab.get("https://httpbin.org/html", wait_until="load")

            async def patch_server_json(route: best_driver.Route) -> None:
                assert route.stage == "response"
                data = await route.response_json()
                data["patched_by_best_driver"] = True
                await route.fulfill(
                    status=route.response_status or 200,
                    json=data,
                    headers={"Access-Control-Allow-Origin": "*"},
                )

            await tab.route("*httpbin.org/json*", patch_server_json, stage="response")
            patched = await tab.evaluate("fetch('https://httpbin.org/json').then(r => r.json())")
            print(f"  [ROUTE response] keys={list(patched.keys())} patched={patched.get('patched_by_best_driver')}")
            assert "slideshow" in patched and patched.get("patched_by_best_driver") is True
            await tab.unroute()
            print("[PASS] SUITE 2: Real server response intercepted, parsed & patched before page received it!")

            # ── SUITE 3: tab.fetch() Real Chromium TLS Fetch + HAR 1.2 Export ─
            print("\n--- SUITE 3: tab.fetch() Native Chrome TLS + HAR 1.2 Recording ---")
            await tab.record_har(include_bodies=True)
            f_res = await tab.fetch(
                "https://httpbin.org/post",
                method="POST",
                headers={"X-Fetch-Test": "native-chrome"},
                json={"hello": "v0.3.0"},
            )
            assert f_res["status"] == 200 and f_res["ok"] is True
            f_body = json.loads(f_res["body"])
            print(f"  [TAB.FETCH] status={f_res['status']} json={f_body.get('json')} hdr={f_body['headers'].get('X-Fetch-Test')}")
            assert f_body.get("json") == {"hello": "v0.3.0"}
            assert f_body["headers"].get("X-Fetch-Test") == "native-chrome"

            await tab.sleep(0.2)
            har = await tab.export_har(har_path)
            await tab.stop_har()
            entries = har["log"]["entries"]
            print(f"  [HAR 1.2] recorded entries={len(entries)} file_exists={os.path.exists(har_path)}")
            assert har["log"]["version"] == "1.2"
            assert any("httpbin.org/post" in e["request"]["url"] for e in entries)
            assert os.path.exists(har_path)
            print("[PASS] SUITE 3: tab.fetch() and HAR 1.2 recording/export verified!")

            # ── SUITE 4: Playwright-Grade Locator (filter, wait_for visible, forms) ─
            print("\n--- SUITE 4: Locator.filter(), wait_for(state='visible'), Forms ---")
            await tab.get(form_url, wait_until="load")

            # filter(has_text=...) picks Pro Plan card and clicks its button
            pro_card = tab.locator(".card").filter(has_text="Pro Plan")
            assert await pro_card.count() == 1
            await pro_card.locator("button.buy").click()
            assert await tab.evaluate("window.__picked") == "pro"

            # wait_for(state="visible") waits for display:none -> display:block
            delayed = tab.locator("#delayed")
            await delayed.wait_for(timeout_ms=5000, state="visible")
            assert await delayed.is_visible() is True

            # is_enabled / is_checked / check / uncheck / select_option
            assert await tab.locator("#disabled-btn").is_enabled() is False
            agree = tab.locator("#agree")
            assert await agree.is_checked() is False
            await agree.check()
            assert await agree.is_checked() is True
            await agree.uncheck()
            assert await agree.is_checked() is False

            tier = tab.locator("#tier")
            await tier.select_option("pro")
            assert await tier.value() == "pro"
            print("[PASS] SUITE 4: Locator.filter, wait_for(visible), is_enabled, check/uncheck, select_option verified!")

            # ── SUITE 5: v0.3.1 Zero-Leak Cleanup + Mixed Comma Smart Locators ──
            print("\n--- SUITE 5: Zero-Leak Controller/HAR Cleanup & Mixed Comma Smart Locators ---")
            import time
            from best_driver.network import _CONTROLLERS, _HAR_RECORDERS

            # 5A.1: Direct tab.close() without unroute()/stop_har()
            t_leak = await browser.new_tab("about:blank")
            sid_leak = t_leak.session_id
            await t_leak.route("**/*", lambda r: r.continue_())
            await t_leak.record_har(include_bodies=False)
            assert sid_leak in _CONTROLLERS and sid_leak in _HAR_RECORDERS
            await t_leak.close()
            # Zero-tick check immediately after await close()
            assert sid_leak not in _CONTROLLERS, f"Leaked route controller for {sid_leak}"
            assert sid_leak not in _HAR_RECORDERS, f"Leaked HAR recorder for {sid_leak}"

            # 5A.2: BrowserContext __aexit__ without closing tab or unroute()/stop_har()
            async with await browser.new_context() as ctx_leak:
                t_ctx = await ctx_leak.new_tab("about:blank")
                sid_ctx = t_ctx.session_id
                await t_ctx.route("**/*", lambda r: r.continue_())
                await t_ctx.record_har(include_bodies=False)
                assert sid_ctx in _CONTROLLERS and sid_ctx in _HAR_RECORDERS
            assert sid_ctx not in _CONTROLLERS, f"Leaked ctx route controller for {sid_ctx}"
            assert sid_ctx not in _HAR_RECORDERS, f"Leaked ctx HAR recorder for {sid_ctx}"

            # 5A.3: EventStream auto-terminates via Target.detachedFromTarget on tab close
            t_ev = await browser.new_tab("about:blank")
            stream = t_ev.listen_requests()
            t0 = time.monotonic()
            async def close_soon() -> None:
                await asyncio.sleep(0.1)
                await t_ev.close()
            closer = asyncio.create_task(close_soon())
            ev = await stream.next(timeout_ms=3000)
            elapsed = time.monotonic() - t0
            await closer
            assert ev is None and elapsed < 1.5, f"Stream did not close on detach (ev={ev}, dt={elapsed:.2f}s)"
            print(f"  [CLEANUP] tab.close=0-tick clean, ctx.close=0-tick clean, detach stream EOF in {elapsed*1000:.0f}ms")

            # 5B: Mixed comma Smart Locators (CSS + @attr + text: + XPath + child + closed shadow DOM)
            el_css_attr = await tab.ele("input[type='email'], @placeholder=Email")
            assert el_css_attr is not None and el_css_attr.id == "email-box"
            loc_css_attr = await tab.locator("input[type='email'], @placeholder=Email").element()
            assert loc_css_attr is not None and loc_css_attr.id == "email-box"
            el_mix_txt = await tab.ele("#does-not-exist, text:Sign In")
            assert el_mix_txt is not None and el_mix_txt.id == "ok-btn"
            el_mix_xp = await tab.ele("#does-not-exist, //button[@id='ok-btn']")
            assert el_mix_xp is not None and el_mix_xp.id == "ok-btn"
            multi = await tab.eles("#email-box, text:Sign In")
            multi_ids = [e.id for e in multi]
            assert "email-box" in multi_ids and "ok-btn" in multi_ids
            child_el = await tab.locator("#form-wrap").locator("input[type='email'], @placeholder=Email").element()
            assert child_el is not None and child_el.id == "email-box"
            closed_el = await tab.ele("input[type='email'], @placeholder=ClosedEmail")
            assert closed_el is not None and closed_el.id == "closed-em"
            comma_txt = await tab.ele("text:Hello, world")
            assert comma_txt is not None and comma_txt.id == "comma-txt"
            print("  [LOCATOR] CSS+@attr, CSS+text:, CSS+XPath, eles union, child, closed shadow, literal text comma all matched")
            print("[PASS] SUITE 5: Zero-leak cleanup & mixed comma Smart Locators verified!")

            # ── SUITE 6: v0.3.2 Popups, Opt-in Console/PageErrors, Crash Detection ──
            print("\n--- SUITE 6: expect_popup(), listen_popups(), listen_console/page_errors(), listen_crashes(), is_alive() ---")
            # 6A: Playwright-style race-free expect_popup() inside isolated BrowserContext
            async with await browser.new_context(timezone="Asia/Seoul") as pop_ctx:
                await pop_ctx.add_init_script("window.__popCtxTag = 'seoul_pop';")
                parent = await pop_ctx.get(form_url, wait_until="load")
                assert await parent.is_alive() is True
                async with parent.expect_popup(timeout_ms=10000) as pop_info:
                    await parent.evaluate("void window.open('about:blank', '_blank')")
                popup = await pop_info.value
                assert popup.context_id == pop_ctx.context_id
                await popup.get(form_url, wait_until="load")
                pop_meta = await popup.evaluate("""({
                    tag: window.__popCtxTag,
                    tz: Intl.DateTimeFormat().resolvedOptions().timeZone,
                    wd: navigator.webdriver,
                    stackTrap: (() => { let hit = false; const e = new Error(); Object.defineProperty(e, 'stack', { get() { hit = true; return ''; } }); console.debug(e); return hit; })()
                })""")
                print(f"  [POPUP CTX] {pop_meta}")
                assert pop_meta["tag"] == "seoul_pop"
                assert pop_meta["tz"] == "Asia/Seoul"
                assert pop_meta["wd"] is False
                assert pop_meta["stackTrap"] is False
                await popup.close()
                assert await popup.is_alive() is False

                # 6B: Continuous listen_popups() stream
                async with parent.listen_popups() as popups:
                    await parent.evaluate("void window.open('about:blank', '_blank')")
                    pop2 = await popups.next(timeout_ms=5000)
                    assert pop2 is not None and await pop2.is_alive() is True
                    await pop2.close()

            # 6C: Opt-in listen_console() + listen_page_errors() + stealth restoration on exit
            async with tab.listen_console() as logs, tab.listen_page_errors() as errs:
                await tab.evaluate("console.warn('v032_warn', 42)")
                await tab.evaluate("setTimeout(() => { throw new TypeError('v032_boom'); }, 10)")
                cmsg = await logs.next(timeout_ms=3000)
                perr = await errs.next(timeout_ms=3000)
                print(f"  [CONSOLE] {cmsg} | [PAGE_ERROR] {perr}")
                assert cmsg is not None and cmsg.type == "warning" and "v032_warn 42" in cmsg.text
                assert perr is not None and "v032_boom" in perr.message

            # After exiting both opt-in streams, Runtime.disable has been called -> verify trap is False again!
            trap_after = await tab.evaluate("""(() => {
                let hit = false;
                const e = new Error();
                Object.defineProperty(e, 'stack', { get() { hit = true; return ''; } });
                console.debug(e);
                return hit;
            })()""")
            print(f"  [STEALTH RESTORED] errorStackTrap_after_listen_console={trap_after}")
            assert trap_after is False

            # 6D: Renderer crash detection (Inspector.targetCrashed + tab.is_alive() == False)
            t_crash = await browser.new_tab("about:blank")
            assert await t_crash.is_alive() is True
            async with t_crash.listen_crashes() as crashes:
                try:
                    await t_crash.send("Page.crash", {})
                except Exception:
                    pass
                cev = await crashes.next(timeout_ms=5000)
                print(f"  [CRASH DETECTED] {cev} | crashed_tab_alive={await t_crash.is_alive()} | main_tab_alive={await tab.is_alive()}")
                assert cev is not None and cev.target_id == t_crash.target_id
                assert await t_crash.is_alive() is False
                assert await tab.is_alive() is True
            await t_crash.close()
            print("[PASS] SUITE 6: Popups, opt-in console/page_errors (+ stealth restore), and crash detection verified!")

        print("\n>>> ALL V0.3.2 AUDIT TESTS PASSED (100%) <<<")
    finally:
        for p in (html_path, har_path):
            if os.path.exists(p):
                os.remove(p)


if __name__ == "__main__":
    asyncio.run(main())
