"""Network routing (request & response interception/mocking), live async streams, and HAR 1.2 recording."""

from __future__ import annotations

import asyncio
import base64
import datetime as _dt
import fnmatch
import inspect
import json as _json
import os
from typing import Any, Awaitable, Callable, Dict, List, Optional, Tuple, Union
from urllib.parse import urlsplit


def _match_pattern(pattern: Optional[str], url: str) -> bool:
    if not pattern or pattern == "*":
        return True
    if "*" in pattern or "?" in pattern:
        return fnmatch.fnmatch(url, pattern)
    return pattern in url


def _split_proxy_creds(raw_proxy: Optional[str]) -> Optional[Tuple[str, str]]:
    if not raw_proxy:
        return None
    parsed = urlsplit(raw_proxy if "://" in raw_proxy else f"http://{raw_proxy}")
    if parsed.username is None:
        return None
    return (parsed.username, parsed.password or "")


class Route:
    """Intercepted CDP Fetch request or response passed to a `tab.route()` callback."""

    def __init__(self, tab: Any, params: Dict[str, Any]) -> None:
        self._tab = tab
        self.request_id: str = str(params.get("requestId", ""))
        req = params.get("request") or {}
        self.url: str = str(req.get("url", ""))
        self.method: str = str(req.get("method", "GET"))
        self.headers: Dict[str, str] = {
            str(k): str(v) for k, v in (req.get("headers") or {}).items()
        }
        self.post_data: Optional[str] = req.get("postData")
        self.resource_type: str = str(params.get("resourceType", ""))
        self.response_status: Optional[int] = params.get("responseStatusCode")
        self.response_headers: Dict[str, str] = {
            str(h.get("name", "")): str(h.get("value", ""))
            for h in (params.get("responseHeaders") or [])
            if isinstance(h, dict) and h.get("name")
        }
        self.stage: str = "response" if self.response_status is not None else "request"
        self.handled: bool = False
        self._resp_body_cache: Optional[bytes] = None

    async def response_body(self) -> bytes:
        """Read the real server response body bytes (available when `stage == 'response'`)."""
        if self._resp_body_cache is not None:
            return self._resp_body_cache
        res = await self._tab.send("Fetch.getResponseBody", {"requestId": self.request_id})
        raw = res.get("body") or ""
        data = base64.b64decode(raw) if res.get("base64Encoded") else str(raw).encode("utf-8")
        self._resp_body_cache = data
        return data

    async def response_text(self, encoding: str = "utf-8") -> str:
        """Decode the intercepted server response body as text."""
        return (await self.response_body()).decode(encoding, errors="replace")

    async def response_json(self) -> Any:
        """Parse the intercepted server response body as JSON."""
        return _json.loads(await self.response_text())

    async def fetch(
        self,
        *,
        url: Optional[str] = None,
        method: Optional[str] = None,
        headers: Optional[Dict[str, str]] = None,
        body: Optional[str] = None,
    ) -> Dict[str, Any]:
        """Perform the real upstream HTTP request via `http_tls` (if installed) or `tab.fetch()`."""
        target_url = url or self.url
        target_method = method or self.method
        target_headers = headers if headers is not None else dict(self.headers)
        target_body = body if body is not None else self.post_data
        try:
            async with await self._tab.tls_session(async_mode=True) as tls:
                resp = await tls.request(
                    target_method,
                    target_url,
                    headers=target_headers,
                    data=target_body,
                )
                return {
                    "url": resp.url,
                    "status": resp.status_code,
                    "status_text": resp.reason,
                    "ok": resp.ok,
                    "headers": dict(resp.headers),
                    "body": resp.text,
                }
        except ImportError:
            return await self._tab.fetch(
                target_url,
                method=target_method,
                headers=target_headers,
                body=target_body,
            )

    async def continue_(
        self,
        *,
        url: Optional[str] = None,
        method: Optional[str] = None,
        headers: Optional[Dict[str, str]] = None,
        post_data: Optional[Union[str, bytes]] = None,
    ) -> None:
        """Allow the intercepted request (or response) to proceed, with optional overrides."""
        if self.handled:
            return
        if self.stage == "response":
            await self.continue_response(headers=headers)
            return
        self.handled = True
        payload: Dict[str, Any] = {"requestId": self.request_id}
        if url is not None:
            payload["url"] = url
        if method is not None:
            payload["method"] = method
        if headers is not None:
            payload["headers"] = [{"name": str(k), "value": str(v)} for k, v in headers.items()]
        if post_data is not None:
            raw = post_data.encode("utf-8") if isinstance(post_data, str) else bytes(post_data)
            payload["postData"] = base64.b64encode(raw).decode("ascii")
        await self._tab.send("Fetch.continueRequest", payload)

    async def continue_response(
        self,
        *,
        status: Optional[int] = None,
        headers: Optional[Dict[str, str]] = None,
    ) -> None:
        """Continue an intercepted response (`stage == 'response'`) with optional status/header edits."""
        if self.handled:
            return
        self.handled = True
        payload: Dict[str, Any] = {"requestId": self.request_id}
        if status is not None:
            payload["responseCode"] = int(status)
        if headers is not None:
            payload["responseHeaders"] = [
                {"name": str(k), "value": str(v)} for k, v in headers.items()
            ]
        await self._tab.send("Fetch.continueResponse", payload)

    async def fulfill(
        self,
        *,
        status: int = 200,
        body: Union[str, bytes] = "",
        headers: Optional[Dict[str, str]] = None,
        content_type: Optional[str] = None,
        json: Any = None,
    ) -> None:
        """Fulfill the intercepted request or replace the intercepted server response."""
        if self.handled:
            return
        self.handled = True
        hdrs: Dict[str, str] = {str(k): str(v) for k, v in (headers or {}).items()}
        if json is not None:
            raw = _json.dumps(json).encode("utf-8")
            if not any(k.lower() == "content-type" for k in hdrs):
                hdrs["Content-Type"] = content_type or "application/json; charset=utf-8"
        else:
            raw = body.encode("utf-8") if isinstance(body, str) else bytes(body)
            if content_type and not any(k.lower() == "content-type" for k in hdrs):
                hdrs["Content-Type"] = content_type

        payload: Dict[str, Any] = {
            "requestId": self.request_id,
            "responseCode": int(status),
            "responseHeaders": [{"name": k, "value": v} for k, v in hdrs.items()],
            "body": base64.b64encode(raw).decode("ascii"),
        }
        await self._tab.send("Fetch.fulfillRequest", payload)

    async def abort(self, error_reason: str = "Failed") -> None:
        """Abort the intercepted request with a Chromium network error reason."""
        if self.handled:
            return
        self.handled = True
        await self._tab.send(
            "Fetch.failRequest",
            {"requestId": self.request_id, "errorReason": error_reason},
        )


