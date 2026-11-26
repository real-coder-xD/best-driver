"""Network routing (request/response), HAR 1.2, wait_for_response globs, and interactive Trace Viewer."""

from __future__ import annotations

import asyncio
import json
from pathlib import Path

import best_driver
from best_driver.network import _CONTROLLERS, _HAR_RECORDERS
from best_driver.tracing import _TRACERS


def test_routing_har_and_interactive_trace_viewer(server_url: str, tmp_path: Path) -> None:
    async def _run() -> None:
        async with await best_driver.start(headless=True) as browser:
            tab = await browser.get(server_url + "/")
            sid = tab.session_id

            # Start tracing with screenshots + network + page errors
            await tab.start_tracing(screenshots=True, network=True, console=True, page_errors=True)
            await tab.trace_snapshot("initial")

            # Request & response stage routing
            async def on_req(route):
                hdrs = dict(route.headers)
                hdrs["X-Trace-Test"] = "v040"
                await route.continue_(headers=hdrs, post_data='{"k":"v"}')

            async def on_resp(route):
                body = await route.response_json()
                body["patched"] = True
                await route.fulfill(status=200, json=body)

            await tab.route("*/echo*", on_req, stage="request")
            await tab.route("*/api/data*", on_resp, stage="response")

            r_post = await tab.fetch(server_url + "/echo", method="POST", body="orig")
            assert json.loads(r_post["body"]) == {"hdr": "v040", "body": '{"k":"v"}'}

            resp_fut = asyncio.ensure_future(tab.wait_for_response("*/api/data?q=*", timeout_ms=5000))
            await asyncio.sleep(0.05)
            r_get = await tab.fetch(server_url + "/api/data?q=42")
            w_resp = await resp_fut
            assert json.loads(r_get["body"])["patched"] is True
            assert w_resp["status"] == 200

            await tab.unroute()
            await tab.evaluate("console.warn('trace_warn_msg')")
            await asyncio.sleep(0.1)

            trace_html = tmp_path / "trace.html"
            trace_data = await tab.stop_tracing(trace_html)
            assert trace_html.exists()
            html_txt = trace_html.read_text(encoding="utf-8")
            assert "best-driver Trace Viewer" in html_txt
            assert trace_data["snapshot_count"] >= 2
            assert any(e["kind"] == "network" for e in trace_data["events"])
            assert any(e["kind"] == "console" and "trace_warn_msg" in e["title"] for e in trace_data["events"])

            # Verify HAR export with PathLike + zero-leak eviction on tab close
            await tab.record_har()
            await tab.fetch(server_url + "/api/data?har=1")
            await asyncio.sleep(0.15)
            har_file = tmp_path / "net.har"
            har = await tab.export_har(har_file)
            assert har_file.exists() and len(har["log"]["entries"]) >= 1

            # Open second tab so closing tab doesn't terminate browser
            await browser.new_tab(server_url + "/")
            await tab.close()
            assert sid not in _CONTROLLERS
            assert sid not in _HAR_RECORDERS
            assert sid not in _TRACERS

    asyncio.run(_run())
