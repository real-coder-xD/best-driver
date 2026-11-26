"""Lazy, auto-retrying chained Locator, Semantic ARIA Locators, and FrameLocator."""

from __future__ import annotations

import asyncio
import os
import time
from typing import Any, List, Literal, Optional, Tuple, Union

LocatorState = Literal["attached", "visible", "hidden", "detached"]


def _escape_css_val(value: str) -> str:
    return value.replace("\\", "\\\\").replace('"', '\\"')


def _build_role_query(role: str, name: Optional[str] = None, exact: bool = False) -> str:
    r = role.strip().lower()
    if name is None:
        return f"role:{r}"
    esc = _escape_css_val(name)
    suffix = " exact" if exact else ""
    return f'role:{r}[name="{esc}"{suffix}]'


def _build_text_query(text: str, exact: bool = False) -> str:
    return f"text=:{text}" if exact else f"text:{text}"


def _build_label_query(label: str, exact: bool = False) -> str:
    return f"label=:{label}" if exact else f"label:{label}"


def _build_placeholder_query(placeholder: str, exact: bool = False) -> str:
    esc = _escape_css_val(placeholder)
    op = "=" if exact else "*="
    return f'[placeholder{op}"{esc}"]'


def _build_test_id_query(test_id: str, attribute: str = "data-testid") -> str:
    esc = _escape_css_val(test_id)
    return f'[{attribute}="{esc}"]'


