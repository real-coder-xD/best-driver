"""
best-driver quickstart example
Demonstrates stealth navigation, DOM interaction, and full-page capture.
"""
import asyncio
import best_driver

async def main():
    async with await best_driver.start(headless=False) as browser:
        print(f"[*] Engine started on debugging port: {browser.port}")

        # Navigate to target
        tab = await browser.get("https://bot.sannysoft.com", wait_until="networkidle")
        print("[+] Title:", await tab.title())

        # Verify bot detection flag is cleanly absent
        webdriver_flag = await tab.evaluate("navigator.webdriver")
        print("[+] navigator.webdriver:", webdriver_flag)  # None

        # Take full-page screenshot
        await tab.screenshot("quickstart_result.png", full_page=True)
        print("[+] Full-page screenshot saved to quickstart_result.png")

if __name__ == "__main__":
    asyncio.run(main())
