import asyncio
import os
import best_driver

HTML_TEST_PAGE = """<!DOCTYPE html>
<html>
<head>
    <title>Best Driver Upgrade Verification</title>
</head>
<body>
    <div id="app">
        <h1 id="main-title">Best Driver v0.1.3</h1>
        <div id="container">
            <ul id="item-list">
                <li class="item" data-code="A1">Alpha Item</li>
                <li class="item" data-code="B2">Beta Item</li>
                <li class="item" data-code="C3">Gamma Item</li>
            </ul>
        </div>
        <form id="sample-form">
            <input type="text" id="username" placeholder="Enter username" />
            <button type="button" id="submit-btn" onclick="document.getElementById('status').innerText = 'Submitted: ' + document.getElementById('username').value;">Submit</button>
        </form>
        <div id="status">Pending</div>
        <div id="shadow-host"></div>
        <div id="closed-shadow-host"></div>
        <iframe id="demo-frame" style="width:320px;height:120px;border:2px solid #333;" srcdoc="<!DOCTYPE html><html><body><h2 id='frame-title'>Inside Iframe</h2><input id='frame-input' /><button id='frame-btn' onclick=&quot;document.getElementById('frame-out').innerText='Frame: '+document.getElementById('frame-input').value&quot;>Click Frame</button><div id='frame-out'>Idle</div></body></html>"></iframe>
        <a id="dl-link" download="best_driver_test.txt" href="data:text/plain;charset=utf-8,hello%20best%20driver">Download File</a>
    </div>
    <script>
        const host = document.getElementById('shadow-host');
        const root = host.attachShadow({ mode: 'open' });
        root.innerHTML = '<div class="shadow-inner"><span class="shadow-text">Inside Shadow Root</span><button id="shadow-btn" onclick="this.innerText=\\'Clicked Inside Shadow\\'">Click Shadow</button></div>';

        // Closed shadow root (mode: 'closed') with checkbox + nested iframe
        window.__closedClicked = false;
        const closedHost = document.getElementById('closed-shadow-host');
        const closedRoot = closedHost.attachShadow({ mode: 'closed' });
        closedRoot.innerHTML = '<label class="cf-label"><input id="closed-cb" type="checkbox" onclick="window.__closedClicked = true" />Verify you are human</label><iframe id="closed-iframe" style="width:200px;height:60px;" srcdoc="<!DOCTYPE html><html><body><span id=\\x27closed-frame-msg\\x27>Inside Closed Shadow Iframe</span></body></html>"></iframe>';

        // Watch for ANY unexpected attribute mutations from locator queries (anti-bot MutationObserver probe)
        window.__attrMutations = [];
        new MutationObserver((records) => {
            for (const r of records) {
                if (r.type === 'attributes') window.__attrMutations.push(r.attributeName);
            }
        }).observe(document.documentElement, { attributes: true, subtree: true });
    </script>
</body>
</html>
"""

