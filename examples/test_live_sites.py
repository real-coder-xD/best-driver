import asyncio
import best_driver

async def test_sannysoft(browser):
    print("\n[TEST 1] bot.sannysoft.com (Bot Detection Testbench)...")
    tab = await browser.get("https://bot.sannysoft.com")
    await tab.wait_for_selector("#webdriver-result", timeout_ms=10000)

    # Check ALL anti-detect rows accurately using real Sannysoft CSS classes ('failed' / 'warn')
    rows = await tab.evaluate("""(() => {
        const out = [];
        for (const r of document.querySelectorAll('table tr')) {
            const tds = r.querySelectorAll('td');
            if (tds.length >= 2) {
                const name = tds[0].innerText.trim().replace(/\\s+/g, ' ');
                const val = tds[1].innerText.trim();
                const cls = (tds[1].className || '').toLowerCase();
                out.push({ name, val, cls, failed: cls.includes('fail'), warn: cls.includes('warn') });
            }
        }
        return out;
    })()""")

    failed = [r for r in rows if r["failed"]]
    warns = [r for r in rows if r["warn"]]
    print(f"Total tests evaluated: {len(rows)} | Failed: {len(failed)} | Warn: {len(warns)}")
    for r in failed:
        print(f"  [FAIL] {r['name']}: {r['val']} (class={r['cls']})")
    for r in warns:
        print(f"  [WARN] {r['name']}: {r['val']} (class={r['cls']})")
    assert len(failed) == 0, f"Sannysoft failures: {failed}"
    assert len(warns) == 0, f"Sannysoft warnings: {warns}"
    print(f"  • Sannysoft ALL {len(rows)}/{len(rows)} PASSED (0 fail, 0 warn)")

async def test_duckduckgo(browser):
    print("\n[TEST 2] DuckDuckGo (Smart Locator & Human Typing)...")
    tab = await browser.get("https://duckduckgo.com")
    
    # 1. Use Smart Locator attribute selector to find search box
    await tab.wait_for_selector("@name=q", timeout_ms=10000)
    print("  • Found search box via Smart Locator '@name=q'")

    # 2. Use human_type with biological cadence
    print("  • Typing query with human_type...")
    await tab.human_type("@name=q", "Rust CDP Best Driver")

    # 3. Click search button via smart locator or press enter
    print("  • Submitting search...")
    if await tab.exists("button[type=\"submit\"]"):
        await tab.click("button[type=\"submit\"]")
    else:
        await tab.press_key("Enter")

    # Wait for results page
    await tab.sleep(3)
    titles = await tab.evaluate("""(() => {
        const links = Array.from(document.querySelectorAll('a h2, [data-testid="result-title-a"], article'));
        return links.slice(0, 3).map(h => (h.innerText || h.textContent || '').trim()).filter(Boolean);
    })()""")
    print("  • Search results loaded:")
    for i, t in enumerate(titles, 1):
        print(f"    [{i}] {t}")

async def test_cloudflare(browser):
    print("\n[TEST 3] nowsecure.nl (Cloudflare Turnstile Detection)...")
    tab = await browser.get("https://nowsecure.nl")
    await tab.sleep(3)
    title = await tab.title()
    content = await tab.content()
    passed = "moment" not in title.lower() and "just a moment" not in content.lower()
    print(f"  • Title: {title}")
    if passed:
        print("  • Cloudflare Turnstile check: PASSED (Bypassed / Access granted)")
    else:
        print("  • Cloudflare Turnstile check: Interstitial challenge active")

async def main():
    print("Starting Chromium in Headless mode...")
    browser = await best_driver.start(headless=True)
    try:
        await test_sannysoft(browser)
        await test_duckduckgo(browser)
        await test_cloudflare(browser)
    finally:
        await browser.stop()
        print("\nAll live tests completed.")

if __name__ == "__main__":
    asyncio.run(main())
