"""Popup, console, page-error, and renderer-crash async event streams for `Tab`.

Design invariant:
- `expect_popup()` / `listen_popups()` and `listen_crashes()` NEVER call `Runtime.enable`
  (zero stealth impact).
- `listen_console()` and `listen_page_errors()` use opt-in, reference-counted
  `Runtime.enable` / `Runtime.disable` scoped strictly to the listening tab so default tabs
  never trigger V8 `console.debug(new Error())` anti-bot traps.
"""

from __future__ import annotations

import asyncio
import time
from typing import Any, Dict, Iterable, List, Optional, Set

_RUNTIME_REFS: Dict[str, int] = {}


async def _acquire_runtime(tab: Any) -> None:
    sid = tab.session_id
    count = _RUNTIME_REFS.get(sid, 0)
    _RUNTIME_REFS[sid] = count + 1
    if count == 0:
        await tab.send("Runtime.enable", {})


async def _release_runtime(tab: Any) -> None:
    sid = tab.session_id
    count = _RUNTIME_REFS.get(sid, 0)
    if count <= 1:
        _RUNTIME_REFS.pop(sid, None)
        if count == 1:
            try:
                await tab.send("Runtime.disable", {})
            except Exception:
                pass
    else:
        _RUNTIME_REFS[sid] = count - 1


def _evict_runtime_ref(session_id: str) -> None:
    _RUNTIME_REFS.pop(session_id, None)


def _format_remote_object(obj: Dict[str, Any]) -> str:
    if "value" in obj:
        val = obj["value"]
        return "null" if val is None else str(val)
    if obj.get("unserializableValue") is not None:
        return str(obj["unserializableValue"])
    if obj.get("description") is not None:
        return str(obj["description"])
    return str(obj.get("type", ""))


class ConsoleMessage:
    """JavaScript `console.*` event yielded by `tab.listen_console()`."""

    def __init__(self, params: Dict[str, Any]) -> None:
        self.type: str = str(params.get("type", "log"))
        raw_args = params.get("args") or []
        self.args: List[Dict[str, Any]] = [a for a in raw_args if isinstance(a, dict)]
        self.text: str = " ".join(_format_remote_object(a) for a in self.args)
        self.timestamp: float = float(params.get("timestamp") or 0.0)
        frames = (params.get("stackTrace") or {}).get("callFrames") or []
        top = frames[0] if frames and isinstance(frames[0], dict) else {}
        self.url: str = str(top.get("url", ""))
        self.line: int = int(top.get("lineNumber") or 0)
        self.column: int = int(top.get("columnNumber") or 0)

    def __repr__(self) -> str:
        return f"<ConsoleMessage [{self.type}] {self.text!r}>"


class PageError:
    """Uncaught JavaScript exception or unhandled Promise rejection from `tab.listen_page_errors()`."""

    def __init__(self, params: Dict[str, Any]) -> None:
        details = params.get("exceptionDetails") or {}
        exc = details.get("exception") or {}
        self.name: str = str(exc.get("className") or "Error")
        desc = exc.get("description") or details.get("text") or ""
        self.message: str = str(desc).splitlines()[0] if desc else str(details.get("text") or "")
        self.stack: str = str(desc)
        self.url: str = str(details.get("url") or "")
        self.line: int = int(details.get("lineNumber") or 0)
        self.column: int = int(details.get("columnNumber") or 0)
        self.timestamp: float = float(params.get("timestamp") or 0.0)

    def __repr__(self) -> str:
        return f"<PageError {self.name}: {self.message!r}>"


class CrashEvent:
    """Renderer crash event (`Inspector.targetCrashed`) yielded by `tab.listen_crashes()`."""

    def __init__(self, target_id: str, session_id: str) -> None:
        self.target_id: str = target_id
        self.session_id: str = session_id
        self.timestamp: float = time.time()

    def __repr__(self) -> str:
        return f"<CrashEvent target_id={self.target_id!r}>"


class ConsoleStream:
    """Opt-in async stream of JS console messages (`Runtime.consoleAPICalled`).

    Enables CDP `Runtime` while active and automatically calls `Runtime.disable`
    on exit (`async with` / `await stream.aclose()`) so default stealth is preserved.
    """

    def __init__(self, tab: Any, types: Optional[Iterable[str]] = None) -> None:
        self._tab = tab
        self._types: Optional[Set[str]] = (
            {str(t).lower() for t in types} if types is not None else None
        )
        self._stream = tab._event_stream(["Runtime.consoleAPICalled"])
        self._acquired = False
        self._closed = False

    async def _ensure_started(self) -> None:
        if not self._acquired and not self._closed:
            self._acquired = True
            await _acquire_runtime(self._tab)

    async def next(self, timeout_ms: Optional[int] = None) -> Optional[ConsoleMessage]:
        await self._ensure_started()
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
            msg = ConsoleMessage(ev.get("params") or {})
            if self._types is None or msg.type.lower() in self._types:
                return msg

    def close(self) -> None:
        self._closed = True
        self._stream.close()

    async def aclose(self) -> None:
        self._closed = True
        self._stream.close()
        if self._acquired:
            self._acquired = False
            await _release_runtime(self._tab)

    def __aiter__(self) -> "ConsoleStream":
        return self

    async def __anext__(self) -> ConsoleMessage:
        item = await self.next()
        if item is None:
            raise StopAsyncIteration
        return item

    async def __aenter__(self) -> "ConsoleStream":
        await self._ensure_started()
        return self

    async def __aexit__(self, *args: Any) -> bool:
        await self.aclose()
        return False


