import asyncio
import best_driver

async def run():
    print("Launching browser...")
    b = await best_driver.start(headless=True)
    tab = await b.get("about:blank")

    html = """
    (() => {
        document.body.innerHTML = `
            <div id="container">
                <input id="inp" placeholder="Nhập tên đăng nhập" name="username" />
                <button id="btn" class="primary-btn">Xác nhận ngay</button>
                <div id="shadow-host"></div>
            </div>
        `;
        const host = document.getElementById('shadow-host');
        const root = host.attachShadow({ mode: 'open' });
        root.innerHTML = '<span id="secret-shadow" class="shadow-item">Dữ liệu trong Shadow DOM</span>';
    })()
    """
    await tab.evaluate(html)

    # 1. Test standard CSS
    c1 = await tab.exists("#btn")

    # 2. Test XPath
    c2 = await tab.exists("//button[@id='btn']")

    # 3. Test Text locator
    c3 = await tab.exists("text:Xác nhận ngay")
    t_text = await tab.text("text:Xác nhận ngay")

    # 4. Test Attribute locator
    c4 = await tab.exists("@placeholder=Nhập tên đăng nhập")
    c5 = await tab.exists("@name=username")

    # 5. Test Shadow DOM piercing
    c6 = await tab.exists("#secret-shadow")
    shadow_txt = await tab.text("#secret-shadow")

    # 6. Test ele() metadata
    info = await tab.ele("@name=username")

    # 7. Test human_type
    await tab.human_type("@placeholder=Nhập tên đăng nhập", "Ritmada User")
    typed_val = await tab.value("@name=username")

    # 8. Test swipe
    await tab.swipe(100.0, 100.0, 300.0, 100.0)

    print("--- SMART LOCATOR VERIFICATION RESULTS ---")
    print(f"1. CSS (#btn): {c1}")
    print(f"2. XPath (//button...): {c2}")
    print(f"3. Text (text:Xác nhận ngay): {c3} | Content: '{t_text}'")
    print(f"4. Attribute (@placeholder=...): {c4}")
    print(f"5. Attribute (@name=...): {c5}")
    print(f"6. Shadow DOM (#secret-shadow): {c6} | Content: '{shadow_txt}'")
    print(f"7. ele() metadata: tag={info.get('tag')}, id={info.get('id')}, value={info.get('value')}")
    print(f"8. human_type result: '{typed_val}'")
    print("9. swipe gesture: OK")

    await b.stop()
    print("Browser stopped cleanly.")

if __name__ == "__main__":
    asyncio.run(run())