RouteHandler = Callable[[Route], Union[None, Awaitable[None]]]


class _RouteController:
    def __init__(self, tab: Any) -> None:
        self.tab = tab
        self.routes: List[Tuple[str, RouteHandler, str]] = []
        self._stream: Any = None
        self._task: Optional[asyncio.Task[None]] = None
        self._proxy_creds = _split_proxy_creds(getattr(tab, "proxy", None))
        self._response_enabled: bool = False

    async def start(self) -> None:
        need_response = any(stage == "response" for _, _, stage in self.routes)
        patterns: List[Dict[str, str]] = [{"urlPattern": "*", "requestStage": "Request"}]
        if need_response:
            patterns.append({"urlPattern": "*", "requestStage": "Response"})

        if self._task is not None and not self._task.done():
            if need_response != self._response_enabled:
                self._response_enabled = need_response
                await self.tab.send(
                    "Fetch.enable",
                    {"patterns": patterns, "handleAuthRequests": True},
                )
            return

        self.tab._abort_auth_task()
        self._stream = self.tab._event_stream(["Fetch.requestPaused", "Fetch.authRequired"])
        self._response_enabled = need_response
        await self.tab.send(
            "Fetch.enable",
            {"patterns": patterns, "handleAuthRequests": True},
        )
        self._task = asyncio.create_task(self._loop())

    async def stop(self) -> None:
        if self._stream is not None:
            self._stream.close()
        if self._task is not None:
            self._task.cancel()
            try:
                await self._task
            except BaseException:
                pass
            self._task = None
        self._stream = None
        self._response_enabled = False
        try:
            await self.tab.send("Fetch.disable", {})
        except Exception:
            pass

    async def _dispatch_pause(self, params: Dict[str, Any]) -> None:
        route = Route(self.tab, params)
        try:
            for pattern, handler, stage in list(self.routes):
                if stage == route.stage and _match_pattern(pattern, route.url):
                    res = handler(route)
                    if inspect.isawaitable(res):
                        await res
                    if route.handled:
                        return
        except Exception:
            pass
        if not route.handled:
            try:
                if route.stage == "response":
                    await route.continue_response()
                else:
                    await route.continue_()
            except Exception:
                pass

    async def _loop(self) -> None:
        assert self._stream is not None
        sid = self.tab.session_id
        try:
            while True:
                ev = await self._stream.next()
                if not ev:
                    break
                method = ev.get("method")
                params = ev.get("params") or {}
                if method == "Fetch.requestPaused":
                    asyncio.create_task(self._dispatch_pause(params))
                elif method == "Fetch.authRequired":
                    rid = params.get("requestId")
                    if not rid:
                        continue
                    is_proxy = (params.get("authChallenge") or {}).get("source") == "Proxy"
                    if is_proxy and self._proxy_creds is not None:
                        resp = {
                            "response": "ProvideCredentials",
                            "username": self._proxy_creds[0],
                            "password": self._proxy_creds[1],
                        }
                    else:
                        resp = {"response": "Default"}
                    asyncio.create_task(
                        self.tab.send(
                            "Fetch.continueWithAuth",
                            {"requestId": rid, "authChallengeResponse": resp},
                        )
                    )
        finally:
            if self._stream is not None:
                self._stream.close()
            if _CONTROLLERS.get(sid) is self:
                _CONTROLLERS.pop(sid, None)


