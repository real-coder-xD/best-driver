"""Self-contained interactive Trace Recorder & HTML Trace Viewer for `best-driver`.

Captures timestamped network waterfall, screenshots, URL/title snapshots, and
optional console/page-error streams, exporting either JSON or a standalone
single-file dark-mode interactive HTML Trace Viewer (`trace.html`) with zero
external dependencies or local web server required.
"""

from __future__ import annotations

import asyncio
import base64
import datetime as _dt
import html as _html
import json as _json
import os
import time
from typing import Any, Dict, List, Optional, Union

PathLike = Union[str, os.PathLike[str]]


class TraceRecorder:
    """Active background trace session attached to a `Tab`."""

    def __init__(
        self,
        tab: Any,
        *,
        screenshots: bool = True,
        network: bool = True,
        console: bool = False,
        page_errors: bool = False,
        screenshot_interval_ms: int = 1000,
    ) -> None:
        self._tab = tab
        self._screenshots = screenshots
        self._network = network
        self._console = console
        self._page_errors = page_errors
        self._interval_s = max(0.25, screenshot_interval_ms / 1000.0)
        self._started_monotonic = time.monotonic()
        self.started_at = _dt.datetime.now(_dt.timezone.utc).isoformat()
        self.events: List[Dict[str, Any]] = []
        self.snapshots: List[Dict[str, Any]] = []
        self._pending_net: Dict[str, Dict[str, Any]] = {}
        self._net_stream: Any = None
        self._console_stream: Any = None
        self._err_stream: Any = None
        self._tasks: List[asyncio.Task[None]] = []
        self._stopped = False

    def _rel_ms(self) -> float:
        return round((time.monotonic() - self._started_monotonic) * 1000.0, 1)

    async def start(self) -> None:
        if self._network:
            self._net_stream = self._tab._event_stream(
                [
                    "Network.requestWillBeSent",
                    "Network.responseReceived",
                    "Network.loadingFinished",
                    "Network.loadingFailed",
                    "Page.frameNavigated",
                ]
            )
            self._tasks.append(asyncio.create_task(self._net_loop()))
        if self._console:
            self._console_stream = self._tab.listen_console()
            await self._console_stream._ensure_started()
            self._tasks.append(asyncio.create_task(self._console_loop()))
        if self._page_errors:
            self._err_stream = self._tab.listen_page_errors()
            await self._err_stream._ensure_started()
            self._tasks.append(asyncio.create_task(self._err_loop()))
        if self._screenshots:
            await self.capture_snapshot("trace:start")
            self._tasks.append(asyncio.create_task(self._screenshot_loop()))

    async def capture_snapshot(self, label: str = "snapshot") -> Dict[str, Any]:
        url = ""
        title = ""
        img_b64 = ""
        try:
            url = await self._tab.url()
        except Exception:
            pass
        try:
            title = await self._tab.title()
        except Exception:
            pass
        if self._screenshots:
            try:
                raw = await self._tab.screenshot(format="jpeg", quality=68)
                img_b64 = base64.b64encode(raw).decode("ascii")
            except Exception:
                pass
        snap = {
            "t_ms": self._rel_ms(),
            "label": label,
            "url": url,
            "title": title,
            "image_jpeg_b64": img_b64,
        }
        self.snapshots.append(snap)
        self.events.append(
            {
                "t_ms": snap["t_ms"],
                "kind": "snapshot",
                "title": label,
                "detail": f"{title} ({url})".strip(),
                "snapshot_idx": len(self.snapshots) - 1,
            }
        )
        return snap

    async def _screenshot_loop(self) -> None:
        try:
            while not self._stopped:
                await asyncio.sleep(self._interval_s)
                if self._stopped:
                    break
                await self.capture_snapshot("auto")
        except asyncio.CancelledError:
            pass

    async def _net_loop(self) -> None:
        assert self._net_stream is not None
        try:
            while not self._stopped:
                ev = await self._net_stream.next()
                if not ev:
                    break
                method = ev.get("method")
                params = ev.get("params") or {}
                if method == "Page.frameNavigated":
                    frame = params.get("frame") or {}
                    if not frame.get("parentId"):
                        self.events.append(
                            {
                                "t_ms": self._rel_ms(),
                                "kind": "navigation",
                                "title": str(frame.get("url") or ""),
                                "detail": str(frame.get("mimeType") or ""),
                            }
                        )
                    continue
                rid = str(params.get("requestId") or "")
                if not rid:
                    continue
                if method == "Network.requestWillBeSent":
                    req = params.get("request") or {}
                    self._pending_net[rid] = {
                        "t0_ms": self._rel_ms(),
                        "method": str(req.get("method") or "GET"),
                        "url": str(req.get("url") or ""),
                        "resource_type": str(params.get("type") or ""),
                    }
                elif method == "Network.responseReceived":
                    rec = self._pending_net.get(rid)
                    if rec is not None:
                        resp = params.get("response") or {}
                        rec["status"] = int(resp.get("status") or 0)
                        rec["mime_type"] = str(resp.get("mimeType") or "")
                        rec["protocol"] = str(resp.get("protocol") or "")
                elif method == "Network.loadingFinished":
                    rec = self._pending_net.pop(rid, None)
                    if rec is not None:
                        t1 = self._rel_ms()
                        dur = round(max(0.0, t1 - rec["t0_ms"]), 1)
                        self.events.append(
                            {
                                "t_ms": rec["t0_ms"],
                                "kind": "network",
                                "method": rec["method"],
                                "status": rec.get("status", 0),
                                "duration_ms": dur,
                                "bytes": int(params.get("encodedDataLength") or 0),
                                "title": f"{rec['method']} {rec.get('status', 0)} {rec['url']}",
                                "detail": f"{dur}ms · {rec.get('mime_type', '')} · {rec.get('protocol', '')}",
                            }
                        )
                elif method == "Network.loadingFailed":
                    rec = self._pending_net.pop(rid, None)
                    if rec is not None:
                        self.events.append(
                            {
                                "t_ms": rec["t0_ms"],
                                "kind": "network_error",
                                "method": rec["method"],
                                "status": 0,
                                "title": f"{rec['method']} FAILED {rec['url']}",
                                "detail": str(params.get("errorText") or "Failed"),
                            }
                        )
        except asyncio.CancelledError:
            pass
        finally:
            if self._net_stream is not None:
                self._net_stream.close()

    async def _console_loop(self) -> None:
        assert self._console_stream is not None
        try:
            async for msg in self._console_stream:
                self.events.append(
                    {
                        "t_ms": self._rel_ms(),
                        "kind": "console",
                        "level": msg.type,
                        "title": f"[{msg.type}] {msg.text}",
                        "detail": f"{msg.url}:{msg.line}" if msg.url else "",
                    }
                )
        except asyncio.CancelledError:
            pass

    async def _err_loop(self) -> None:
        assert self._err_stream is not None
        try:
            async for err in self._err_stream:
                self.events.append(
                    {
                        "t_ms": self._rel_ms(),
                        "kind": "page_error",
                        "title": f"{err.name}: {err.message}",
                        "detail": err.stack,
                    }
                )
        except asyncio.CancelledError:
            pass

    async def stop(self) -> Dict[str, Any]:
        if not self._stopped:
            if self._screenshots:
                await self.capture_snapshot("trace:stop")
            self._stopped = True
            if self._net_stream is not None:
                self._net_stream.close()
            for task in self._tasks:
                task.cancel()
            for task in self._tasks:
                try:
                    await task
                except BaseException:
                    pass
            self._tasks.clear()
            if self._console_stream is not None:
                await self._console_stream.aclose()
                self._console_stream = None
            if self._err_stream is not None:
                await self._err_stream.aclose()
                self._err_stream = None
        return self.to_dict()

    def close_sync(self) -> None:
        self._stopped = True
        if self._net_stream is not None:
            self._net_stream.close()
        if self._console_stream is not None:
            self._console_stream.close()
        if self._err_stream is not None:
            self._err_stream.close()
        for task in self._tasks:
            if not task.done():
                try:
                    task.cancel()
                except Exception:
                    pass
        self._tasks.clear()

    def to_dict(self) -> Dict[str, Any]:
        return {
            "tool": "best-driver",
            "version": "0.4.0",
            "started_at": self.started_at,
            "duration_ms": self._rel_ms(),
            "event_count": len(self.events),
            "snapshot_count": len(self.snapshots),
            "events": list(self.events),
            "snapshots": list(self.snapshots),
        }


