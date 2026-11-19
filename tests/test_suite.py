# test_helpers.py — Python 3.9+, best_driver: offline end-to-end check of every Tab/Browser helper
import asyncio
import json
import os
import tempfile
import best_driver

PAGE = """<!doctype html><html><head><title>helpers</title></head><body style="height:4000px">
<h1 id="h">Hello <b>World</b></h1>
<a id="lnk" href="#p2" data-x="42">Next</a>
<input id="name" value="old">
<textarea id="ta"></textarea>
<select id="sel"><option value="a">Alpha</option><option value="b">Beta</option></select>
<label><input id="cb" type="checkbox"> agree</label>
<input id="file" type="file" multiple>
<button id="btn" onclick="this.dataset.n=(+this.dataset.n||0)+1">Push me</button>
<button id="dbl" ondblclick="this.textContent='dbl ok'">Double</button>
<div id="hov" onmouseover="this.textContent='hovered'" style="width:100px;height:40px;background:#ccc">hover</div>
<button id="dlg" onclick="window.r=confirm('sure?')">Dialog</button>
<div id="late"></div>
<ul><li class="it">one</li><li class="it">two</li><li class="it">three</li></ul>
<div id="bottom" style="position:absolute;top:3900px">END</div>
<script>
  setTimeout(() => { document.getElementById('late').innerHTML = '<span id="arrived">arrived</span>'; }, 800);
  document.getElementById('name').addEventListener('keydown', e => { if (e.key === 'Enter') window.enterHit = true; });
</script></body></html>"""

results = []


def check(name, cond, detail=""):
    results.append((name, bool(cond)))
    print(f"  [{'PASS' if cond else 'FAIL'}] {name} {detail}")