async def run_tests():
    print("=" * 60)
    print(">>> 1. Launching best-driver in headless mode...")
    browser = await best_driver.start(headless=True)
    test_file_path = os.path.abspath("test_page_v013.html")
    try:
        tab = await browser.new_tab()
        with open(test_file_path, "w", encoding="utf-8") as f:
            f.write(HTML_TEST_PAGE)

        file_url = f"file:///{test_file_path.replace(os.sep, '/')}"
        await tab.get(file_url)

        print("\n--- TEST SUITE 1: Native CDP Element API & Zero DOM Mutation ---")
        el_title = await tab.ele("#main-title")
        assert el_title is not None
        print(f"[PASS] tab.ele('#main-title'): {el_title} (backend_node_id={el_title.backend_node_id})")
        assert el_title.tag == "h1"
        assert "Best Driver" in el_title.text
        assert el_title["id"] == "main-title"
        assert el_title.get("missing", "fallback") == "fallback"

        el_input = await tab.ele("@id=username")
        assert el_input is not None
        await el_input.human_type("ritmada_benchmark", clear=True)

        el_btn = await tab.ele("#submit-btn")
        assert el_btn is not None
        await el_btn.click()

        el_status = await tab.ele("#status")
        assert el_status is not None and "ritmada_benchmark" in el_status.text
        print(f"[PASS] human_type + click verified: '{el_status.text}'")

        items = await tab.eles(".item")
        assert len(items) == 3
        assert (await items[1].attr("data-code")) == "B2"
        print(f"[PASS] tab.eles('.item') returned {len(items)} elements")

        # Chained child queries (CSS, @attr, text:, XPath)
        el_container = await tab.ele("#container")
        assert el_container is not None
        child_li = await el_container.ele(".item")
        assert child_li is not None and child_li.text == "Alpha Item"
        child_text = await el_container.ele("text:Gamma Item")
        assert child_text is not None and (await child_text.attr("data-code")) == "C3"
        child_items = await el_container.eles(".item")
        assert len(child_items) == 3
        print(f"[PASS] el_container.ele/eles (CSS + text:) verified")

        # Shadow DOM piercing + child chaining + outer_html + count/text_all
        shadow_host = await tab.ele("#shadow-host")
        assert shadow_host is not None
        shadow_inner = await shadow_host.ele(".shadow-inner")
        assert shadow_inner is not None
        shadow_btn = await shadow_inner.ele("#shadow-btn")
        assert shadow_btn is not None and shadow_btn.text == "Click Shadow"
        outer = await shadow_btn.outer_html()
        assert 'id="shadow-btn"' in outer
        await shadow_btn.click()
        assert await tab.text("#shadow-btn") == "Clicked Inside Shadow"
        assert await tab.count(".shadow-text") == 1
        assert await tab.text_all(".shadow-text") == ["Inside Shadow Root"]
        print(f"[PASS] Shadow DOM chaining, outer_html, click, count, text_all verified")

        # Verify ZERO DOM mutations happened during all locator queries!
        mutations = await tab.evaluate("window.__attrMutations")
        print(f"[PASS] MutationObserver attribute mutations during ele/eles: {mutations}")
        assert mutations == [], f"Unexpected DOM attribute mutations: {mutations}"

        print("\n--- TEST SUITE 2: Cookie Ergonomics ---")
        await tab.set_cookie("session_token", "secret_token_123")
        cookies = await tab.cookies()
        assert any(c.get("name") == "session_token" and c.get("value") == "secret_token_123" for c in cookies)
        print("[PASS] tab.set_cookie() & tab.cookies() verified")

        print("\n--- TEST SUITE 3: Deep Stealth & Prototype Integrity ---")
        checks = await tab.evaluate("""(() => {
            const wdDesc = Object.getOwnPropertyDescriptor(Navigator.prototype, 'webdriver');
            const c = document.createElement('canvas');
            c.width = 1; c.height = 1;
            const px = Array.from(c.getContext('2d').getImageData(0, 0, 1, 1).data);
            return {
                ua: navigator.userAgent,
                hasHeadlessUA: navigator.userAgent.includes('HeadlessChrome'),
                webdriver: navigator.webdriver,
                wdInProto: 'webdriver' in Navigator.prototype,
                wdGetterHasProto: wdDesc && wdDesc.get ? ('prototype' in wdDesc.get) : true,
                wdGetterStr: wdDesc && wdDesc.get ? wdDesc.get.toString() : '',
                navOwnProps: Object.getOwnPropertyNames(navigator),
                permOwnProps: Object.getOwnPropertyNames(navigator.permissions),
                hasChrome: !!(window.chrome && window.chrome.app),
                voicesLength: window.speechSynthesis ? window.speechSynthesis.getVoices().length : -1,
                effectiveType: navigator.connection ? navigator.connection.effectiveType : null,
                outerWidth: window.outerWidth,
                innerWidth: window.innerWidth,
                outerHeight: window.outerHeight,
                innerHeight: window.innerHeight,
                transparentPixel: px,
            };
        })()""")

        for k, v in checks.items():
            print(f"  [STEALTH] {k}: {v}")

        assert checks["hasHeadlessUA"] is False, f"HeadlessChrome leaked in UA: {checks['ua']}"
        assert checks["webdriver"] is False
        assert checks["wdInProto"] is True
        assert checks["wdGetterHasProto"] is False, "webdriver getter has .prototype (constructable!)"
        assert checks["wdGetterStr"] == "function get webdriver() { [native code] }"
        assert checks["navOwnProps"] == [], f"Leaked navigator own props: {checks['navOwnProps']}"
        assert checks["permOwnProps"] == [], f"Leaked permissions own props: {checks['permOwnProps']}"
        assert checks["hasChrome"] is True
        assert checks["voicesLength"] > 0
        assert checks["effectiveType"] == "4g"
        assert checks["outerWidth"] > checks["innerWidth"]
        assert checks["outerHeight"] > checks["innerHeight"]
        assert checks["transparentPixel"] == [0, 0, 0, 0], f"Transparent pixel corrupted: {checks['transparentPixel']}"

        print("\n--- TEST SUITE 4: Flattened Multi-Tab, Popup Auto-Attach Stealth & Typed Errors ---")
        # Multiple tabs share ONE browser WebSocket with distinct flattened sessionIds
        tab2 = await browser.new_tab(file_url)
        tab3 = await browser.new_tab(file_url)
        sids = {tab.session_id, tab2.session_id, tab3.session_id}
        print(f"[PASS] 3 tabs over 1 browser WS ({browser.ws_url[:35]}...): unique session_ids={len(sids)}")
        assert len(sids) == 3 and all(sids)

        # Popup window.open() inherits evasion + clean UA automatically via Target.setAutoAttach
        known_ids = {t.target_id for t in await browser.tabs()}
        await tab.evaluate(f"void window.open({file_url!r}, '_blank')")
        await tab.sleep(0.8)
        all_tabs = await browser.tabs()
        new_tabs = [t for t in all_tabs if t.target_id not in known_ids]
        assert len(new_tabs) >= 1, f"Expected popup tab, got {len(all_tabs)} total tabs"
        popup = new_tabs[0]
        popup_stealth = await popup.evaluate("({ wd: navigator.webdriver, headless: navigator.userAgent.includes('HeadlessChrome'), navOwn: Object.getOwnPropertyNames(navigator) })")
        print(f"[PASS] Auto-attached popup stealth (session_id={popup.session_id[:12]}...): {popup_stealth}")
        assert popup_stealth["wd"] is False
        assert popup_stealth["headless"] is False
        assert popup_stealth["navOwn"] == []

        # Typed Python TimeoutError on wait_for timeout
        try:
            await tab.wait_for("#definitely-never-exists", timeout_ms=300)
            raise AssertionError("Expected TimeoutError")
        except TimeoutError as e:
            print(f"[PASS] Typed TimeoutError raised cleanly: {e}")

        await tab2.close()
        await tab3.close()
        await popup.close()

        print("\n--- TEST SUITE 5: Iframe Controller, Download Manager & Graceful Lifecycle ---")
        assert await browser.is_alive() is True
        frames_list = await tab.frames()
        print(f"[PASS] tab.frames() found {len(frames_list)} frames")
        assert len(frames_list) >= 2

        frame = await tab.iframe("#demo-frame")
        assert frame.frame_id is not None
        ftitle = await frame.text("#frame-title")
        assert ftitle == "Inside Iframe", f"Unexpected iframe title: {ftitle}"
        await frame.human_type("#frame-input", "iframe_ok")
        fbtn = await frame.ele("#frame-btn")
        assert fbtn is not None
        await fbtn.click()
        fout = await frame.text("#frame-out")
        print(f"[PASS] tab.iframe('#demo-frame') human_type + Bézier click -> '{fout}'")
        assert fout == "Frame: iframe_ok"

        # Download Manager test (PyO3 future starts running on Tokio immediately upon call)
        dl_dir = os.path.abspath("tmp_test_downloads")
        dl_fut = tab.wait_for_download(dl_dir, timeout_ms=10000)
        await tab.sleep(0.15)
        await tab.click("#dl-link")
        dl_info = await dl_fut
        print(f"[PASS] tab.wait_for_download: {dl_info}")
        assert dl_info["state"] == "completed"
        assert dl_info["filename"] == "best_driver_test.txt"
        assert os.path.exists(dl_info["path"])
        with open(dl_info["path"], "r", encoding="utf-8") as df:
            assert df.read() == "hello best driver"
        os.remove(dl_info["path"])
        os.rmdir(dl_dir)

        print("\n--- TEST SUITE 6: Runtime.enable Leak Traps & Closed Shadow DOM ---")
        # 6a. Cloudflare/DataDome console.debug(Error) stack-getter CDP side-channel trap
        cdp_traps = await tab.evaluate("""(() => {
            let errorStackTrap = false;
            const e = new Error();
            Object.defineProperty(e, 'stack', {
                get() {
                    errorStackTrap = true;
                    return '';
                }
            });
            console.debug(e);

            let objGetterTrap = false;
            const probe = {};
            Object.defineProperty(probe, 'id', {
                enumerable: true,
                get() {
                    objGetterTrap = true;
                    return 1;
                }
            });
            console.debug(probe);
            console.log(probe);

            return {
                errorStackTrap,
                objGetterTrap,
                closedHostShadowRootInJs: document.getElementById('closed-shadow-host').shadowRoot
            };
        })()""")
        print(f"  [TRAP] Runtime.enable probe results: {cdp_traps}")
        assert cdp_traps["errorStackTrap"] is False, "FAILED: Runtime.enable Error.stack trap triggered!"
        assert cdp_traps["objGetterTrap"] is False, "FAILED: Runtime.enable object getter trap triggered!"
        assert cdp_traps["closedHostShadowRootInJs"] is None, "Sanity check: closed shadowRoot must be hidden from JS"

        # 6b. Closed Shadow DOM (#shadow-root (closed)) piercing via native CDP DOM.getDocument(pierce=True)
        assert await tab.exists("#closed-cb") is True
        cb_text = await tab.text("text:Verify you are human")
        assert cb_text == "Verify you are human", f"Unexpected closed-shadow text: {cb_text}"
        cb_list = await tab.eles("input[type='checkbox']")
        assert len(cb_list) == 1 and cb_list[0].id == "closed-cb"
        closed_host_el = await tab.ele("#closed-shadow-host")
        assert closed_host_el is not None
        cb_el = await closed_host_el.ele("#closed-cb")
        assert cb_el is not None and cb_el.tag == "input"
        await cb_el.click()
        assert await tab.evaluate("window.__closedClicked") is True
        print(f"[PASS] Closed Shadow DOM pierced & clicked (backend_node_id={cb_el.backend_node_id})")

        # 6c. Iframe nested inside #shadow-root (closed)
        closed_frame = await tab.iframe("#closed-iframe")
        closed_frame_msg = await closed_frame.text("#closed-frame-msg")
        print(f"[PASS] Iframe inside Closed Shadow DOM read -> '{closed_frame_msg}'")
        assert closed_frame_msg == "Inside Closed Shadow Iframe"

        print("\n>>> ALL V0.1.3+ AUDIT TESTS PASSED (100%) <<<")
    finally:
        await browser.stop()
        assert await browser.is_alive() is False
        if os.path.exists(test_file_path):
            os.remove(test_file_path)

if __name__ == "__main__":
    asyncio.run(run_tests())
