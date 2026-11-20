import asyncio
import best_driver

async def test_network_and_live():
    print(">>> Testing Network Response Interception & Live Sannysoft (Headless)...")
    async with await best_driver.start(headless=True) as browser:
        tab = await browser.new_tab()

        # 1. Test wait_for_response with fetch
        await tab.get("https://httpbin.org/html")
        print(f"Current URL: {await tab.current_url()}")

        async def trigger():
            await asyncio.sleep(0.2)
            await tab.evaluate("fetch('https://httpbin.org/json')")

        task = asyncio.create_task(trigger())
        resp = await tab.wait_for_response("httpbin.org/json", timeout_ms=15000)
        await task

        print(f"[PASS] wait_for_response: status={resp['status']} mime={resp['mime_type']} url={resp['url']}")
        assert resp.get("status") == 200
        assert resp.get("body") and "slideshow" in resp["body"]

        # 2. Full Sannysoft check (ALL rows, 0 fail, 0 warn)
        print("\n>>> Testing bot.sannysoft.com (Headless)...")
        await tab.get("https://bot.sannysoft.com", wait_until="networkidle")
        await tab.sleep(2)

        rows = await tab.evaluate("""(() => {
            return Array.from(document.querySelectorAll('table tr')).map(r => {
                const tds = r.querySelectorAll('td');
                if (tds.length < 2) return null;
                const cls = (tds[1].className || '').toLowerCase();
                return {
                    name: tds[0].innerText.trim().replace(/\\s+/g, ' '),
                    val: tds[1].innerText.trim(),
                    cls,
                    failed: cls.includes('fail'),
                    warn: cls.includes('warn')
                };
            }).filter(Boolean);
        })()""")
        failed = [r for r in rows if r["failed"]]
        warns = [r for r in rows if r["warn"]]
        print(f"[SANNYSOFT] Total rows: {len(rows)} | Failed: {len(failed)} | Warn: {len(warns)}")
        for r in failed:
            print("  [FAIL]", r)
        for r in warns:
            print("  [WARN]", r)
        assert len(failed) == 0, f"Sannysoft failed rows: {failed}"
        assert len(warns) == 0, f"Sannysoft warn rows: {warns}"

    print("\n>>> ALL NETWORK & LIVE TESTS PASSED (0 FAIL, 0 WARN)! <<<")

if __name__ == "__main__":
    asyncio.run(test_network_and_live())
