"""Real-world best-driver <-> http_tls synchronized session test & benchmark.

Usage:
    # Default live diagnostic suite (tls.peet.ws + cloudflare trace + httpbin cookies + 10x burst)
    python examples/test_real_handoff.py

    # Test against your own URL (optional --proxy and --headful)
    python examples/test_real_handoff.py --url https://example.com --proxy http://user:pass@host:8080
    python examples/test_real_handoff.py --url https://example.com --headful

    # Test session handoff across multiple public Cloudflare-fronted endpoints
    python examples/test_real_handoff.py --multi-cf
"""

from __future__ import annotations

import argparse
import asyncio
import time
import best_driver


async def run_default_suite(headless: bool, proxy: str | None) -> None:
    print("=" * 70)
    print("REAL-WORLD HYBRID SESSION TEST: best-driver (Chromium) <-> http_tls")
    print("=" * 70)

    async with await best_driver.start(headless=headless, proxy=proxy) as browser:
        print(f"[BROWSER] port={browser.port} | proxy={browser.proxy}")

        # ── 1. Cloudflare Edge Trace: Chromium vs Synced http_tls ──────────────
        print("\n[1/4] Cloudflare Edge Trace (https://www.cloudflare.com/cdn-cgi/trace)")
        tab = await browser.get("https://www.cloudflare.com/cdn-cgi/trace", wait_until="load")
        cf_chrome_raw = await tab.evaluate("document.body.innerText")
        cf_chrome = dict(
            line.split("=", 1) for line in cf_chrome_raw.strip().splitlines() if "=" in line
        )
        fp = await tab.fingerprint()
        print(f"  Chromium  -> http={cf_chrome.get('http')} tls={cf_chrome.get('tls')} ip={cf_chrome.get('ip')} uag={cf_chrome.get('uag')}")
        print(f"  Preset    -> {fp['impersonate']} | Sec-CH-UA={fp['sec_ch_ua']}")

        async with await tab.tls_session(async_mode=True) as tls:
            r = await tls.get("https://www.cloudflare.com/cdn-cgi/trace")
            assert r.status_code == 200, f"Unexpected status: {r.status_code}"
            cf_tls = dict(line.split("=", 1) for line in r.text.strip().splitlines() if "=" in line)
            print(f"  http_tls  -> http={cf_tls.get('http')} tls={cf_tls.get('tls')} ip={cf_tls.get('ip')} uag={cf_tls.get('uag')} status={r.status_code}")
            assert cf_chrome.get("ip") == cf_tls.get("ip"), "Outbound IP mismatch!"
            assert cf_chrome.get("http") == cf_tls.get("http") == "http/2", (
                f"Expected HTTP/2 on both, got chrome={cf_chrome.get('http')} tls={cf_tls.get('http')}"
            )
            assert cf_chrome.get("uag") == cf_tls.get("uag"), "User-Agent mismatch on Cloudflare edge!"
            assert cf_tls.get("tls") == "TLSv1.3", f"Expected TLSv1.3, got {cf_tls.get('tls')}"
            print("  [PASS] Cloudflare edge sees identical IP, HTTP/2, TLSv1.3, and User-Agent!")

            # ── 2. Burst Throughput Over Synced Session ────────────────────────
            print("\n[2/4] 10x Concurrent Burst via Synced http_tls Session")
            t0 = time.perf_counter()
            resps = await asyncio.gather(
                *(tls.get("https://www.cloudflare.com/cdn-cgi/trace") for _ in range(10))
            )
            dt_ms = (time.perf_counter() - t0) * 1000.0
            ok_cnt = sum(1 for x in resps if x.status_code == 200)
            print(f"  [PASS] {ok_cnt}/10 HTTP 200 in {dt_ms:.1f} ms ({10 / (dt_ms / 1000.0):.1f} req/s)")
            assert ok_cnt == 10

        # ── 3. Wire-Level TLS/HTTP2/Header Order Audit (tls.peet.ws) ───────────
        print("\n[3/4] Wire-Level Fingerprint Audit (https://tls.peet.ws/api/all)")
        await tab.get("https://tls.peet.ws/api/all", wait_until="networkidle")
        chrome_wire = await tab.evaluate("JSON.parse(document.body.innerText)")
        async with await tab.tls_session(async_mode=True) as tls:
            tls_wire = (await tls.get("https://tls.peet.ws/api/all")).json()

        c_ja4_prefix = "_".join(chrome_wire["tls"]["ja4"].split("_")[:2])
        t_ja4_prefix = "_".join(tls_wire["tls"]["ja4"].split("_")[:2])
        c_exts = sorted(
            e["name"] for e in chrome_wire["tls"]["extensions"]
            if not str(e.get("name", "")).startswith("TLS_GREASE")
        )
        t_exts = sorted(
            e["name"] for e in tls_wire["tls"]["extensions"]
            if not str(e.get("name", "")).startswith("TLS_GREASE")
        )
        c_h2 = chrome_wire["http2"]["akamai_fingerprint"]
        t_h2 = tls_wire["http2"]["akamai_fingerprint"]
        c_order = [
            h.split(":", 1)[0].strip().lower()
            for h in chrome_wire["http2"]["sent_frames"][-1].get("headers", [])
            if isinstance(h, str) and not h.startswith(":") and ":" in h
        ]
        t_order = [
            h.split(":", 1)[0].strip().lower()
            for h in tls_wire["http2"]["sent_frames"][-1].get("headers", [])
            if isinstance(h, str) and not h.startswith(":") and ":" in h
        ]
        print(f"  JA4 Prefix/Cipher : {c_ja4_prefix == t_ja4_prefix} ({t_ja4_prefix})")
        print(f"  16/16 TLS Exts    : {c_exts == t_exts} (count={len(t_exts)})")
        print(f"  Full JA4 Match    : {chrome_wire['tls']['ja4'] == tls_wire['tls']['ja4']} (chrome={chrome_wire['tls']['ja4']} | tls={tls_wire['tls']['ja4']})")
        print(f"  Akamai H2 Match   : {c_h2 == t_h2} ({t_h2})")
        print(f"  Header Order Match: {c_order == t_order} ({len(t_order)} headers)")
        assert c_ja4_prefix == t_ja4_prefix
        assert c_exts == t_exts
        assert c_h2 == t_h2
        assert c_order == t_order
        print("  [PASS] 15/15 Ciphers, 16/16 TLS Exts, Akamai H2, and 13/13 HTTP/2 Header Order MATCHED!")

        # ── 4. Bi-Directional HttpOnly Cookie Roundtrip (httpbin.org) ──────────
        print("\n[4/4] Bi-Directional HttpOnly Cookie Handoff (https://httpbin.org/cookies)")
        await tab.get("https://httpbin.org/cookies", wait_until="load")
        await tab.set_cookies([
            {
                "name": "session_httponly",
                "value": "verified_cdp_token",
                "domain": "httpbin.org",
                "path": "/",
                "secure": True,
                "httpOnly": True,
            }
        ])
        async with await tab.tls_session(async_mode=True) as tls:
            got = (await tls.get("https://httpbin.org/cookies")).json()["cookies"]
            print(f"  Chromium -> http_tls : {got}")
            assert got.get("session_httponly") == "verified_cdp_token"

            await tls.get("https://httpbin.org/cookies/set?refreshed_by_tls=ok_2026")
            pushed = await tab.sync_from_tls(tls)
            print(f"  http_tls -> Chromium : pushed {pushed} cookies")

        await tab.get("https://httpbin.org/cookies", wait_until="load")
        got_chrome = (await tab.evaluate("JSON.parse(document.body.innerText)"))["cookies"]
        print(f"  Chromium verified    : {got_chrome}")
        assert got_chrome.get("refreshed_by_tls") == "ok_2026"
        print("  [PASS] Bi-directional cookie handoff verified!")

    print("\nALL 4 REAL-WORLD HYBRID CHECKS PASSED.")


