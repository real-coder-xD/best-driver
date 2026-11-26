"""Actionability engine (overlay hit-test, layout stability, enabled wait), ARIA locators, strict mode, and FrameLocator."""

from __future__ import annotations

import asyncio

import pytest
import best_driver


def test_actionability_waits_for_overlay_layout_shift_and_enabled(server_url: str) -> None:
    async def _run() -> None:
        async with await best_driver.start(headless=True) as browser:
            tab = await browser.get(server_url + "/")

            # 1. Hit-test waits until modal overlay disappears (no blind click on blocker)
            await tab.evaluate("window.__unblockIn(300)")
            await tab.click("#covered-btn", timeout_ms=5000)
            assert await tab.evaluate("window.__coveredClicks") == 1

            # 2. force=True clicks even when disabled/covered
            await tab.evaluate("document.getElementById('blocker').style.display = 'block'")
            with pytest.raises(TimeoutError):
                await tab.click("#covered-btn", timeout_ms=350)

            # 3. Layout shift stability: waits for shifting button to settle before clicking
            await tab.evaluate("window.__startShift()")
            await tab.click("#shifting-btn", timeout_ms=5000)
            assert await tab.evaluate("window.__shiftClicks") == 1

            # 4. Enabled check: waits until button becomes enabled
            await tab.evaluate("window.__enableIn(250)")
            await tab.click("#delayed-enable", timeout_ms=5000)
            assert await tab.evaluate("window.__enabledClicks") == 1

    asyncio.run(_run())


def test_semantic_aria_locators_strict_and_closed_shadow(server_url: str) -> None:
    async def _run() -> None:
        async with await best_driver.start(headless=True) as browser:
            tab = await browser.get(server_url + "/")

            assert await tab.get_by_test_id("hero-title").text() == "Welcome Portal"
            assert await tab.get_by_role("heading", name="Welcome Portal", exact=True).text() == "Welcome Portal"

            await tab.get_by_label("Email Address").fill("engineer@example.com")
            assert await tab.get_by_placeholder("you@example.com").value() == "engineer@example.com"

            await tab.get_by_label("Remember Me").check()
            assert await tab.get_by_role("checkbox", name="Remember Me").is_checked() is True

            await tab.get_by_role("combobox", name="Plan").select_option("Pro Tier")
            assert await tab.get_by_role("combobox", name="Plan").value() == "pro"

            await tab.get_by_role("button", name="Sign In", exact=True).click()
            assert await tab.evaluate("window.__submitted") == "engineer@example.com"

            # Strict mode raises ValueError when >1 element matches
            with pytest.raises(ValueError, match="Strict mode violation"):
                await tab.locator(".dup", strict=True).click(timeout_ms=1000)
            assert await tab.locator(".dup").count() == 2

            # Closed shadow DOM via placeholder + role
            await tab.get_by_placeholder("Secret Code").fill("OTP-999")
            assert await tab.get_by_placeholder("Secret Code").value() == "OTP-999"
            await tab.get_by_role("button", name="Verify Human").click()
            assert await tab.evaluate("window.__csClicked") is True

    asyncio.run(_run())


def test_frame_locator_survives_iframe_reload(server_url: str) -> None:
    async def _run() -> None:
        async with await best_driver.start(headless=True) as browser:
            tab = await browser.get(server_url + "/")
            fl = tab.frame_locator("#reloading-frame")

            assert await fl.locator("#f-ver").text() == "Frame v1"
            await fl.get_by_role("button", name="Press v1").click()
            assert await fl.locator("#f-out").text() == "clicked-v1"

            # Reload iframe to v2; the SAME FrameLocator re-resolves cleanly without stale errors
            await tab.evaluate("window.__reloadFrame('v2')")
            await fl.get_by_text("Frame v2", exact=True).wait_for(timeout_ms=5000)
            await fl.get_by_role("button", name="Press v2").click()
            assert await fl.locator("#f-out").text() == "clicked-v2"

    asyncio.run(_run())
