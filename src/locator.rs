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

// ───────────────────── CDP Pierced DOM Matcher (Closed Shadow Roots) ─────────────────────

fn cdp_attr<'a>(node: &'a serde_json::Value, name: &str) -> Option<&'a str> {
    let attrs = node.get("attributes")?.as_array()?;
    let mut i = 0;
    while i + 1 < attrs.len() {
        if let Some(k) = attrs[i].as_str() {
            if k.eq_ignore_ascii_case(name) {
                return attrs[i + 1].as_str();
            }
        }
        i += 2;
    }
    None
}

fn cdp_collect_text(node: &serde_json::Value, buf: &mut String) {
    if node.get("nodeType").and_then(|v| v.as_i64()) == Some(3) {
        if let Some(val) = node.get("nodeValue").and_then(|v| v.as_str()) {
            buf.push_str(val);
        }
        return;
    }
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for c in children {
            cdp_collect_text(c, buf);
        }
    }
}

/// Checks whether a CDP `DOM.Node` matches a simple compound selector (`tag`, `#id`, `.cls`, `[attr]`, `[attr=val]`).
fn matches_simple_css(node: &serde_json::Value, sel: &str) -> bool {
    if node.get("nodeType").and_then(|v| v.as_i64()) != Some(1) {
        return false;
    }
    let tag_name = node
        .get("localName")
        .or_else(|| node.get("nodeName"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let bytes = sel.as_bytes();
    let mut i = 0;
    let mut matched_any = false;

    // Optional leading tag name (e.g. `input`, `button`, `*`)
    let start = i;
    while i < bytes.len() && !matches!(bytes[i], b'#' | b'.' | b'[') {
        i += 1;
    }
    if i > start {
        let want_tag = sel[start..i].trim();
        if !want_tag.is_empty() && want_tag != "*" && !tag_name.eq_ignore_ascii_case(want_tag) {
            return false;
        }
        matched_any = true;
    }

    while i < bytes.len() {
        match bytes[i] {
            b'#' => {
                i += 1;
                let s = i;
                while i < bytes.len() && !matches!(bytes[i], b'#' | b'.' | b'[') {
                    i += 1;
                }
                let want_id = &sel[s..i];
                if cdp_attr(node, "id") != Some(want_id) {
                    return false;
                }
                matched_any = true;
            }
            b'.' => {
                i += 1;
                let s = i;
                while i < bytes.len() && !matches!(bytes[i], b'#' | b'.' | b'[') {
                    i += 1;
                }
                let want_cls = &sel[s..i];
                let has_cls = cdp_attr(node, "class")
                    .map(|c| c.split_whitespace().any(|tok| tok == want_cls))
                    .unwrap_or(false);
                if !has_cls {
                    return false;
                }
                matched_any = true;
            }
            b'[' => {
                i += 1;
                let s = i;
                while i < bytes.len() && bytes[i] != b']' {
                    i += 1;
                }
                if i >= bytes.len() {
                    return false;
                }
                let inner = sel[s..i].trim();
                i += 1; // skip ']'
                if let Some((k, v)) = inner.split_once("*=") {
                    let k = k.trim();
                    let v = v.trim().trim_matches('"').trim_matches('\'');
                    if !cdp_attr(node, k).map(|a| a.contains(v)).unwrap_or(false) {
                        return false;
                    }
                } else if let Some((k, v)) = inner.split_once("^=") {
                    let k = k.trim();
                    let v = v.trim().trim_matches('"').trim_matches('\'');
                    if !cdp_attr(node, k).map(|a| a.starts_with(v)).unwrap_or(false) {
                        return false;
                    }
                } else if let Some((k, v)) = inner.split_once("$=") {
                    let k = k.trim();
                    let v = v.trim().trim_matches('"').trim_matches('\'');
                    if !cdp_attr(node, k).map(|a| a.ends_with(v)).unwrap_or(false) {
                        return false;
                    }
                } else if let Some((k, v)) = inner.split_once('=') {
                    let k = k.trim();
                    let v = v.trim().trim_matches('"').trim_matches('\'');
                    if cdp_attr(node, k) != Some(v) {
                        return false;
                    }
                } else if cdp_attr(node, inner).is_none() {
                    return false;
                }
                matched_any = true;
            }
            _ => return false,
        }
    }
    matched_any
}

/// Walks a `DOM.getDocument({ depth: -1, pierce: true })` tree (including `#shadow-root (closed)`)
/// and returns matching `backendNodeId`s for CSS, `@attr=val`, or `text:`/`t:` queries.
pub fn find_pierced_backend_node_ids(root: &serde_json::Value, query: &str) -> Vec<i64> {
    let s = query.trim();
    if s.is_empty() || s.starts_with("//") || s.starts_with("(//") || s.starts_with(".//") {
        return Vec::new();
    }

    if let Some(txt) = s.strip_prefix("text:").or_else(|| s.strip_prefix("t:")) {
        let needle = txt.trim().to_lowercase();
        let mut out = Vec::new();
        fn walk_text(node: &serde_json::Value, needle: &str, out: &mut Vec<i64>) -> bool {
            let mut child_matched = false;
            if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
                for c in children {
                    if walk_text(c, needle, out) {
                        child_matched = true;
                    }
                }
            }
            if let Some(shadows) = node.get("shadowRoots").and_then(|v| v.as_array()) {
                for sr in shadows {
                    if walk_text(sr, needle, out) {
                        child_matched = true;
                    }
                }
            }
            if let Some(cd) = node.get("contentDocument") {
                if walk_text(cd, needle, out) {
                    child_matched = true;
                }
            }
            if child_matched {
                return true;
            }
            if node.get("nodeType").and_then(|v| v.as_i64()) == Some(1) {
                let mut text = String::new();
                cdp_collect_text(node, &mut text);
                if text.trim().to_lowercase().contains(needle) {
                    if let Some(bid) = node.get("backendNodeId").and_then(|v| v.as_i64()) {
                        out.push(bid);
                        return true;
                    }
                }
            }
            false
        }
        walk_text(root, &needle, &mut out);
        return out;
    }

    let css = if let Some(attr_part) = s.strip_prefix('@') {
        attr_to_css(attr_part)
    } else {
        s.to_string()
    };

    // Support whitespace-separated descendant selectors (e.g. `form input[type="checkbox"]`)
    let parts: Vec<&str> = css.split_whitespace().filter(|p| *p != ">").collect();
    if parts.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    fn walk_css<'a>(
        node: &'a serde_json::Value,
        parts: &[&str],
        ancestors: &mut Vec<&'a serde_json::Value>,
        out: &mut Vec<i64>,
    ) {
        let is_elem = node.get("nodeType").and_then(|v| v.as_i64()) == Some(1);
        if is_elem && matches_simple_css(node, parts[parts.len() - 1]) {
            // Verify remaining ancestor selector parts in reverse order
            let mut need = parts.len() - 1;
            if need == 0 {
                if let Some(bid) = node.get("backendNodeId").and_then(|v| v.as_i64()) {
                    out.push(bid);
                }
            } else {
                for anc in ancestors.iter().rev() {
                    if matches_simple_css(anc, parts[need - 1]) {
                        need -= 1;
                        if need == 0 {
                            break;
                        }
                    }
                }
                if need == 0 {
                    if let Some(bid) = node.get("backendNodeId").and_then(|v| v.as_i64()) {
                        out.push(bid);
                    }
                }
            }
        }

        if is_elem {
            ancestors.push(node);
        }
        if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
            for c in children {
                walk_css(c, parts, ancestors, out);
            }
        }
        if let Some(shadows) = node.get("shadowRoots").and_then(|v| v.as_array()) {
            for sr in shadows {
                walk_css(sr, parts, ancestors, out);
            }
        }
        if let Some(cd) = node.get("contentDocument") {
            walk_css(cd, parts, ancestors, out);
        }
        if is_elem {
            ancestors.pop();
        }
    }

    let mut ancestors = Vec::new();
    walk_css(root, &parts, &mut ancestors, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    #[test]
    fn pierced_matcher_finds_nodes_inside_closed_shadow_root() {
        let doc = json!({
            "nodeType": 9,
            "children": [{
                "nodeType": 1,
                "localName": "div",
                "backendNodeId": 10,
                "attributes": ["id", "host"],
                "shadowRoots": [{
                    "nodeType": 11,
                    "shadowRootType": "closed",
                    "children": [{
                        "nodeType": 1,
                        "localName": "label",
                        "backendNodeId": 20,
                        "attributes": ["class", "cb-wrap active"],
                        "children": [
                            {
                                "nodeType": 1,
                                "localName": "input",
                                "backendNodeId": 21,
                                "attributes": ["id", "cf-cb", "type", "checkbox"]
                            },
                            {
                                "nodeType": 3,
                                "nodeValue": "Verify you are human"
                            }
                        ]
                    }]
                }]
            }]
        });

        assert_eq!(find_pierced_backend_node_ids(&doc, "#cf-cb"), vec![21]);
        assert_eq!(find_pierced_backend_node_ids(&doc, "input[type=\"checkbox\"]"), vec![21]);
        assert_eq!(find_pierced_backend_node_ids(&doc, "@type=checkbox"), vec![21]);
        assert_eq!(find_pierced_backend_node_ids(&doc, "label.active input#cf-cb"), vec![21]);
        assert_eq!(find_pierced_backend_node_ids(&doc, "input[id*=\"cf-\"]"), vec![21]);
        assert_eq!(find_pierced_backend_node_ids(&doc, "input[type^=\"check\"]"), vec![21]);
        assert_eq!(find_pierced_backend_node_ids(&doc, "input[type$=\"box\"]"), vec![21]);
        assert_eq!(find_pierced_backend_node_ids(&doc, "text:Verify you are human"), vec![20]);
    }
}
