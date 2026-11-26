"""Stealth Engine v2 verification: standard (zero-lie), minimal (zero-JS), aggressive, and CDP traps."""

from __future__ import annotations

import asyncio

import pytest
import best_driver


def test_stealth_standard_zero_webdriver_lie_and_cdp_traps(server_url: str) -> None:
    async def _run() -> None:
        async with await best_driver.start(headless=True, stealth="standard") as browser:
            tab = await browser.get(server_url + "/")
            res = await tab.evaluate("""(() => {
                // 1. Cross-realm Function.prototype.toString probe (CreepJS lie detector)
                const iframe = document.createElement('iframe');
                document.body.appendChild(iframe);
                const xToString = iframe.contentWindow.Function.prototype.toString;
                const wdDesc = Object.getOwnPropertyDescriptor(Navigator.prototype, 'webdriver');
                const wdCrossRealm = wdDesc && wdDesc.get ? xToString.call(wdDesc.get) : '';
                const canvasCrossRealm = xToString.call(HTMLCanvasElement.prototype.toDataURL);
                const audioCrossRealm = window.AudioBuffer ? xToString.call(AudioBuffer.prototype.getChannelData) : '[native code]';

                // 2. Runtime.enable Error.stack getter trap
                let stackTrap = false;
                const err = new Error();
                Object.defineProperty(err, 'stack', { get() { stackTrap = true; return ''; } });
                console.debug(err);

                // 3. WebGL OS consistency
                const cv = document.createElement('canvas');
                const gl = cv.getContext('webgl');
                let vendor = '', renderer = '';
                if (gl) {
                    const ext = gl.getExtension('WEBGL_debug_renderer_info');
                    if (ext) {
                        vendor = gl.getParameter(ext.UNMASKED_VENDOR_WEBGL) || '';
                        renderer = gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) || '';
                    }
                }
                return {
                    ua: navigator.userAgent,
                    webdriver: navigator.webdriver,
                    wdCrossRealm,
                    canvasCrossRealm,
                    audioCrossRealm,
                    navOwn: Object.getOwnPropertyNames(navigator),
                    stackTrap,
                    vendor,
                    renderer,
                };
            })()""")

            assert "HeadlessChrome" not in res["ua"]
            assert res["webdriver"] is False
            assert "[native code]" in res["wdCrossRealm"], res["wdCrossRealm"]
            assert "[native code]" in res["canvasCrossRealm"], res["canvasCrossRealm"]
            assert "[native code]" in res["audioCrossRealm"], res["audioCrossRealm"]
            assert res["navOwn"] == []
            assert res["stackTrap"] is False
            assert "swiftshader" not in res["renderer"].lower()

    asyncio.run(_run())


def test_stealth_minimal_zero_injected_js(server_url: str) -> None:
    async def _run() -> None:
        async with await best_driver.start(headless=True, stealth="minimal") as browser:
            tab = await browser.get(server_url + "/")
            res = await tab.evaluate("""(() => {
                const iframe = document.createElement('iframe');
                document.body.appendChild(iframe);
                const xToString = iframe.contentWindow.Function.prototype.toString;
                return {
                    headlessUa: navigator.userAgent.includes('HeadlessChrome'),
                    webdriver: navigator.webdriver,
                    toStringNative: xToString.call(Function.prototype.toString).includes('[native code]'),
                    permQueryNative: xToString.call(Permissions.prototype.query).includes('[native code]'),
                };
            })()""")
            assert res["headlessUa"] is False
            assert res["webdriver"] is False
            assert res["toStringNative"] is True
            assert res["permQueryNative"] is True

    asyncio.run(_run())


def test_stealth_invalid_mode_raises_value_error() -> None:
    async def _run() -> None:
        with pytest.raises(ValueError):
            await best_driver.start(headless=True, stealth="invalid_mode")  # type: ignore[arg-type]

    asyncio.run(_run())
