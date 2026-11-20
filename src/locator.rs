// src/locator.rs — Rust 2021: Smart Locator JS codegen (XPath, text:, @attr, CSS + Deep Shadow DOM)

/// Quote a Rust string as a JS string literal (handles quotes, backslashes, unicode).
pub fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

/// Escape an `@attr` or `@attr=val` smart locator into a valid CSS attribute selector.
pub fn attr_to_css(attr_part: &str) -> String {
    let (attr_name, attr_val) = match attr_part.split_once('=') {
        Some((k, v)) => (k.trim(), v.trim().trim_matches('"').trim_matches('\'')),
        None => (attr_part.trim(), ""),
    };
    if attr_val.is_empty() {
        format!("[{}]", attr_name)
    } else {
        let escaped = attr_val.replace('\\', "\\\\").replace('"', "\\\"");
        format!("[{}=\"{}\"]", attr_name, escaped)
    }
}

/// Smart locator resolver supporting XPath, Text, Attributes, Shadow DOM, and standard CSS.
pub fn sel_expr(selector: &str) -> String {
    let s = selector.trim();
    if s.starts_with("//") || s.starts_with("(//") || s.starts_with(".//") {
        format!(
            "(() => {{ \
                const r = document.evaluate({}, document, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null); \
                return r.singleNodeValue; \
            }})()",
            js_str(s)
        )
    } else if let Some(txt) = s.strip_prefix("text:").or_else(|| s.strip_prefix("t:")) {
        format!(
            "(() => {{ \
                const target = {}.trim().toLowerCase(); \
                let best = null; \
                function walk(root) {{ \
                    for (const el of root.querySelectorAll('*')) {{ \
                        if (el.shadowRoot) walk(el.shadowRoot); \
                        if (el.getClientRects().length === 0) continue; \
                        const c = (el.innerText || el.textContent || el.value || el.getAttribute('aria-label') || '').trim().toLowerCase(); \
                        if (c.includes(target)) {{ \
                            if (!best || best.contains(el)) best = el; \
                        }} \
                    }} \
                }} \
                walk(document); \
                return best; \
            }})()",
            js_str(txt)
        )
    } else {
        let css_sel = if let Some(attr_part) = s.strip_prefix('@') {
            attr_to_css(attr_part)
        } else {
            s.to_string()
        };
        format!(
            "(() => {{ \
                const sel = {}; \
                const direct = document.querySelector(sel); \
                if (direct) return direct; \
                function queryDeep(root, q) {{ \
                    for (const child of root.querySelectorAll('*')) {{ \
                        if (child.shadowRoot) {{ \
                            const found = child.shadowRoot.querySelector(q) || queryDeep(child.shadowRoot, q); \
                            if (found) return found; \
                        }} \
                    }} \
                    return null; \
                }} \
                return queryDeep(document, sel); \
            }})()",
            js_str(&css_sel)
        )
    }
}

/// Multi-element smart locator resolver supporting XPath, Text, Attributes, Shadow DOM, and CSS.
pub fn sel_all_expr(selector: &str) -> String {
    let s = selector.trim();
    if s.starts_with("//") || s.starts_with("(//") || s.starts_with(".//") {
        format!(
            "(() => {{ \
                const snap = document.evaluate({}, document, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null); \
                const list = []; \
                for (let i = 0; i < snap.snapshotLength; i++) {{ \
                    const el = snap.snapshotItem(i); \
                    if (el && el.nodeType === 1) list.push(el); \
                }} \
                return list; \
            }})()",
            js_str(s)
        )
    } else if let Some(txt) = s.strip_prefix("text:").or_else(|| s.strip_prefix("t:")) {
        format!(
            "(() => {{ \
                const target = {}.trim().toLowerCase(); \
                const list = []; \
                function walk(root) {{ \
                    for (const el of root.querySelectorAll('*')) {{ \
                        if (el.shadowRoot) walk(el.shadowRoot); \
                        if (el.getClientRects().length === 0) continue; \
                        const c = (el.innerText || el.textContent || el.value || el.getAttribute('aria-label') || '').trim().toLowerCase(); \
                        if (c.includes(target)) list.push(el); \
                    }} \
                }} \
                walk(document); \
                return list; \
            }})()",
            js_str(txt)
        )
    } else {
        let css_sel = if let Some(attr_part) = s.strip_prefix('@') {
            attr_to_css(attr_part)
        } else {
            s.to_string()
        };
        format!(
            "(() => {{ \
                const sel = {}; \
                const list = []; \
                function queryDeepAll(root) {{ \
                    try {{ for (const el of root.querySelectorAll(sel)) list.push(el); }} catch (_) {{}} \
                    for (const child of root.querySelectorAll('*')) {{ \
                        if (child.shadowRoot) queryDeepAll(child.shadowRoot); \
                    }} \
                }} \
                queryDeepAll(document); \
                return list; \
            }})()",
            js_str(&css_sel)
        )
    }
}

