"""Synchronized `best-driver` <-> `http_tls` fingerprint and session bridge.

Transfers all 5 identity layers from a live Chromium `Tab` into an `http_tls`
BoringSSL session:
1. Outbound proxy URL (`browser.proxy`)
2. Matching BoringSSL TLS/HTTP2 preset (`chrome_<ver>[_<os>]` / `edge_<ver>`)
3. Exact `User-Agent`, `Sec-CH-UA*`, and `Accept-Language` headers
4. Exact Chrome HTTP/2 wire `header_order` (`accept-encoding` before `accept-language`)
5. Full CDP cookie jar including `HttpOnly` and `Secure` cookies
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Any, Literal, Union, overload

if TYPE_CHECKING:
    from http_tls import AsyncSession as HttpTlsAsyncSession  # type: ignore[import-not-found]
    from http_tls import Session as HttpTlsSession  # type: ignore[import-not-found]
else:
    HttpTlsAsyncSession = Any
    HttpTlsSession = Any


def _import_http_tls() -> Any:
    try:
        import http_tls  # type: ignore[import-not-found]
    except ImportError as exc:
        raise ImportError(
            "http_tls is required for tab.tls_session() / tab.sync_from_tls(). "
            "Install it with: pip install 'best-driver[tls]' (or pip install http-tls)"
        ) from exc
    return http_tls


@overload
async def from_tab(tab: Any, async_mode: Literal[True] = True, **kwargs: Any) -> HttpTlsAsyncSession: ...


@overload
async def from_tab(tab: Any, async_mode: Literal[False], **kwargs: Any) -> HttpTlsSession: ...


@overload
async def from_tab(tab: Any, async_mode: bool = True, **kwargs: Any) -> Union[HttpTlsAsyncSession, HttpTlsSession]: ...


async def from_tab(tab: Any, async_mode: bool = True, **kwargs: Any) -> Any:
    """Create an `http_tls.AsyncSession` (or `Session`) synchronized with `tab`."""
    http_tls = _import_http_tls()
    state = await tab.storage_state()

    session_kwargs: dict[str, Any] = {
        "impersonate": state.get("impersonate") or "chrome_149",
    }
    if state.get("proxy"):
        session_kwargs["proxy"] = state["proxy"]
    elif state.get("local_address"):
        # Align outbound IP family (IPv6 `::` vs IPv4 `0.0.0.0`) with the live Chromium connection
        # so dual-stack hosts (e.g. Cloudflare) see the exact same client IP across browser -> http_tls.
        session_kwargs["local_address"] = state["local_address"]
    if state.get("header_order"):
        session_kwargs["header_order"] = list(state["header_order"])

    # Merge caller headers on top of exact live browser headers
    base_headers = dict(state.get("headers") or {})
    caller_headers = kwargs.pop("headers", None)
    if caller_headers:
        base_headers.update(caller_headers)

    session_kwargs.update(kwargs)

    session_cls = http_tls.AsyncSession if async_mode else http_tls.Session
    session = session_cls(**session_kwargs)

    # Inject domain-scoped cookies directly into http_tls's Rust cookie jar when available
    # so cookies from one origin never leak onto requests for other domains.
    sync_inner = getattr(session, "_sync_session", session)
    rust_core = getattr(sync_inner, "_session", None)
    if getattr(sync_inner, "_mode", None) == "rust" and rust_core is not None and hasattr(rust_core, "add_raw_cookie"):
        for c in state.get("cookies") or []:
            if not isinstance(c, dict) or "name" not in c or "value" not in c:
                continue
            name, val = str(c["name"]), str(c["value"])
            dom = str(c.get("domain") or "").lstrip(".")
            path = str(c.get("path") or "/")
            if not dom:
                continue
            parts = [f"{name}={val}", f"Domain={dom}", f"Path={path}"]
            if c.get("secure"):
                parts.append("Secure")
            if c.get("httpOnly"):
                parts.append("HttpOnly")
            scheme = "https" if c.get("secure", True) else "http"
            rust_core.add_raw_cookie("; ".join(parts), f"{scheme}://{dom}{path}")
    else:
        session.load_storage_state(state)

    if base_headers:
        session.headers.update(base_headers)
    return session


async def sync_to_tab(tab: Any, session: Any, default_url: str | None = None) -> int:
    """Push cookies from an `http_tls` `Session` / `AsyncSession` back into `tab`.

    Cookies that carry an explicit `domain` are applied to that domain; host-only
    cookies with an empty `domain` fall back to `default_url` or `await tab.url()`.
    """
    state = session.save_storage_state()
    raw_cookies = state.get("cookies") or []
    fallback_url: str | None = default_url
    cdp_cookies: list[dict[str, Any]] = []
    for c in raw_cookies:
        if not isinstance(c, dict):
            continue
        name = c.get("name")
        value = c.get("value")
        if not name or value is None:
            continue
        domain = str(c.get("domain") or "").strip()
        item: dict[str, Any] = {
            "name": str(name),
            "value": str(value),
            "path": str(c.get("path") or "/"),
            "secure": bool(c.get("secure", False)),
            "httpOnly": bool(c.get("httpOnly", False)),
        }
        if domain:
            item["domain"] = domain
        else:
            if fallback_url is None:
                cur = await tab.url()
                fallback_url = cur if isinstance(cur, str) and cur.startswith(("http://", "https://")) else ""
            if not fallback_url:
                continue
            item["url"] = fallback_url

        same_site = c.get("sameSite")
        if same_site in ("Strict", "Lax", "None"):
            item["sameSite"] = same_site
        expires = c.get("expires")
        if isinstance(expires, (int, float)) and expires > 0:
            item["expires"] = expires
        cdp_cookies.append(item)

    if cdp_cookies:
        await tab.set_cookies(cdp_cookies)
    return len(cdp_cookies)


tls_session = from_tab
sync_from_tls = sync_to_tab

__all__ = ["from_tab", "sync_to_tab", "tls_session", "sync_from_tls"]