class _Step:
    __slots__ = ("query", "index", "has_text", "has_not_text")

    def __init__(
        self,
        query: str,
        index: Optional[int] = None,
        has_text: Tuple[str, ...] = (),
        has_not_text: Tuple[str, ...] = (),
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
    both open and closed `#shadow-root` trees, ARIA roles, labels, and reloading iframes
    when created from a `FrameLocator`) and retries transient `StaleElement` or
    actionability errors when React, Vue, or Next.js re-renders mid-interaction.
    """

    def __init__(
        self,
        tab: Any,
        steps: List[Any],
        *,
        frame_chain: Tuple[str, ...] = (),
        strict: bool = False,
    ) -> None:
        self._tab = tab
        self._frame_chain: Tuple[str, ...] = tuple(frame_chain)
        self._strict: bool = strict
        self._steps: List[_Step] = []
        for s in steps:
            if isinstance(s, _Step):
                self._steps.append(s)
            elif isinstance(s, tuple):
                self._steps.append(_Step(s[0], s[1]))
            else:
                self._steps.append(_Step(str(s)))

    def __repr__(self) -> str:
        prefix = "".join(f"frame({f!r}) >> " for f in self._frame_chain)
        parts = []
        for st in self._steps:
            s = st.query
            for ht in st.has_text:
                s += f":has-text({ht!r})"
            for hnt in st.has_not_text:
                s += f":not-text({hnt!r})"
            if st.index is not None:
                s += f"[{st.index}]"
            parts.append(s)
        return f"<Locator {prefix}{' >> '.join(parts)!r}>"

    def _spawn(self, steps: List[_Step], *, strict: Optional[bool] = None) -> "Locator":
        return Locator(
            self._tab,
            steps,
            frame_chain=self._frame_chain,
            strict=self._strict if strict is None else strict,
        )

    def locator(self, query: str) -> "Locator":
        """Chain a descendant Smart Locator query (CSS, XPath, `text:`, `@attr`, `role:`, `label:`, Shadow DOM)."""
        return self._spawn([*self._steps, _Step(query)])

    def get_by_role(
        self,
        role: str,
        *,
        name: Optional[str] = None,
        exact: bool = False,
    ) -> "Locator":
        """Chain a Playwright-style ARIA role locator (e.g. `get_by_role('button', name='Submit')`)."""
        return self.locator(_build_role_query(role, name=name, exact=exact))

    def get_by_text(self, text: str, *, exact: bool = False) -> "Locator":
        """Chain a visible-text locator (substring by default, or exact match with `exact=True`)."""
        return self.locator(_build_text_query(text, exact=exact))

    def get_by_label(self, label: str, *, exact: bool = False) -> "Locator":
        """Chain a form control locator matched by its associated `<label>` or `aria-label`."""
        return self.locator(_build_label_query(label, exact=exact))

    def get_by_placeholder(self, placeholder: str, *, exact: bool = False) -> "Locator":
        """Chain an input/textarea locator matched by its `placeholder` attribute."""
        return self.locator(_build_placeholder_query(placeholder, exact=exact))

    def get_by_test_id(self, test_id: str, *, attribute: str = "data-testid") -> "Locator":
        """Chain a locator matched by `data-testid` (or a custom test-id attribute)."""
        return self.locator(_build_test_id_query(test_id, attribute=attribute))

    def filter(
        self,
        *,
        has_text: Optional[str] = None,
        has_not_text: Optional[str] = None,
    ) -> "Locator":
        """Filter the current step's matched elements by substring inclusion/exclusion (case-insensitive)."""
        steps = list(self._steps)
        cur = steps[-1]
        next_has = (*cur.has_text, has_text) if has_text is not None else cur.has_text
        next_not = (*cur.has_not_text, has_not_text) if has_not_text is not None else cur.has_not_text
        steps[-1] = cur.clone(has_text=next_has, has_not_text=next_not)
        return self._spawn(steps)

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
        return self._spawn(steps)

    def with_strict(self, strict: bool = True) -> "Locator":
        """Return a copy of this locator with strict single-match enforcement toggled."""
        return self._spawn(list(self._steps), strict=strict)

    @staticmethod
    def _apply_step_filters(nodes: List[Any], step: _Step) -> List[Any]:
        filtered = nodes
        for ht in step.has_text:
            needle = ht.lower()
            filtered = [n for n in filtered if needle in (n.text or "").lower()]
        for hnt in step.has_not_text:
            needle = hnt.lower()
            filtered = [n for n in filtered if needle not in (n.text or "").lower()]
        if step.index is not None:
            try:
                return [filtered[step.index]]
            except IndexError:
                return []
        return filtered

    async def _resolve_target_tab(self, timeout_ms: int = 1000) -> Optional[Any]:
        target = self._tab
        for frame_sel in self._frame_chain:
            try:
                target = await target.iframe(frame_sel, timeout_ms=timeout_ms)
            except (TimeoutError, RuntimeError, ValueError):
                return None
        return target

    async def _resolve_all_once(self) -> List[Any]:
        if not self._steps:
            return []
        target_tab = await self._resolve_target_tab(timeout_ms=250)
        if target_tab is None:
            return []
        s0 = self._steps[0]
        try:
            nodes: List[Any] = await target_tab.eles(s0.query, timeout_ms=0)
        except RuntimeError:
            return []
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
                if self._strict and len(nodes) > 1:
                    raise ValueError(
                        f"Strict mode violation: {self!r} matched {len(nodes)} elements; "
                        "narrow with .first, .last, .nth(i), or .filter(...)"
                    )
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
        force: bool = False,
    ) -> None:
        await self._run_with_retry(
            timeout_ms,
            lambda el: el.click(button=button, click_count=click_count, force=force),
        )

    async def double_click(self, timeout_ms: int = 10000, force: bool = False) -> None:
        await self.click(button="left", click_count=2, timeout_ms=timeout_ms, force=force)

    async def right_click(self, timeout_ms: int = 10000, force: bool = False) -> None:
        await self.click(button="right", click_count=1, timeout_ms=timeout_ms, force=force)

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
        file_path: Optional[Union[str, os.PathLike[str]]] = None,
        timeout_ms: int = 10000,
    ) -> bytes:
        return await self._run_with_retry(timeout_ms, lambda el: el.screenshot(file_path))