class PageErrorStream:
    """Opt-in async stream of uncaught JS exceptions (`Runtime.exceptionThrown`)."""

    def __init__(self, tab: Any) -> None:
        self._tab = tab
        self._stream = tab._event_stream(["Runtime.exceptionThrown"])
        self._acquired = False
        self._closed = False

    async def _ensure_started(self) -> None:
        if not self._acquired and not self._closed:
            self._acquired = True
            await _acquire_runtime(self._tab)

    async def next(self, timeout_ms: Optional[int] = None) -> Optional[PageError]:
        await self._ensure_started()
        ev = await self._stream.next(timeout_ms)
        if not ev:
            return None
        return PageError(ev.get("params") or {})

    def close(self) -> None:
        self._closed = True
        self._stream.close()

    async def aclose(self) -> None:
        self._closed = True
        self._stream.close()
        if self._acquired:
            self._acquired = False
            await _release_runtime(self._tab)

    def __aiter__(self) -> "PageErrorStream":
        return self

    async def __anext__(self) -> PageError:
        item = await self.next()
        if item is None:
            raise StopAsyncIteration
        return item

    async def __aenter__(self) -> "PageErrorStream":
        await self._ensure_started()
        return self

    async def __aexit__(self, *args: Any) -> bool:
        await self.aclose()
        return False


class CrashStream:
    """Stealth-safe async stream of renderer crash events (`Inspector.targetCrashed`)."""

    def __init__(self, tab: Any) -> None:
        self._tab = tab
        self._stream = tab._event_stream(["Inspector.targetCrashed"])

    async def next(self, timeout_ms: Optional[int] = None) -> Optional[CrashEvent]:
        ev = await self._stream.next(timeout_ms)
        if not ev:
            return None
        return CrashEvent(self._tab.target_id, self._tab.session_id)

    def close(self) -> None:
        self._stream.close()

    def __aiter__(self) -> "CrashStream":
        return self

    async def __anext__(self) -> CrashEvent:
        item = await self.next()
        if item is None:
            raise StopAsyncIteration
        return item

    async def __aenter__(self) -> "CrashStream":
        return self

    async def __aexit__(self, *args: Any) -> bool:
        self.close()
        return False


class PopupStream:
    """Stealth-safe async stream of popup `Tab`s opened by a parent `Tab` (`window.open` / `target=_blank`)."""

    def __init__(self, tab: Any) -> None:
        self._tab = tab
        self._stream = tab._popup_event_stream()

    async def next(self, timeout_ms: Optional[int] = None) -> Optional[Any]:
        ev = await self._stream.next(timeout_ms)
        if not ev:
            return None
        info = (ev.get("params") or {}).get("targetInfo") or {}
        tid = info.get("targetId")
        if not tid:
            return None
        ctx_id = info.get("browserContextId") or self._tab.context_id
        return await self._tab._attach_target(str(tid), ctx_id)

    def close(self) -> None:
        self._stream.close()

    def __aiter__(self) -> "PopupStream":
        return self

    async def __anext__(self) -> Any:
        item = await self.next()
        if item is None:
            raise StopAsyncIteration
        return item

    async def __aenter__(self) -> "PopupStream":
        return self

    async def __aexit__(self, *args: Any) -> bool:
        self.close()
        return False


class PopupExpectation:
    """Playwright-style popup waiter supporting both `async with` and direct `await`.

    Usage::

        async with tab.expect_popup(timeout_ms=15000) as popup_info:
            await tab.click("#oauth-login")
        popup = await popup_info.value
    """

    def __init__(self, tab: Any, timeout_ms: int = 30000) -> None:
        self._tab = tab
        self._timeout_ms = timeout_ms
        self._stream: Optional[PopupStream] = None
        self._task: Optional[asyncio.Task[Any]] = None
        self.popup: Optional[Any] = None

    async def _wait(self) -> Any:
        assert self._stream is not None
        try:
            tab = await self._stream.next(self._timeout_ms)
        finally:
            self._stream.close()
        if tab is None:
            raise TimeoutError(f"Timed out after {self._timeout_ms}ms waiting for popup from {self._tab!r}")
        self.popup = tab
        return tab

    @property
    def value(self) -> Any:
        """Awaitable resolving to the newly opened popup `Tab`."""
        async def _get() -> Any:
            if self.popup is not None:
                return self.popup
            if self._task is not None:
                return await self._task
            raise RuntimeError("PopupExpectation was never entered or awaited")

        return _get()

    async def __aenter__(self) -> "PopupExpectation":
        self._stream = PopupStream(self._tab)
        self._task = asyncio.create_task(self._wait())
        return self

    async def __aexit__(self, exc_type: Any, exc: Any, tb: Any) -> bool:
        if exc_type is not None:
            if self._stream is not None:
                self._stream.close()
            if self._task is not None and not self._task.done():
                self._task.cancel()
                try:
                    await self._task
                except BaseException:
                    pass
            return False
        if self._task is not None:
            await self._task
        return False

    def __await__(self) -> Any:
        async def _run() -> Any:
            self._stream = PopupStream(self._tab)
            return await self._wait()

        return _run().__await__()


__all__ = [
    "ConsoleMessage",
    "ConsoleStream",
    "CrashEvent",
    "CrashStream",
    "PageError",
    "PageErrorStream",
    "PopupExpectation",
    "PopupStream",
]