_CONTROLLERS: Dict[str, _RouteController] = {}


async def add_route(
    tab: Any,
    url_pattern: str,
    handler: RouteHandler,
    stage: str = "request",
) -> None:
    """Register a request or response interceptor (`stage='request'|'response'`) on `tab`."""
    norm_stage = stage.strip().lower()
    if norm_stage not in ("request", "response"):
        raise ValueError(f"Unsupported route stage {stage!r}; expected 'request' or 'response'")
    sid = tab.session_id
    ctrl = _CONTROLLERS.get(sid)
    if ctrl is None:
        ctrl = _RouteController(tab)
        _CONTROLLERS[sid] = ctrl
    ctrl.routes.append((url_pattern, handler, norm_stage))
    await ctrl.start()


async def remove_route(tab: Any, url_pattern: Optional[str] = None) -> None:
    """Remove matching route(s) (or all routes when `url_pattern=None`) from `tab`."""
    sid = tab.session_id
    ctrl = _CONTROLLERS.get(sid)
    if ctrl is None:
        return
    if url_pattern is None:
        ctrl.routes.clear()
    else:
        ctrl.routes = [(p, h, st) for (p, h, st) in ctrl.routes if p != url_pattern]
    if not ctrl.routes:
        await ctrl.stop()
        _CONTROLLERS.pop(sid, None)
    else:
        await ctrl.start()


class StreamedRequest:
    """Outgoing HTTP request yielded by `tab.listen_requests()`."""

    def __init__(self, params: Dict[str, Any]) -> None:
        req = params.get("request") or {}
        self.request_id: str = str(params.get("requestId", ""))
        self.url: str = str(req.get("url", ""))
        self.method: str = str(req.get("method", "GET"))
        self.headers: Dict[str, str] = {
            str(k): str(v) for k, v in (req.get("headers") or {}).items()
        }
        self.post_data: Optional[str] = req.get("postData")
        self.resource_type: str = str(params.get("type", ""))
        self.timestamp: float = float(params.get("timestamp") or 0.0)

    def __repr__(self) -> str:
        return f"<StreamedRequest {self.method} {self.url!r}>"


