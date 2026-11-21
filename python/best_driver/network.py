"""Network routing (request interception/mocking) and live async request/response streams."""

from __future__ import annotations

import asyncio
import base64
import fnmatch
import inspect
import json as _json
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
    """Intercepted CDP Fetch request passed to a `tab.route(pattern, handler)` callback."""

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
        self.handled: bool = False

    async def continue_(
        self,
        *,
        url: Optional[str] = None,
        method: Optional[str] = None,
        headers: Optional[Dict[str, str]] = None,
        post_data: Optional[Union[str, bytes]] = None,
    ) -> None:
        """Allow the request to proceed, optionally modifying URL, method, headers, or body."""
        if self.handled:
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

    async def fulfill(
        self,
        *,
        status: int = 200,
        body: Union[str, bytes] = "",
        headers: Optional[Dict[str, str]] = None,
        content_type: Optional[str] = None,
        json: Any = None,
    ) -> None:
        """Fulfill the request directly with a synthetic response (mocking API/HTML/assets)."""
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
        self.routes: List[Tuple[str, RouteHandler]] = []
        self._stream: Any = None
        self._task: Optional[asyncio.Task[None]] = None
        self._proxy_creds = _split_proxy_creds(getattr(tab, "proxy", None))

    async def start(self) -> None:
        if self._task is not None and not self._task.done():
            return
        # Take clean single-owner control over Fetch events so proxy auth & routes never race
        self.tab._abort_auth_task()
        self._stream = self.tab._event_stream(["Fetch.requestPaused", "Fetch.authRequired"])
        await self.tab.send(
            "Fetch.enable",
            {"patterns": [{"urlPattern": "*"}], "handleAuthRequests": True},
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
        try:
            await self.tab.send("Fetch.disable", {})
        except Exception:
            pass

    async def _dispatch_pause(self, params: Dict[str, Any]) -> None:
        route = Route(self.tab, params)
        try:
            for pattern, handler in list(self.routes):
                if _match_pattern(pattern, route.url):
                    res = handler(route)
                    if inspect.isawaitable(res):
                        await res
                    if route.handled:
                        return
        except Exception:
            pass
        if not route.handled:
            try:
                await route.continue_()
            except Exception:
                pass

    async def _loop(self) -> None:
        assert self._stream is not None
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


_CONTROLLERS: Dict[str, _RouteController] = {}


async def add_route(tab: Any, url_pattern: str, handler: RouteHandler) -> None:
    """Register a request interceptor for `url_pattern` on `tab`."""
    sid = tab.session_id
    ctrl = _CONTROLLERS.get(sid)
    if ctrl is None:
        ctrl = _RouteController(tab)
        _CONTROLLERS[sid] = ctrl
    ctrl.routes.append((url_pattern, handler))
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
        ctrl.routes = [(p, h) for (p, h) in ctrl.routes if p != url_pattern]
    if not ctrl.routes:
        await ctrl.stop()
        _CONTROLLERS.pop(sid, None)


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
        # Body may still be streaming right when responseReceived fires; retry briefly
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
        """Decode response body as text."""
        return (await self.body()).decode(encoding, errors="replace")

    async def json(self) -> Any:
        """Parse response body as JSON."""
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


__all__ = [
    "Route",
    "RouteHandler",
    "RequestStream",
    "ResponseStream",
    "StreamedRequest",
    "StreamedResponse",
    "add_route",
    "remove_route",
]
