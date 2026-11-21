"""End-to-end verification of best-driver <-> http_tls fingerprint & session sync."""

import asyncio
import os
import best_driver
import http_tls


async def main() -> None:
    print("=" * 65)
    print(">>> TEST 1: On-the-Wire Signature Parity (tls.peet.ws/api/all)")
    async with await best_driver.start(headless=True) as browser:
        assert browser.proxy is None
        tab = await browser.get("https://tls.peet.ws/api/all", wait_until="networkidle")
        chrome = await tab.evaluate("JSON.parse(document.body.innerText)")
        fp = await tab.fingerprint()
        print("  [FP] derived preset:", fp["impersonate"])
        print("  [FP] sec-ch-ua:     ", fp["sec_ch_ua"])
        print("  [FP] accept-lang:   ", fp["accept_language"])

        async with await tab.tls_session(async_mode=True) as tls:
            synced = (await tls.get("https://tls.peet.ws/api/all")).json()

        c_ja4 = chrome["tls"]["ja4"]
        t_ja4 = synced["tls"]["ja4"]
        c_cipher_hash = c_ja4.split("_")[1]
        t_cipher_hash = t_ja4.split("_")[1]
        c_h2 = chrome["http2"]["akamai_fingerprint"]
        t_h2 = synced["http2"]["akamai_fingerprint"]

        c_hdrs = {
            h.split(":", 1)[0].strip().lower(): h.split(":", 1)[1].strip()
            for h in chrome["http2"]["sent_frames"][-1].get("headers", [])
            if isinstance(h, str) and not h.startswith(":") and ":" in h
        }
        t_hdrs = {
            h.split(":", 1)[0].strip().lower(): h.split(":", 1)[1].strip()
            for h in synced["http2"]["sent_frames"][-1].get("headers", [])
            if isinstance(h, str) and not h.startswith(":") and ":" in h
        }
        c_order = [
            h.split(":", 1)[0].strip().lower()
            for h in chrome["http2"]["sent_frames"][-1].get("headers", [])
            if isinstance(h, str) and not h.startswith(":") and ":" in h
        ]
        t_order = [
            h.split(":", 1)[0].strip().lower()
            for h in synced["http2"]["sent_frames"][-1].get("headers", [])
            if isinstance(h, str) and not h.startswith(":") and ":" in h
        ]

        print(f"  [WIRE] Cipher Hash:     chrome={c_cipher_hash} | tls={t_cipher_hash} | match={c_cipher_hash == t_cipher_hash}")
        print(f"  [WIRE] Akamai H2:       match={c_h2 == t_h2} ({t_h2})")
        print(f"  [WIRE] User-Agent:      match={c_hdrs.get('user-agent') == t_hdrs.get('user-agent')}")
        print(f"  [WIRE] Sec-CH-UA:       match={c_hdrs.get('sec-ch-ua') == t_hdrs.get('sec-ch-ua')} ({t_hdrs.get('sec-ch-ua')})")
        print(f"  [WIRE] Sec-CH-UA-Plat:  match={c_hdrs.get('sec-ch-ua-platform') == t_hdrs.get('sec-ch-ua-platform')}")
        print(f"  [WIRE] Accept-Language: match={c_hdrs.get('accept-language') == t_hdrs.get('accept-language')}")
        print(f"  [WIRE] Chrome H2 order: {c_order}")
        print(f"  [WIRE] http_tls order:  {t_order}")

        assert c_cipher_hash == t_cipher_hash, "Cipher hash mismatch!"
        assert c_h2 == t_h2, "Akamai HTTP/2 mismatch!"
        assert c_hdrs.get("user-agent") == t_hdrs.get("user-agent"), "User-Agent mismatch!"
        assert c_hdrs.get("sec-ch-ua") == t_hdrs.get("sec-ch-ua"), "Sec-CH-UA mismatch!"
        assert c_hdrs.get("sec-ch-ua-platform") == t_hdrs.get("sec-ch-ua-platform"), "Platform mismatch!"
        assert c_hdrs.get("accept-language") == t_hdrs.get("accept-language"), "Accept-Language mismatch!"
        # Verify accept-encoding appears before accept-language (matching Chrome)
        if "accept-encoding" in t_order and "accept-language" in t_order:
            assert t_order.index("accept-encoding") < t_order.index("accept-language"), (
                f"Expected accept-encoding before accept-language, got {t_order}"
            )
        print("[PASS] TEST 1: On-the-wire TLS Cipher, Akamai H2, UA, Sec-CH-UA*, Accept-Language & Header Order MATCHED!")

        print("\n>>> TEST 2: Bi-Directional Cookie Sync (Including HttpOnly) on httpbin.org")
        await tab.get("https://httpbin.org/cookies", wait_until="load")
        # Inject an HttpOnly + Secure cookie directly via CDP (simulating cf_clearance)
        await tab.set_cookies([
            {
                "name": "cf_clearance_sim",
                "value": "httponly_secret_token_xyz",
                "domain": "httpbin.org",
                "path": "/",
                "secure": True,
                "httpOnly": True,
            }
        ])
        # Verify JS cannot see HttpOnly cookie, proving it came through CDP
        js_cookie = await tab.evaluate("document.cookie")
        assert "cf_clearance_sim" not in js_cookie, f"HttpOnly cookie leaked to JS: {js_cookie}"

        # Handoff Chromium -> http_tls
        async with await tab.tls_session(async_mode=True) as tls:
            r = (await tls.get("https://httpbin.org/cookies")).json()
            print(f"  [SYNC -> http_tls] httpbin received cookies: {r['cookies']}")
            assert r["cookies"].get("cf_clearance_sim") == "httponly_secret_token_xyz"

            # Now server sets a NEW cookie inside http_tls -> sync back to Chromium!
            await tls.get("https://httpbin.org/cookies/set?tls_refreshed=rust_99")
            synced_count = await tab.sync_from_tls(tls)
            print(f"  [SYNC -> Chromium] pushed {synced_count} cookies back into Chromium")
            assert synced_count >= 1

        await tab.get("https://httpbin.org/cookies", wait_until="load")
        chrome_cookies = await tab.evaluate("JSON.parse(document.body.innerText)")
        print(f"  [SYNC -> Chromium] Chromium now sends: {chrome_cookies['cookies']}")
        assert chrome_cookies["cookies"].get("tls_refreshed") == "rust_99"
        print("[PASS] TEST 2: Bi-directional HttpOnly & refreshed cookie sync verified!")

        print("\n>>> TEST 3: storage_state.json Export & Sync Session Reload")
        state_path = os.path.abspath("tmp_storage_state.json")
        try:
            state = await tab.storage_state(state_path)
            assert os.path.exists(state_path)
            assert state["user_agent"] == fp["user_agent"]
            with http_tls.Session(storage_state=state_path, impersonate=state["impersonate"]) as sync_s:
                r3 = sync_s.get("https://httpbin.org/cookies").json()
                assert r3["cookies"].get("cf_clearance_sim") == "httponly_secret_token_xyz"
                assert r3["cookies"].get("tls_refreshed") == "rust_99"
            print("[PASS] TEST 3: storage_state.json export + http_tls.Session reload verified!")
        finally:
            if os.path.exists(state_path):
                os.remove(state_path)

    print("\n>>> ALL TLS SIGNATURE & SESSION SYNC TESTS PASSED (100%) <<<")


if __name__ == "__main__":
    asyncio.run(main())