class StreamedResponse:
    """HTTP response yielded by `tab.listen_responses()` with lazy body decoding."""

    def __init__(self, tab: Any, params: Dict[str, Any]) -> None:
        self._tab = tab
        resp = params.get("response") or {}
        self.request_id: str = str(params.get("requestId", ""))
        self.url: str = str(resp.get("url", ""))
        self.status: int = int(resp.get("status") or 0)
        self.status_text: str = str(resp.get("statusText", ""))
        self.headers: Dict[str, str] = {
            str(k): str(v) for k, v in (resp.get("headers") or {}).items()
        }
        self.mime_type: str = str(resp.get("mimeType", ""))
        self.remote_ip: str = str(resp.get("remoteIPAddress", ""))
        self.protocol: str = str(resp.get("protocol", ""))
        self.from_cache: bool = bool(resp.get("fromDiskCache") or resp.get("fromServiceWorker"))
        self._body_cache: Optional[bytes] = None

    async def body(self) -> bytes:
        """Fetch raw response body bytes from Chromium via `Network.getResponseBody`."""
        if self._body_cache is not None:
            return self._body_cache
        last_err: Optional[Exception] = None
        for _ in range(12):
            try:
                res = await self._tab.send(
                    "Network.getResponseBody", {"requestId": self.request_id}
                )
                raw = res.get("body") or ""
                data = (
                    base64.b64decode(raw)
                    if res.get("base64Encoded")
                    else str(raw).encode("utf-8")
                )
                self._body_cache = data
                return data
            except Exception as exc:
                last_err = exc
                await asyncio.sleep(0.05)
        raise RuntimeError(f"Failed to read response body for {self.url}: {last_err}")

    async def text(self, encoding: str = "utf-8") -> str:
        return (await self.body()).decode(encoding, errors="replace")

    async def json(self) -> Any:
        return _json.loads(await self.text())

    def __repr__(self) -> str:
        return f"<StreamedResponse {self.status} {self.url!r}>"


class RequestStream:
    """Async stream of outgoing HTTP requests on a `Tab`."""

    def __init__(self, tab: Any, url_pattern: Optional[str] = None) -> None:
        self._tab = tab
        self._pattern = url_pattern
        self._stream = tab._event_stream(["Network.requestWillBeSent"])

    async def next(self, timeout_ms: Optional[int] = None) -> Optional[StreamedRequest]:
        loop = asyncio.get_running_loop()
        deadline = None if timeout_ms is None else loop.time() + timeout_ms / 1000.0
        while True:
            rem_ms: Optional[int] = None
            if deadline is not None:
                left = deadline - loop.time()
                if left <= 0:
                    return None
                rem_ms = max(1, int(left * 1000))
            ev = await self._stream.next(rem_ms)
            if not ev:
                return None
            req = StreamedRequest(ev.get("params") or {})
            if _match_pattern(self._pattern, req.url):
                return req

    def close(self) -> None:
        self._stream.close()

    def __aiter__(self) -> "RequestStream":
        return self

    async def __anext__(self) -> StreamedRequest:
        item = await self.next()
        if item is None:
            raise StopAsyncIteration
        return item

    async def __aenter__(self) -> "RequestStream":
        return self

    async def __aexit__(self, *args: Any) -> bool:
        self.close()
        return False


class ResponseStream:
    """Async stream of incoming HTTP responses on a `Tab`."""

    def __init__(self, tab: Any, url_pattern: Optional[str] = None) -> None:
        self._tab = tab
        self._pattern = url_pattern
        self._stream = tab._event_stream(["Network.responseReceived"])

    async def next(self, timeout_ms: Optional[int] = None) -> Optional[StreamedResponse]:
        loop = asyncio.get_running_loop()
        deadline = None if timeout_ms is None else loop.time() + timeout_ms / 1000.0
        while True:
            rem_ms: Optional[int] = None
            if deadline is not None:
                left = deadline - loop.time()
                if left <= 0:
                    return None
                rem_ms = max(1, int(left * 1000))
            ev = await self._stream.next(rem_ms)
            if not ev:
                return None
            resp = StreamedResponse(self._tab, ev.get("params") or {})
            if _match_pattern(self._pattern, resp.url):
                return resp

    def close(self) -> None:
        self._stream.close()

    def __aiter__(self) -> "ResponseStream":
        return self

    async def __anext__(self) -> StreamedResponse:
        item = await self.next()
        if item is None:
            raise StopAsyncIteration
        return item

    async def __aenter__(self) -> "ResponseStream":
        return self

    async def __aexit__(self, *args: Any) -> bool:
        self.close()
        return False


