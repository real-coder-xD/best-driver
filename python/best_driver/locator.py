"""Lazy, auto-retrying chained Locator resilient to SPA DOM re-renders and Shadow DOM."""

from __future__ import annotations

import asyncio
import time
from typing import Any, List, Literal, Optional, Tuple

LocatorState = Literal["attached", "visible", "hidden", "detached"]


class _Step:
    __slots__ = ("query", "index", "has_text", "has_not_text")

    def __init__(
        self,
        query: str,
        index: Optional[int] = None,
        has_text: Optional[str] = None,
        has_not_text: Optional[str] = None,
    ) -> None:
        self.query = query
        self.index = index
        self.has_text = has_text
        self.has_not_text = has_not_text

    def clone(
        self,
        *,
        index: Any = ...,
        has_text: Any = ...,
        has_not_text: Any = ...,
    ) -> "_Step":
        return _Step(
            self.query,
            self.index if index is ... else index,
            self.has_text if has_text is ... else has_text,
            self.has_not_text if has_not_text is ... else has_not_text,
        )


class Locator:
    """Playwright-style lazy Smart Locator chain that re-resolves on every action.

    Unlike a snapshot `Element`, a `Locator` transparently re-queries the DOM (including
    both open and closed `#shadow-root` trees) and retries transient `StaleElement` errors
    when React, Vue, or Next.js re-renders a subtree mid-interaction.
    """

    def __init__(
        self,
        tab: Any,
        steps: List[Any],
    ) -> None:
        self._tab = tab
        self._steps: List[_Step] = []
        for s in steps:
            if isinstance(s, _Step):
                self._steps.append(s)
            elif isinstance(s, tuple):
                self._steps.append(_Step(s[0], s[1]))
            else:
                self._steps.append(_Step(str(s)))

    def __repr__(self) -> str:
        parts = []
        for st in self._steps:
            s = st.query
            if st.has_text is not None:
                s += f":has-text({st.has_text!r})"
            if st.has_not_text is not None:
                s += f":not-text({st.has_not_text!r})"
            if st.index is not None:
                s += f"[{st.index}]"
            parts.append(s)
        return f"<Locator {' >> '.join(parts)!r}>"

    def locator(self, query: str) -> "Locator":
        """Chain a descendant Smart Locator query (CSS, XPath, `text:`, `@attr`, Shadow DOM)."""
        return Locator(self._tab, [*self._steps, _Step(query)])

    def filter(
        self,
        *,
        has_text: Optional[str] = None,
        has_not_text: Optional[str] = None,
    ) -> "Locator":
        """Filter the current step's matched elements by substring inclusion/exclusion (case-insensitive)."""
        steps = list(self._steps)
        steps[-1] = steps[-1].clone(
            has_text=has_text if has_text is not None else steps[-1].has_text,
            has_not_text=has_not_text if has_not_text is not None else steps[-1].has_not_text,
        )
        return Locator(self._tab, steps)

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
        steps[-1] = steps[-1].clone(index=index)
        return Locator(self._tab, steps)

    @staticmethod
    def _apply_step_filters(nodes: List[Any], step: _Step) -> List[Any]:
        filtered = nodes
        if step.has_text is not None:
            needle = step.has_text.lower()
            filtered = [n for n in filtered if needle in (n.text or "").lower()]
        if step.has_not_text is not None:
            needle = step.has_not_text.lower()
            filtered = [n for n in filtered if needle not in (n.text or "").lower()]
        if step.index is not None:
            try:
                return [filtered[step.index]]
            except IndexError:
                return []
        return filtered

    async def _resolve_all_once(self) -> List[Any]:
        if not self._steps:
            return []
        s0 = self._steps[0]
        nodes: List[Any] = await self._tab.eles(s0.query, timeout_ms=0)
        nodes = self._apply_step_filters(nodes, s0)
        if not nodes:
            return []
        for st in self._steps[1:]:
            next_nodes: List[Any] = []
            for parent in nodes:
                try:
                    found = await parent.eles(st.query)
                except RuntimeError:
                    continue
                next_nodes.extend(found)
            nodes = self._apply_step_filters(next_nodes, st)
            if not nodes:
                return []
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
        return len(await self._resolve_all_once())

    async def exists(self) -> bool:
        return (await self.count()) > 0

    async def is_visible(self) -> bool:
        """Return `True` if the matched element exists and has non-zero visible layout."""
        nodes = await self._resolve_all_once()
        if not nodes:
            return False
        try:
            return bool(await nodes[0].is_visible())
        except RuntimeError:
            return False

    async def is_enabled(self) -> bool:
        """Return `True` if the matched element is not disabled."""
        nodes = await self._resolve_all_once()
        if not nodes:
            return False
        try:
            return bool(await nodes[0].is_enabled())
        except RuntimeError:
            return False

    async def is_checked(self) -> bool:
        """Return `True` if the matched checkbox/radio is checked."""
        nodes = await self._resolve_all_once()
        if not nodes:
            return False
        try:
            return bool(await nodes[0].is_checked())
        except RuntimeError:
            return False

    async def wait_for(
        self,
        timeout_ms: int = 10000,
        state: LocatorState = "visible",
    ) -> None:
        """Wait until the locator reaches `state` (`'visible'`, `'attached'`, `'hidden'`, or `'detached'`)."""
        deadline = time.monotonic() + timeout_ms / 1000.0
        while True:
            nodes = await self._resolve_all_once()
            if state == "attached":
                if nodes:
                    return
            elif state == "detached":
                if not nodes:
                    return
            elif state == "visible":
                if nodes:
                    try:
                        if await nodes[0].is_visible():
                            return
                    except RuntimeError:
                        pass
            elif state == "hidden":
                if not nodes:
                    return
                try:
                    if not await nodes[0].is_visible():
                        return
                except RuntimeError:
                    return
            else:
                raise ValueError(f"Unsupported locator state: {state!r}")

            if time.monotonic() >= deadline:
                raise TimeoutError(
                    f"Locator wait_for(state={state!r}) timed out after {timeout_ms}ms: {self!r}"
                )
            await asyncio.sleep(0.08)

    async def wait_for_hidden(self, timeout_ms: int = 10000) -> None:
        """Wait until the element is detached or hidden."""
        await self.wait_for(timeout_ms=timeout_ms, state="hidden")

    async def click(
        self,
        button: str = "left",
        click_count: int = 1,
        timeout_ms: int = 10000,
    ) -> None:
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

    async def check(self, timeout_ms: int = 10000) -> None:
        await self._run_with_retry(timeout_ms, lambda el: el.check())

    async def uncheck(self, timeout_ms: int = 10000) -> None:
        await self._run_with_retry(timeout_ms, lambda el: el.uncheck())

    async def select_option(self, value: str, timeout_ms: int = 10000) -> None:
        await self._run_with_retry(timeout_ms, lambda el: el.select_option(value))

    async def press_key(self, key: str, timeout_ms: int = 10000) -> None:
        async def _act(el: Any) -> None:
            await el.click()
            await self._tab.press_key(key)

        await self._run_with_retry(timeout_ms, _act)

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


__all__ = ["Locator", "LocatorState"]