class FrameLocator:
    """Lazy, auto-retrying iframe locator chain resilient to iframe reloads and SPA re-mounts."""

    def __init__(self, tab: Any, frame_chain: Tuple[str, ...]) -> None:
        self._tab = tab
        self._frame_chain: Tuple[str, ...] = tuple(frame_chain)

    def __repr__(self) -> str:
        return f"<FrameLocator {' >> '.join(self._frame_chain)!r}>"

    def frame_locator(self, selector: str) -> "FrameLocator":
        """Chain a nested `<iframe>` selector inside this frame."""
        return FrameLocator(self._tab, (*self._frame_chain, selector))

    def locator(self, query: str, *, strict: bool = False) -> Locator:
        """Create a lazy `Locator` scoped to this iframe chain."""
        return Locator(self._tab, [_Step(query)], frame_chain=self._frame_chain, strict=strict)

    def get_by_role(
        self,
        role: str,
        *,
        name: Optional[str] = None,
        exact: bool = False,
        strict: bool = False,
    ) -> Locator:
        """Locate an element by ARIA role and optional accessible name inside this iframe."""
        return self.locator(_build_role_query(role, name=name, exact=exact), strict=strict)

    def get_by_text(self, text: str, *, exact: bool = False, strict: bool = False) -> Locator:
        """Locate an element by visible text inside this iframe."""
        return self.locator(_build_text_query(text, exact=exact), strict=strict)

    def get_by_label(self, label: str, *, exact: bool = False, strict: bool = False) -> Locator:
        """Locate a form control by its `<label>` or `aria-label` inside this iframe."""
        return self.locator(_build_label_query(label, exact=exact), strict=strict)

    def get_by_placeholder(
        self,
        placeholder: str,
        *,
        exact: bool = False,
        strict: bool = False,
    ) -> Locator:
        """Locate an input/textarea by its `placeholder` inside this iframe."""
        return self.locator(_build_placeholder_query(placeholder, exact=exact), strict=strict)

    def get_by_test_id(
        self,
        test_id: str,
        *,
        attribute: str = "data-testid",
        strict: bool = False,
    ) -> Locator:
        """Locate an element by `data-testid` (or custom attribute) inside this iframe."""
        return self.locator(_build_test_id_query(test_id, attribute=attribute), strict=strict)


def make_locator(tab: Any, query: str, strict: bool = False) -> Locator:
    return Locator(tab, [_Step(query)], strict=strict)


def make_frame_locator(tab: Any, selector: str) -> FrameLocator:
    return FrameLocator(tab, (selector,))


def make_role_locator(
    tab: Any,
    role: str,
    name: Optional[str] = None,
    exact: bool = False,
    strict: bool = False,
) -> Locator:
    return Locator(tab, [_Step(_build_role_query(role, name=name, exact=exact))], strict=strict)


def make_text_locator(tab: Any, text: str, exact: bool = False, strict: bool = False) -> Locator:
    return Locator(tab, [_Step(_build_text_query(text, exact=exact))], strict=strict)


def make_label_locator(tab: Any, label: str, exact: bool = False, strict: bool = False) -> Locator:
    return Locator(tab, [_Step(_build_label_query(label, exact=exact))], strict=strict)


def make_placeholder_locator(
    tab: Any,
    placeholder: str,
    exact: bool = False,
    strict: bool = False,
) -> Locator:
    return Locator(tab, [_Step(_build_placeholder_query(placeholder, exact=exact))], strict=strict)


def make_test_id_locator(
    tab: Any,
    test_id: str,
    attribute: str = "data-testid",
    strict: bool = False,
) -> Locator:
    return Locator(tab, [_Step(_build_test_id_query(test_id, attribute=attribute))], strict=strict)


__all__ = [
    "FrameLocator",
    "Locator",
    "LocatorState",
    "make_frame_locator",
    "make_label_locator",
    "make_locator",
    "make_placeholder_locator",
    "make_role_locator",
    "make_test_id_locator",
    "make_text_locator",
]
