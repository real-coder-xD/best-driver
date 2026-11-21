"""Lazy, auto-retrying chained Locator resilient to SPA DOM re-renders and Shadow DOM."""

from __future__ import annotations

import asyncio
import time
from typing import Any, List, Optional, Tuple


class Locator:
    """Playwright-style lazy Smart Locator chain that re-resolves on every action.

    Unlike a snapshot `Element`, a `Locator` transparently re-queries the DOM (including
    both open and closed `#shadow-root` trees) and retries transient `StaleElement` errors
    when React, Vue, or Next.js re-renders a subtree mid-interaction.
    """

    def __init__(
        self,
        tab: Any,
        steps: List[Tuple[str, Optional[int]]] | List[str],
    ) -> None:
        self._tab = tab
        self._steps: List[Tuple[str, Optional[int]]] = [
            (s, None) if isinstance(s, str) else s for s in steps
        ]

    def __repr__(self) -> str:
        chain = " >> ".join(
            f"{q}[{idx}]" if idx is not None else q for q, idx in self._steps
        )
        return f"<Locator {chain!r}>"

    def locator(self, query: str) -> "Locator":
        """Chain a descendant Smart Locator query (CSS, XPath, `text:`, `@attr`, Shadow DOM)."""
        return Locator(self._tab, [*self._steps, (query, None)])

    @property
    def first(self) -> "Locator":
        """Narrow the current step to its first matched element (`nth(0)`)."""
        return self.nth(0)

    @property
    def last(self) -> "Locator":
        """Narrow the current step to its last matched element (`nth(-1)`)."""
        return self.nth(-1)

    def nth(self, index: int) -> "Locator":
        """Select the `index`-th match (supports negative indexing) at the current step."""
        steps = list(self._steps)
        q, _ = steps[-1]
        steps[-1] = (q, index)
        return Locator(self._tab, steps)

    async def _resolve_all_once(self) -> List[Any]:
        if not self._steps:
            return []
        q0, idx0 = self._steps[0]
        nodes: List[Any] = await self._tab.eles(q0, timeout_ms=0)
        if idx0 is not None:
            try:
                nodes = [nodes[idx0]]
            except IndexError:
                return []
        for q, idx in self._steps[1:]:
            next_nodes: List[Any] = []
            for parent in nodes:
                try:
                    found = await parent.eles(q)
                except RuntimeError:
                    continue
                next_nodes.extend(found)
            if not next_nodes:
                return []
            if idx is not None:
                try:
                    nodes = [next_nodes[idx]]
                except IndexError:
                    return []
            else:
                nodes = next_nodes
        return nodes

    async def _resolve_one(self, timeout_ms: int = 10000) -> Any:
        deadline = time.monotonic() + timeout_ms / 1000.0
        while True:
            nodes = await self._resolve_all_once()
            if nodes:
                return nodes[0]
            if time.monotonic() >= deadline:
                raise TimeoutError(f"Locator timed out after {timeout_ms}ms: {self!r}")
            await asyncio.sleep(0.08)

    async def _run_with_retry(self, timeout_ms: int, fn: Any) -> Any:
        deadline = time.monotonic() + timeout_ms / 1000.0
        last_err: Optional[Exception] = None
        while True:
            remaining_ms = max(0, int((deadline - time.monotonic()) * 1000))
            el = await self._resolve_one(timeout_ms=remaining_ms)
            try:
                return await fn(el)
            except RuntimeError as exc:
                last_err = exc
                if time.monotonic() >= deadline:
                    raise TimeoutError(
                        f"Locator action timed out after {timeout_ms}ms ({self!r}): {last_err}"
                    ) from exc
                await asyncio.sleep(0.06)

    async def element(self, timeout_ms: int = 10000) -> Optional[Any]:
        """Resolve and return the first matching `Element` handle, or `None` on timeout."""
        try:
            return await self._resolve_one(timeout_ms=timeout_ms)
        except TimeoutError:
            return None

    async def all(self, timeout_ms: int = 10000) -> List[Any]:
        """Wait until at least one element matches (up to `timeout_ms`) and return all matches."""
        deadline = time.monotonic() + timeout_ms / 1000.0
        while True:
            nodes = await self._resolve_all_once()
            if nodes or timeout_ms == 0 or time.monotonic() >= deadline:
                return nodes
            await asyncio.sleep(0.08)

    async def count(self) -> int:
        """Return the current number of matched elements without waiting."""
        return len(await self._resolve_all_once())

    async def exists(self) -> bool:
        """Return `True` if at least one matching element currently exists."""
        return (await self.count()) > 0

    async def wait_for(self, timeout_ms: int = 10000) -> None:
        """Wait until at least one element matching this locator chain exists."""
        await self._resolve_one(timeout_ms=timeout_ms)

    async def wait_for_hidden(self, timeout_ms: int = 10000) -> None:
        """Wait until no elements match this locator chain."""
        deadline = time.monotonic() + timeout_ms / 1000.0
        while True:
            if not await self._resolve_all_once():
                return
            if time.monotonic() >= deadline:
                raise TimeoutError(
                    f"Locator wait_for_hidden timed out after {timeout_ms}ms: {self!r}"
                )
            await asyncio.sleep(0.08)

    async def click(
        self,
        button: str = "left",
        click_count: int = 1,
        timeout_ms: int = 10000,
    ) -> None:
        """Resolve lazily and click via Cubic Bézier cursor physics (auto-retries stale nodes)."""
        await self._run_with_retry(
            timeout_ms, lambda el: el.click(button=button, click_count=click_count)
        )

    async def double_click(self, timeout_ms: int = 10000) -> None:
        await self.click(button="left", click_count=2, timeout_ms=timeout_ms)

    async def right_click(self, timeout_ms: int = 10000) -> None:
        await self.click(button="right", click_count=1, timeout_ms=timeout_ms)

    async def hover(self, timeout_ms: int = 10000) -> None:
        await self._run_with_retry(timeout_ms, lambda el: el.hover())

    async def human_type(
        self,
        text: str,
        clear: bool = False,
        timeout_ms: int = 10000,
    ) -> None:
        await self._run_with_retry(timeout_ms, lambda el: el.human_type(text, clear=clear))

    async def fill(self, text: str, timeout_ms: int = 10000) -> None:
        await self._run_with_retry(timeout_ms, lambda el: el.fill(text))

    async def clear(self, timeout_ms: int = 10000) -> None:
        await self._run_with_retry(timeout_ms, lambda el: el.clear())

    async def text(self, timeout_ms: int = 10000) -> str:
        el = await self._resolve_one(timeout_ms=timeout_ms)
        return str(el.text)

    async def attr(self, name: str, timeout_ms: int = 10000) -> Optional[str]:
        return await self._run_with_retry(timeout_ms, lambda el: el.attr(name))

    async def value(self, timeout_ms: int = 10000) -> Optional[str]:
        el = await self._resolve_one(timeout_ms=timeout_ms)
        return el.value

    async def inner_html(self, timeout_ms: int = 10000) -> Optional[str]:
        return await self._run_with_retry(timeout_ms, lambda el: el.inner_html())

    async def outer_html(self, timeout_ms: int = 10000) -> str:
        return await self._run_with_retry(timeout_ms, lambda el: el.outer_html())

    async def scroll_into_view(self, timeout_ms: int = 10000) -> None:
        await self._run_with_retry(timeout_ms, lambda el: el.scroll_into_view())

    async def screenshot(
        self,
        file_path: Optional[str] = None,
        timeout_ms: int = 10000,
    ) -> bytes:
        return await self._run_with_retry(timeout_ms, lambda el: el.screenshot(file_path))


__all__ = ["Locator"]
