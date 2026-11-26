"""Lifecycle (start_url, keep_open/connect), BrowserContext downloads, dialogs, chords, storage_state, cleanup."""

from __future__ import annotations

import asyncio
from pathlib import Path

import best_driver


def test_start_url_modes_and_browser_context_download(server_url: str, tmp_path: Path) -> None:
    async def _run() -> None:
        # Default start_url=None skips forcing about:blank (allows --no-startup-window cleanly)
        async with await best_driver.start(headless=True, extra_args=["--no-startup-window"]) as browser:
            assert len(await browser.tabs()) == 0
            tab = await browser.get(server_url + "/popup")
            assert await tab.title() == "Popup Target"

        async with await best_driver.start(headless=True) as browser:
            tabs = await browser.tabs()
            assert len(tabs) >= 1

            # Isolated BrowserContext download with pathlib.Path
            dl_dir = tmp_path / "ctx_dl"
            async with await browser.new_context() as ctx:
                ctab = await ctx.get(server_url + "/")
                dl_fut = asyncio.ensure_future(ctab.wait_for_download(dl_dir, timeout_ms=10000))
                await asyncio.sleep(0.1)
                await ctab.click("#dl-link")
                info = await dl_fut
                assert info["state"] == "completed"
                assert Path(info["path"]).read_bytes() == b"BEST-DRIVER-V040-DOWNLOAD"

        # Explicit start_url opens directly on target with 1 tab
        async with await best_driver.start(headless=True, start_url=server_url + "/popup") as browser:
            tabs = await browser.tabs()
            assert len(tabs) == 1 and (await tabs[0].title()) == "Popup Target"

    asyncio.run(_run())


def test_dialogs_popups_chords_storage_state_and_cleanup(server_url: str, tmp_path: Path) -> None:
    async def _run() -> None:
        async with await best_driver.start(headless=True) as browser:
            tab = await browser.get(server_url + "/")

            # Sync auto_dialog (zero Tokio panic)
            tab.auto_dialog(accept=True, prompt_text="OK-040")
            assert await tab.evaluate("confirm('proceed?')") is True
            assert await tab.evaluate("prompt('code?')") == "OK-040"
            tab.auto_dialog(None)

            # Real <a target="_blank"> popup capture
            async with tab.expect_popup(timeout_ms=10000) as pinfo:
                await tab.click("#popup-link")
            popup = await pinfo.value
            await popup.wait_for("#pop-h1", timeout_ms=5000)
            assert await popup.text("#pop-h1") == "Popup Ready"
            await popup.close()

            # Modifier key chord Control+a -> Backspace
            await tab.fill("#email-in", "old@example.com")
            await tab.focus("#email-in")
            await tab.press_key("Control+a")
            await tab.press_key("Backspace")
            await tab.type_text("new@example.com", delay_ms=5)
            assert await tab.value("#email-in") == "new@example.com"

            # Symmetric storage_state + load_storage_state with PathLike
            await tab.set_cookie("sid", "tok_040")
            await tab.storage_set("theme", "dark")
            state_path = tmp_path / "state.json"
            await tab.storage_state(state_path)
            await tab.clear_cookies()
            await tab.storage_clear()
            loaded = await tab.load_storage_state(state_path)
            assert loaded["cookies"] >= 1 and loaded["local_storage"] >= 1
            assert await tab.storage_get("theme") == "dark"

            # Permissions + window bounds
            await tab.grant_permissions(["notifications", "clipboard-read"])
            bounds = await tab.get_window_bounds()
            assert isinstance(bounds["window_id"], int)

    asyncio.run(_run())

    # Conflict-safe cleanup on isolated temp dir
    fake_tmp = tmp_path / "fake_temp"
    fake_tmp.mkdir()
    stale = fake_tmp / "best_driver_profile_stale1"
    keep = fake_tmp / "best_driver_profile_keep1"
    stale.mkdir()
    (stale / "a.bin").write_bytes(b"0" * 1024)
    keep.mkdir()
    (keep / "b.bin").write_bytes(b"1" * 1024)
    rep = best_driver.cleanup(temp_dir=fake_tmp, exclude_dirs=[keep])
    assert len(rep["removed_dirs"]) == 1 and not stale.exists() and keep.exists()
    assert rep["freed_bytes"] >= 1024