class HarRecorder:
    """Background HAR 1.2 traffic recorder attached to a `Tab`."""

    def __init__(self, tab: Any, include_bodies: bool = True) -> None:
        self._tab = tab
        self._include_bodies = include_bodies
        self._stream = tab._event_stream(
            [
                "Network.requestWillBeSent",
                "Network.responseReceived",
                "Network.loadingFinished",
                "Network.loadingFailed",
            ]
        )
        self._pending: Dict[str, Dict[str, Any]] = {}
        self.entries: List[Dict[str, Any]] = []
        self._task: Optional[asyncio.Task[None]] = None

    def start(self) -> None:
        if self._task is None or self._task.done():
            self._task = asyncio.create_task(self._loop())

    async def stop(self) -> None:
        self._stream.close()
        if self._task is not None:
            self._task.cancel()
            try:
                await self._task
            except BaseException:
                pass
            self._task = None

    async def _loop(self) -> None:
        sid = self._tab.session_id
        try:
            while True:
                ev = await self._stream.next()
                if not ev:
                    break
                method = ev.get("method")
                params = ev.get("params") or {}
                rid = str(params.get("requestId", ""))
                if not rid:
                    continue
                if method == "Network.requestWillBeSent":
                    req = params.get("request") or {}
                    self._pending[rid] = {
                        "started": _dt.datetime.now(_dt.timezone.utc).isoformat(),
                        "t0": float(params.get("timestamp") or 0.0),
                        "method": str(req.get("method", "GET")),
                        "url": str(req.get("url", "")),
                        "req_headers": [
                            {"name": str(k), "value": str(v)}
                            for k, v in (req.get("headers") or {}).items()
                        ],
                        "post_data": req.get("postData"),
                    }
                elif method == "Network.responseReceived":
                    rec = self._pending.get(rid)
                    if rec is None:
                        continue
                    resp = params.get("response") or {}
                    rec["status"] = int(resp.get("status") or 0)
                    rec["status_text"] = str(resp.get("statusText", ""))
                    rec["protocol"] = str(resp.get("protocol", "HTTP/1.1"))
                    rec["mime_type"] = str(resp.get("mimeType", ""))
                    rec["remote_ip"] = str(resp.get("remoteIPAddress", ""))
                    rec["resp_headers"] = [
                        {"name": str(k), "value": str(v)}
                        for k, v in (resp.get("headers") or {}).items()
                    ]
                elif method == "Network.loadingFailed":
                    self._pending.pop(rid, None)
                elif method == "Network.loadingFinished":
                    rec = self._pending.pop(rid, None)
                    if rec is None:
                        continue
                    t1 = float(params.get("timestamp") or rec["t0"])
                    elapsed_ms = max(0.0, round((t1 - rec["t0"]) * 1000.0, 2))
                    body_text = ""
                    body_encoding = None
                    if self._include_bodies:
                        try:
                            res = await self._tab.send("Network.getResponseBody", {"requestId": rid})
                            body_text = str(res.get("body") or "")
                            if res.get("base64Encoded"):
                                body_encoding = "base64"
                        except Exception:
                            pass
                    content: Dict[str, Any] = {
                        "size": int(params.get("encodedDataLength") or len(body_text)),
                        "mimeType": rec.get("mime_type", ""),
                        "text": body_text,
                    }
                    if body_encoding:
                        content["encoding"] = body_encoding
                    entry: Dict[str, Any] = {
                        "startedDateTime": rec["started"],
                        "time": elapsed_ms,
                        "serverIPAddress": rec.get("remote_ip", ""),
                        "request": {
                            "method": rec["method"],
                            "url": rec["url"],
                            "httpVersion": rec.get("protocol", "HTTP/1.1"),
                            "headers": rec["req_headers"],
                            "cookies": [],
                            "queryString": [],
                            "headersSize": -1,
                            "bodySize": len(rec["post_data"]) if rec.get("post_data") else 0,
                        },
                        "response": {
                            "status": rec.get("status", 0),
                            "statusText": rec.get("status_text", ""),
                            "httpVersion": rec.get("protocol", "HTTP/1.1"),
                            "headers": rec.get("resp_headers", []),
                            "cookies": [],
                            "content": content,
                            "redirectURL": "",
                            "headersSize": -1,
                            "bodySize": int(params.get("encodedDataLength") or 0),
                        },
                        "cache": {},
                        "timings": {"send": 0, "wait": elapsed_ms, "receive": 0},
                    }
                    self.entries.append(entry)
        finally:
            self._stream.close()
            if _HAR_RECORDERS.get(sid) is self:
                _HAR_RECORDERS.pop(sid, None)

    def to_har(self) -> Dict[str, Any]:
        return {
            "log": {
                "version": "1.2",
                "creator": {"name": "best-driver", "version": "0.3.3"},
                "entries": list(self.entries),
            }
        }