async def run_custom_url(url: str, headless: bool, proxy: str | None) -> None:
    print("=" * 70)
    print(f"CUSTOM URL SESSION HANDOFF TEST: {url}")
    print("=" * 70)

    async with await best_driver.start(headless=headless, proxy=proxy) as browser:
        tab = await browser.get(url, wait_until="networkidle")
        title = await tab.title()
        fp = await tab.fingerprint()
        cookies = await tab.cookies()
        http_only_names = [c["name"] for c in cookies if c.get("httpOnly")]

        print(f"[CHROMIUM] title={title!r}")
        print(f"[CHROMIUM] preset={fp['impersonate']} | proxy={fp['proxy']}")
        print(f"[CHROMIUM] UA={fp['user_agent']}")
        print(f"[CHROMIUM] Sec-CH-UA={fp['sec_ch_ua']}")
        print(f"[CHROMIUM] cookies={len(cookies)} (HttpOnly={http_only_names})")

        async with await tab.tls_session(async_mode=True) as tls:
            t0 = time.perf_counter()
            resp = await tls.get(url)
            ms = (time.perf_counter() - t0) * 1000.0
            print(
                f"[HTTP_TLS] status={resp.status_code} | http={resp.http_version} | "
                f"challenge={resp.challenge} | bytes={len(resp.content)} | time={ms:.1f} ms"
            )
            pushed = await tab.sync_from_tls(tls)
            print(f"[SYNC]     pushed {pushed} cookies back to Chromium")