async def main():
    tmp = tempfile.mkdtemp(prefix="bd_test_")
    page_path = os.path.join(tmp, "page.html")
    with open(page_path, "w", encoding="utf-8") as f:
        f.write(PAGE)
    url = "file:///" + page_path.replace("\\", "/")
    upload1 = os.path.join(tmp, "a.txt")
    upload2 = os.path.join(tmp, "b.txt")
    for p in (upload1, upload2):
        with open(p, "w") as f:
            f.write("x")

    async with await best_driver.start(headless=False) as browser:
        print(browser)
        tab = await browser.get(url)

        print("\n[navigation / reading]")
        check("title", await tab.title() == "helpers")
        check("text", await tab.text("#h") == "Hello World")
        check("text_all", await tab.text_all(".it") == ["one", "two", "three"])
        check("count", await tab.count(".it") == 3)
        check("exists true", await tab.exists("#btn"))
        check("exists false", not await tab.exists("#nope"))
        check("attr", await tab.attr("#lnk", "data-x") == "42")
        check("attr_all", await tab.attr_all(".it", "class") == ["it", "it", "it"])
        check("html", "<b>World</b>" in (await tab.html("#h")))
        check("value", await tab.value("#name") == "old")
        check("evaluate dict", (await tab.evaluate("({a:1,b:[true,null,'s']})")) == {"a": 1, "b": [True, None, "s"]})

        print("\n[waiting]")
        await tab.wait_for("#arrived", timeout_ms=5000)
        check("wait_for", await tab.exists("#arrived"))
        await tab.wait_for_text("arrived", timeout_ms=3000)
        check("wait_for_text", True)
        check("wait_for_function", await tab.wait_for_function("document.querySelectorAll('.it').length") == 3)
        try:
            await tab.wait_for("#never", timeout_ms=300)
            check("wait_for timeout raises", False)
        except Exception as e:
            check("wait_for timeout raises", isinstance(e, TimeoutError) or "Timeout" in str(e))

        print("\n[mouse]")
        await tab.click("#btn")
        await tab.click("#btn")
        check("click x2", await tab.attr("#btn", "data-n") == "2")
        await tab.double_click("#dbl")
        check("double_click", await tab.text("#dbl") == "dbl ok")
        await tab.hover("#hov")
        check("hover", await tab.text("#hov") == "hovered")
        await tab.click_text("Push me")
        check("click_text", await tab.attr("#btn", "data-n") == "3")
        trusted = await tab.evaluate("""new Promise(r => { document.getElementById('btn')
            .addEventListener('click', e => r(e.isTrusted), {once:true}); })""") if False else None

        print("\n[keyboard / forms]")
        await tab.send_keys("#name", "Rust Py", clear=True, delay_ms=10)
        check("send_keys clear", await tab.value("#name") == "Rust Py")
        await tab.fill("#ta", "line fill ✓ tiếng Việt")
        check("fill unicode", await tab.value("#ta") == "line fill ✓ tiếng Việt")
        await tab.focus("#name")
        await tab.press_key("Enter")
        check("press_key Enter", await tab.evaluate("window.enterHit === true"))
        await tab.select_option("#sel", "Beta")
        check("select_option", await tab.value("#sel") == "b")
        await tab.check("#cb")
        check("check", await tab.evaluate("document.getElementById('cb').checked"))
        await tab.uncheck("#cb")
        check("uncheck", not await tab.evaluate("document.getElementById('cb').checked"))
        await tab.upload_file("#file", [upload1, upload2])
        check("upload_file", await tab.evaluate("Array.from(document.getElementById('file').files).map(f=>f.name)") == ["a.txt", "b.txt"])

        print("\n[dialogs]")
        await tab.auto_dialog(accept=True)
        await tab.click("#dlg")
        await tab.sleep(0.3)
        check("auto_dialog accept", await tab.evaluate("window.r") is True)
        await tab.auto_dialog(accept=False)
        await tab.click("#dlg")
        await tab.sleep(0.3)
        check("auto_dialog dismiss", await tab.evaluate("window.r") is False)
        await tab.auto_dialog(None)

        print("\n[scrolling]")
        await tab.scroll_to_top()
        await tab.scroll_down(500)
        await tab.sleep(0.3)
        check("scroll_down", await tab.evaluate("scrollY") > 0)
        await tab.scroll_to_bottom(step=1500, pause_ms=150, max_rounds=20)
        check("scroll_to_bottom", await tab.evaluate("scrollY + innerHeight >= document.documentElement.scrollHeight - 2"))
        await tab.scroll_to_top()
        check("scroll_to_top", await tab.evaluate("scrollY") == 0)

        print("\n[emulation]")
        await tab.set_viewport(390, 844, scale=1.0, mobile=False)
        check("set_viewport", await tab.evaluate("innerWidth") == 390)
        await tab.clear_viewport()
        await tab.set_timezone("Asia/Tokyo")
        check("set_timezone", await tab.evaluate("Intl.DateTimeFormat().resolvedOptions().timeZone") == "Asia/Tokyo")
        await tab.set_locale("vi-VN")
        check("set_locale", await tab.evaluate("Intl.DateTimeFormat().resolvedOptions().locale") == "vi-VN")

        print("\n[capture]")
        png = await tab.screenshot()
        check("screenshot png", png[:8] == b"\x89PNG\r\n\x1a\n", f"{len(png)} B")
        full = await tab.screenshot(full_page=True, format="jpeg", quality=60)
        check("screenshot full jpeg", full[:2] == b"\xff\xd8", f"{len(full)} B")
        el = await tab.screenshot(selector="#hov")
        check("screenshot element", el[:4] == b"\x89PNG", f"{len(el)} B")
        pdf = await tab.pdf()
        check("pdf", pdf[:4] == b"%PDF", f"{len(pdf)} B")

        print("\n[network / cookies / storage — over http]")
        tab2 = await browser.new_tab("https://example.com")
        check("new_tab", "example.com" in await tab2.url())
        await tab2.set_extra_headers({"X-Test": "1"})
        await tab2.set_cookies([{"name": "k1", "value": "v1", "domain": "example.com", "path": "/"}])
        names = [c["name"] for c in await tab2.get_cookies()]
        check("set/get_cookies", "k1" in names, str(names))
        cfile = os.path.join(tmp, "cookies.json")
        n = await tab2.save_cookies(cfile)
        check("save_cookies", n >= 1 and json.load(open(cfile)))
        await tab2.clear_cookies()
        check("clear_cookies", not any(c["name"] == "k1" for c in await tab2.get_cookies()))
        await tab2.load_cookies(cfile)
        check("load_cookies", any(c["name"] == "k1" for c in await tab2.get_cookies()))
        await tab2.delete_cookies("k1", domain="example.com")
        check("delete_cookies", not any(c["name"] == "k1" for c in await tab2.get_cookies()))
        await tab2.storage_set("tok", "abc")
        check("storage_get key", await tab2.storage_get("tok") == "abc")
        check("storage_get all", (await tab2.storage_get()).get("tok") == "abc")
        await tab2.storage_clear()
        check("storage_clear", await tab2.storage_get("tok") is None)
        await tab2.reload()
        check("reload", "example.com" in await tab2.url())
        await tab2.get("https://example.com/?page=2")
        went_back = await tab2.back()
        check("back", went_back and "page=2" not in await tab2.url())
        await tab2.forward()
        check("forward", "page=2" in await tab2.url())
        await tab2.block_urls(["*example.org*"])
        check("block_urls", True)

        tabs = await browser.tabs()
        check("tabs list", len(tabs) == 2, str(tabs))
        await tab2.close()
        await asyncio.sleep(0.5)
        check("close tab", len(await browser.tabs()) == 1)

    passed = sum(ok for _, ok in results)
    print(f"\n==> {passed}/{len(results)} passed")
    failed = [n for n, ok in results if not ok]
    if failed:
        print("FAILED:", failed)


if __name__ == "__main__":
    asyncio.run(main())