/// Returns `(functionDeclaration, argument)` for querying a single child relative to `this` (including `this.shadowRoot`).
pub fn child_sel_fn(query: &str) -> (&'static str, String) {
    let s = query.trim();
    if s.starts_with("//") || s.starts_with("(//") || s.starts_with(".//") {
        (
            "function(xp) { \
                const r = document.evaluate(xp, this, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null); \
                const el = r.singleNodeValue; \
                return (el && el.nodeType === 1) ? el : null; \
            }",
            s.to_string(),
        )
    } else if let Some(txt) = s.strip_prefix("text:").or_else(|| s.strip_prefix("t:")) {
        (
            "function(t) { \
                const target = t.trim().toLowerCase(); \
                let best = null; \
                function walk(root) { \
                    for (const el of root.querySelectorAll('*')) { \
                        if (el.shadowRoot) walk(el.shadowRoot); \
                        if (el.getClientRects().length === 0) continue; \
                        const c = (el.innerText || el.textContent || el.value || el.getAttribute('aria-label') || '').trim().toLowerCase(); \
                        if (c.includes(target)) { if (!best || best.contains(el)) best = el; } \
                    } \
                } \
                walk(this); \
                if (this.shadowRoot) walk(this.shadowRoot); \
                return best; \
            }",
            txt.to_string(),
        )
    } else {
        let css_sel = if let Some(attr_part) = s.strip_prefix('@') {
            attr_to_css(attr_part)
        } else {
            s.to_string()
        };
        (
            "function(sel) { \
                function queryDeep(root, q) { \
                    try { const direct = root.querySelector(q); if (direct) return direct; } catch (_) {} \
                    for (const child of root.querySelectorAll('*')) { \
                        if (child.shadowRoot) { \
                            const found = queryDeep(child.shadowRoot, q); \
                            if (found) return found; \
                        } \
                    } \
                    return null; \
                } \
                const direct = queryDeep(this, sel); \
                if (direct) return direct; \
                return this.shadowRoot ? queryDeep(this.shadowRoot, sel) : null; \
            }",
            css_sel,
        )
    }
}

/// Returns `(functionDeclaration, argument)` for querying all children relative to `this` (including `this.shadowRoot`).
pub fn child_sel_all_fn(query: &str) -> (&'static str, String) {
    let s = query.trim();
    if s.starts_with("//") || s.starts_with("(//") || s.starts_with(".//") {
        (
            "function(xp) { \
                const snap = document.evaluate(xp, this, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null); \
                const list = []; \
                for (let i = 0; i < snap.snapshotLength; i++) { \
                    const el = snap.snapshotItem(i); \
                    if (el && el.nodeType === 1) list.push(el); \
                } \
                return list; \
            }",
            s.to_string(),
        )
    } else if let Some(txt) = s.strip_prefix("text:").or_else(|| s.strip_prefix("t:")) {
        (
            "function(t) { \
                const target = t.trim().toLowerCase(); \
                const list = []; \
                function walk(root) { \
                    for (const el of root.querySelectorAll('*')) { \
                        if (el.shadowRoot) walk(el.shadowRoot); \
                        if (el.getClientRects().length === 0) continue; \
                        const c = (el.innerText || el.textContent || el.value || el.getAttribute('aria-label') || '').trim().toLowerCase(); \
                        if (c.includes(target)) list.push(el); \
                    } \
                } \
                walk(this); \
                if (this.shadowRoot) walk(this.shadowRoot); \
                return list; \
            }",
            txt.to_string(),
        )
    } else {
        let css_sel = if let Some(attr_part) = s.strip_prefix('@') {
            attr_to_css(attr_part)
        } else {
            s.to_string()
        };
        (
            "function(sel) { \
                const list = []; \
                function queryDeepAll(root) { \
                    try { for (const el of root.querySelectorAll(sel)) list.push(el); } catch (_) {} \
                    for (const child of root.querySelectorAll('*')) { \
                        if (child.shadowRoot) queryDeepAll(child.shadowRoot); \
                    } \
                } \
                queryDeepAll(this); \
                if (this.shadowRoot) queryDeepAll(this.shadowRoot); \
                return list; \
            }",
            css_sel,
        )
    }
}

pub const NODE_META_FN: &str = "function() { \
    const r = this.getBoundingClientRect(); \
    return { \
        tag: (this.tagName || '').toLowerCase(), \
        text: (this.innerText || this.textContent || '').trim(), \
        value: 'value' in this ? this.value : null, \
        id: this.id || null, \
        class: typeof this.className === 'string' ? this.className : (this.getAttribute ? this.getAttribute('class') : null), \
        rect: { x: r.left, y: r.top, width: r.width, height: r.height } \
    }; \
}";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attr_to_css_handles_bare_and_quoted_values() {
        assert_eq!(attr_to_css("disabled"), "[disabled]");
        assert_eq!(attr_to_css("name=email"), "[name=\"email\"]");
        assert_eq!(attr_to_css("placeholder='Enter name'"), "[placeholder=\"Enter name\"]");
        assert_eq!(attr_to_css("data-x=a\"b\\c"), "[data-x=\"a\\\"b\\\\c\"]");
    }

    #[test]
    fn sel_expr_routes_all_locator_syntaxes() {
        assert!(sel_expr("//button[@id='ok']").contains("document.evaluate"));
        assert!(sel_expr("text:Sign In").contains("innerText"));
        assert!(sel_expr("t:Submit").contains("innerText"));
        assert!(sel_expr("@name=q").contains("[name=\\\"q\\\"]"));
        assert!(sel_expr("#login").contains("queryDeep"));
    }

    #[test]
    fn child_sel_fn_dispatches_correctly() {
        let (f_xp, arg_xp) = child_sel_fn(".//span");
        assert!(f_xp.contains("document.evaluate"));
        assert_eq!(arg_xp, ".//span");

        let (f_txt, arg_txt) = child_sel_fn("text:Hello");
        assert!(f_txt.contains("shadowRoot"));
        assert_eq!(arg_txt, "Hello");

        let (f_attr, arg_attr) = child_sel_fn("@data-id=42");
        assert!(f_attr.contains("queryDeep"));
        assert_eq!(arg_attr, "[data-id=\"42\"]");
    }
}