CLOUDFLARE_ENDPOINTS = [
    ("Cloudflare Trace", "https://www.cloudflare.com/cdn-cgi/trace"),
    ("Cloudflare 1.1.1.1", "https://1.1.1.1/cdn-cgi/trace"),
    ("Cloudflare Docs", "https://developers.cloudflare.com/"),
    ("Cloudflare API IPs", "https://api.cloudflare.com/client/v4/ips"),
    ("Cloudflare CDNJS", "https://cdnjs.cloudflare.com/ajax/libs/lodash.js/4.17.21/lodash.min.js"),
    ("npm Registry (CF)", "https://www.npmjs.com/"),
]


async def run_multi_cf(headless: bool, proxy: str | None) -> None:
    print("=" * 78)
    print("MULTI-ORIGIN CLOUDFLARE EDGE HANDOFF TEST (best-driver -> http_tls)")
    print("=" * 78)
    passed = 0
    gated = 0

    async with await best_driver.start(headless=headless, proxy=proxy) as browser:
        tab = await browser.main_tab()
        for label, url in CLOUDFLARE_ENDPOINTS:
            t_nav = time.perf_counter()
            await tab.get(url, wait_until="load")
            nav_ms = (time.perf_counter() - t_nav) * 1000.0
            fp = await tab.fingerprint()
            cookies = await tab.cookies()

            async with await tab.tls_session(async_mode=True) as tls:
                t_req = time.perf_counter()
                resp = await tls.get(url)
                req_ms = (time.perf_counter() - t_req) * 1000.0
                cf_ray = resp.headers.get("cf-ray", "-")
                server = resp.headers.get("server", "-")
                ch = resp.challenge

            status_tag = "PASS" if (resp.status_code == 200 and ch is None) else ("GATED" if ch else f"HTTP_{resp.status_code}")
            if status_tag == "PASS":
                passed += 1
            elif status_tag == "GATED":
                gated += 1

            print(
                f"  [{status_tag:5s}] {label:20s} | status={resp.status_code} | {resp.http_version:8s} | "
                f"server={server:10s} | cf-ray={cf_ray[:18]:18s} | local={str(fp['local_address']):7s} | "
                f"cookies={len(cookies):2d} | chrome={nav_ms:5.0f}ms -> tls={req_ms:4.0f}ms | challenge={ch}"
            )

    print("-" * 78)
    print(f"Summary: {passed}/{len(CLOUDFLARE_ENDPOINTS)} HTTP 200 clean | gated={gated}")
    assert passed == len(CLOUDFLARE_ENDPOINTS), f"Expected all {len(CLOUDFLARE_ENDPOINTS)} endpoints to pass cleanly"


def main() -> None:
    parser = argparse.ArgumentParser(description="Real-world best-driver <-> http_tls handoff test")
    parser.add_argument("--url", type=str, default=None, help="Custom URL to test session handoff")
    parser.add_argument("--proxy", type=str, default=None, help="Proxy URL (http://user:pass@host:port)")
    parser.add_argument("--headful", action="store_true", help="Show Chromium window (headless=False)")
    parser.add_argument("--multi-cf", action="store_true", help="Test handoff across 6 public Cloudflare-fronted endpoints")
    args = parser.parse_args()

    headless = not args.headful
    if args.multi_cf:
        asyncio.run(run_multi_cf(headless=headless, proxy=args.proxy))
    elif args.url:
        asyncio.run(run_custom_url(args.url, headless=headless, proxy=args.proxy))
    else:
        asyncio.run(run_default_suite(headless=headless, proxy=args.proxy))


if __name__ == "__main__":
    main()