_HAR_RECORDERS: Dict[str, HarRecorder] = {}


def _cancel_task_safe(task: Optional[asyncio.Task[Any]]) -> None:
    if task is None or task.done():
        return
    try:
        running = asyncio.get_running_loop()
    except RuntimeError:
        running = None
    loop = task.get_loop()
    if running is loop:
        task.cancel()
    elif not loop.is_closed():
        loop.call_soon_threadsafe(task.cancel)


def _evict_session(session_id: str) -> None:
    """Synchronously detach route/HAR/event controllers for a closed tab session (thread-safe)."""
    ctrl = _CONTROLLERS.pop(session_id, None)
    if ctrl is not None:
        if ctrl._stream is not None:
            ctrl._stream.close()
        _cancel_task_safe(ctrl._task)
        ctrl._task = None
        ctrl._stream = None
    rec = _HAR_RECORDERS.pop(session_id, None)
    if rec is not None:
        rec._stream.close()
        _cancel_task_safe(rec._task)
        rec._task = None
    try:
        from .events import _evict_runtime_ref

        _evict_runtime_ref(session_id)
    except Exception:
        pass


def _evict_context(context_id: str) -> None:
    """Synchronously detach route/HAR controllers for every tab inside `context_id`."""
    sids = {
        k
        for k, v in list(_CONTROLLERS.items())
        if getattr(v.tab, "context_id", None) == context_id
    }
    sids.update(
        k
        for k, v in list(_HAR_RECORDERS.items())
        if getattr(v._tab, "context_id", None) == context_id
    )
    for sid in sids:
        _evict_session(sid)


def _evict_sessions(session_ids: Any) -> None:
    """Synchronously detach route/HAR controllers for a batch of closed tab sessions."""
    for sid in session_ids:
        _evict_session(str(sid))


async def start_har(tab: Any, include_bodies: bool = True) -> HarRecorder:
    """Begin recording HAR 1.2 network transactions on `tab`."""
    sid = tab.session_id
    old = _HAR_RECORDERS.pop(sid, None)
    if old is not None:
        await old.stop()
    rec = HarRecorder(tab, include_bodies=include_bodies)
    _HAR_RECORDERS[sid] = rec
    rec.start()
    return rec


async def export_har(
    tab: Any,
    path: Optional[Union[str, os.PathLike[str]]] = None,
) -> Dict[str, Any]:
    """Export recorded HAR 1.2 dictionary (and optionally write to `path`)."""
    rec = _HAR_RECORDERS.get(tab.session_id)
    har = (
        rec.to_har()
        if rec is not None
        else {"log": {"version": "1.2", "creator": {"name": "best-driver", "version": "0.3.3"}, "entries": []}}
    )
    if path is not None:
        with open(os.fspath(path), "w", encoding="utf-8") as fh:
            _json.dump(har, fh, indent=2)
    return har


async def stop_har(tab: Any) -> Dict[str, Any]:
    """Stop HAR 1.2 recording on `tab` and return the captured HAR dictionary."""
    rec = _HAR_RECORDERS.pop(tab.session_id, None)
    if rec is None:
        return {"log": {"version": "1.2", "creator": {"name": "best-driver", "version": "0.3.3"}, "entries": []}}
    await rec.stop()
    return rec.to_har()


__all__ = [
    "HarRecorder",
    "Route",
    "RouteHandler",
    "RequestStream",
    "ResponseStream",
    "StreamedRequest",
    "StreamedResponse",
    "add_route",
    "remove_route",
    "start_har",
    "export_har",
    "stop_har",
]