def render_trace_html(trace: Dict[str, Any]) -> str:
    """Render a standalone single-file dark-mode interactive HTML Trace Viewer."""
    payload = _json.dumps(trace).replace("</", "<\\/")
    started = _html.escape(str(trace.get("started_at", "")))
    dur = trace.get("duration_ms", 0)
    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8" />
<title>best-driver Trace Viewer</title>
<style>
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{ font-family: system-ui, -apple-system, "Segoe UI", sans-serif; background: #0b1120; color: #e2e8f0; height: 100vh; display: flex; flex-direction: column; }}
  header {{ padding: 12px 20px; background: #0f172a; border-bottom: 1px solid #1e293b; display: flex; justify-content: space-between; align-items: center; }}
  header h1 {{ font-size: 16px; color: #38bdf8; }}
  header .meta {{ font-size: 12px; color: #94a3b8; font-family: monospace; }}
  .filmstrip {{ display: flex; gap: 10px; overflow-x: auto; padding: 10px 16px; background: #0a0f1c; border-bottom: 1px solid #1e293b; min-height: 112px; }}
  .thumb {{ flex: 0 0 auto; cursor: pointer; border: 2px solid #1e293b; border-radius: 8px; overflow: hidden; background: #0f172a; width: 150px; }}
  .thumb.active {{ border-color: #38bdf8; }}
  .thumb img {{ width: 100%; height: 74px; object-fit: cover; display: block; }}
  .thumb .cap {{ font-size: 11px; padding: 3px 6px; color: #cbd5e1; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-family: monospace; }}
  main {{ flex: 1; display: grid; grid-template-columns: 1.15fr 1fr; overflow: hidden; }}
  .pane {{ overflow-y: auto; padding: 14px 16px; }}
  .pane.left {{ border-right: 1px solid #1e293b; }}
  .filters {{ display: flex; gap: 8px; margin-bottom: 10px; flex-wrap: wrap; }}
  .fbtn {{ background: #1e293b; color: #cbd5e1; border: 1px solid #334155; border-radius: 6px; padding: 4px 10px; font-size: 12px; cursor: pointer; }}
  .fbtn.on {{ background: #0284c7; border-color: #38bdf8; color: #fff; }}
  .ev {{ padding: 7px 10px; border-radius: 6px; margin-bottom: 5px; background: #0f172a; border-left: 3px solid #475569; font-family: monospace; font-size: 12px; cursor: pointer; }}
  .ev.network {{ border-left-color: #38bdf8; }}
  .ev.network_error, .ev.page_error {{ border-left-color: #ef4444; }}
  .ev.navigation {{ border-left-color: #a855f7; }}
  .ev.snapshot {{ border-left-color: #22c55e; }}
  .ev.console {{ border-left-color: #f59e0b; }}
  .ev .t {{ color: #64748b; margin-right: 8px; }}
  .ev .d {{ color: #94a3b8; font-size: 11px; margin-top: 2px; word-break: break-all; }}
  .preview img {{ width: 100%; border-radius: 8px; border: 1px solid #1e293b; }}
  .preview .info {{ margin-bottom: 10px; font-family: monospace; font-size: 12px; color: #94a3b8; word-break: break-all; }}
</style>
</head>
<body>
<header>
  <h1>best-driver Trace Viewer</h1>
  <div class="meta">started={started} · duration={dur}ms · events={trace.get('event_count', 0)} · snapshots={trace.get('snapshot_count', 0)}</div>
</header>
<div class="filmstrip" id="filmstrip"></div>
<main>
  <div class="pane left">
    <div class="filters" id="filters"></div>
    <div id="events"></div>
  </div>
  <div class="pane preview" id="preview">
    <div class="info" id="snap-info">Select a snapshot or event</div>
    <img id="snap-img" style="display:none" />
  </div>
</main>
<script>
const TRACE = {payload};
const film = document.getElementById('filmstrip');
const evBox = document.getElementById('events');
const filters = document.getElementById('filters');
const snapImg = document.getElementById('snap-img');
const snapInfo = document.getElementById('snap-info');
let activeKind = 'all';

function showSnap(idx) {{
  const s = TRACE.snapshots[idx];
  if (!s) return;
  document.querySelectorAll('.thumb').forEach((t, i) => t.classList.toggle('active', i === idx));
  snapInfo.textContent = `[${{s.t_ms}}ms] ${{s.label}} — ${{s.title}} (${{s.url}})`;
  if (s.image_jpeg_b64) {{
    snapImg.src = 'data:image/jpeg;base64,' + s.image_jpeg_b64;
    snapImg.style.display = 'block';
  }} else {{
    snapImg.style.display = 'none';
  }}
}}

TRACE.snapshots.forEach((s, i) => {{
  const d = document.createElement('div');
  d.className = 'thumb';
  d.onclick = () => showSnap(i);
  d.innerHTML = (s.image_jpeg_b64 ? `<img src="data:image/jpeg;base64,${{s.image_jpeg_b64}}" />` : '') +
    `<div class="cap">${{s.t_ms}}ms ${{s.label}}</div>`;
  film.appendChild(d);
}});
if (TRACE.snapshots.length) showSnap(0);

['all', 'navigation', 'network', 'network_error', 'console', 'page_error', 'snapshot'].forEach(k => {{
  const b = document.createElement('button');
  b.className = 'fbtn' + (k === 'all' ? ' on' : '');
  b.textContent = k;
  b.onclick = () => {{
    activeKind = k;
    document.querySelectorAll('.fbtn').forEach(x => x.classList.toggle('on', x === b));
    renderEvents();
  }};
  filters.appendChild(b);
}});

function renderEvents() {{
  evBox.innerHTML = '';
  TRACE.events.filter(e => activeKind === 'all' || e.kind === activeKind).forEach(e => {{
    const row = document.createElement('div');
    row.className = 'ev ' + e.kind;
    row.innerHTML = `<div><span class="t">${{e.t_ms}}ms</span><span>${{e.title || ''}}</span></div>` +
      (e.detail ? `<div class="d">${{e.detail}}</div>` : '');
    row.onclick = () => {{
      if (typeof e.snapshot_idx === 'number') showSnap(e.snapshot_idx);
      else if (TRACE.snapshots.length) {{
        let best = 0;
        for (let i = 0; i < TRACE.snapshots.length; i++) if (TRACE.snapshots[i].t_ms <= e.t_ms) best = i;
        showSnap(best);
      }}
    }};
    evBox.appendChild(row);
  }});
}}
renderEvents();
</script>
</body>
</html>"""


_TRACERS: Dict[str, TraceRecorder] = {}


def _evict_tracer(session_id: str) -> None:
    rec = _TRACERS.pop(session_id, None)
    if rec is not None:
        rec.close_sync()


async def start_tracing(
    tab: Any,
    screenshots: bool = True,
    network: bool = True,
    console: bool = False,
    page_errors: bool = False,
    screenshot_interval_ms: int = 1000,
) -> TraceRecorder:
    """Start recording a live trace (network, snapshots/screenshots, optional console/errors) on `tab`."""
    sid = tab.session_id
    old = _TRACERS.pop(sid, None)
    if old is not None:
        await old.stop()
    rec = TraceRecorder(
        tab,
        screenshots=screenshots,
        network=network,
        console=console,
        page_errors=page_errors,
        screenshot_interval_ms=screenshot_interval_ms,
    )
    _TRACERS[sid] = rec
    await rec.start()
    return rec


async def trace_snapshot(tab: Any, label: str = "snapshot") -> Optional[Dict[str, Any]]:
    """Capture an immediate labeled screenshot + URL snapshot inside an active trace."""
    rec = _TRACERS.get(tab.session_id)
    if rec is None:
        return None
    return await rec.capture_snapshot(label=label)


async def stop_tracing(tab: Any, path: Optional[PathLike] = None) -> Dict[str, Any]:
    """Stop tracing on `tab`, return the trace dict, and optionally save `.html` (Trace Viewer) or `.json`."""
    rec = _TRACERS.pop(tab.session_id, None)
    trace = (
        await rec.stop()
        if rec is not None
        else {
            "tool": "best-driver",
            "version": "0.4.0",
            "started_at": "",
            "duration_ms": 0.0,
            "event_count": 0,
            "snapshot_count": 0,
            "events": [],
            "snapshots": [],
        }
    )
    if path is not None:
        out_path = os.fspath(path)
        if out_path.lower().endswith(".json"):
            with open(out_path, "w", encoding="utf-8") as fh:
                _json.dump(trace, fh, indent=2)
        else:
            with open(out_path, "w", encoding="utf-8") as fh:
                fh.write(render_trace_html(trace))
    return trace


__all__ = [
    "TraceRecorder",
    "render_trace_html",
    "start_tracing",
    "stop_tracing",
    "trace_snapshot",
]
