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

/// Split a Smart Locator on top-level commas (outside quotes, `[]`, and `()`).
///
/// Preserves literal commas in single `text:` / `t:` queries (e.g. `"text:Hello, world"`)
/// unless at least one subsequent branch starts with a Smart Locator or CSS-sigil prefix.
pub fn split_locator_branches(selector: &str) -> Vec<&str> {
    let s = selector.trim();
    if s.is_empty() {
        return Vec::new();
    }
    let bytes = s.as_bytes();
    let mut branches = Vec::new();
    let mut start = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;

    for (i, &b) in bytes.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if (in_single || in_double) && b == b'\\' {
            escape = true;
            continue;
        }
        match b {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'[' if !in_single && !in_double => bracket_depth += 1,
            b']' if !in_single && !in_double => bracket_depth = bracket_depth.saturating_sub(1),
            b'(' if !in_single && !in_double => paren_depth += 1,
            b')' if !in_single && !in_double => paren_depth = paren_depth.saturating_sub(1),
            b',' if !in_single && !in_double && bracket_depth == 0 && paren_depth == 0 => {
                let part = s[start..i].trim();
                if !part.is_empty() {
                    branches.push(part);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let tail = s[start..].trim();
    if !tail.is_empty() {
        branches.push(tail);
    }

    if branches.len() > 1 && (s.starts_with("text:") || s.starts_with("t:") || s.starts_with("text=:") || s.starts_with("label:") || s.starts_with("label=:")) {
        let has_explicit_branch = branches[1..].iter().any(|b| {
            b.starts_with('@')
                || b.starts_with("text:")
                || b.starts_with("t:")
                || b.starts_with("text=:")
                || b.starts_with("role:")
                || b.starts_with("label:")
                || b.starts_with("label=:")
                || b.starts_with("//")
                || b.starts_with("(//")
                || b.starts_with(".//")
                || b.starts_with('#')
                || b.starts_with('.')
                || b.starts_with('[')
        });
        if !has_explicit_branch {
            return vec![s];
        }
    }

    branches
}

#[inline]
fn is_xpath(s: &str) -> bool {
    s.starts_with("//") || s.starts_with("(//") || s.starts_with(".//")
}

#[inline]
fn strip_text_prefix(s: &str) -> Option<(&str, bool)> {
    if let Some(rest) = s.strip_prefix("text=:") {
        Some((rest, true))
    } else if let Some(rest) = s.strip_prefix("text:").or_else(|| s.strip_prefix("t:")) {
        Some((rest, false))
    } else {
        None
    }
}

#[inline]
fn strip_label_prefix(s: &str) -> Option<(&str, bool)> {
    if let Some(rest) = s.strip_prefix("label=:") {
        Some((rest, true))
    } else if let Some(rest) = s.strip_prefix("label:") {
        Some((rest, false))
    } else {
        None
    }
}

/// Parses `role:button`, `role:button[name="Submit"]`, or `role:button[name="Submit" exact]`
fn parse_role_query(s: &str) -> Option<(String, Option<String>, bool)> {
    let rest = s.strip_prefix("role:")?.trim();
    if rest.is_empty() {
        return None;
    }
    if let Some((role_part, bracket)) = rest.split_once('[') {
        let role = role_part.trim().to_ascii_lowercase();
        let inner = bracket.trim().strip_suffix(']')?.trim();
        let mut exact = false;
        let mut inner_clean = inner;
        if let Some(before) = inner_clean.strip_suffix("exact") {
            exact = true;
            inner_clean = before.trim();
        }
        let name = if let Some((_, val)) = inner_clean.split_once('=') {
            let v = val.trim();
            let unquoted = v
                .strip_prefix('"')
                .and_then(|x| x.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
                .unwrap_or(v);
            Some(unquoted.replace("\\\"", "\"").replace("\\\\", "\\"))
        } else {
            None
        };
        return Some((role, name, exact));
    }
    Some((rest.to_ascii_lowercase(), None, false))
}

fn branch_to_css(b: &str) -> Option<String> {
    if is_xpath(b)
        || strip_text_prefix(b).is_some()
        || strip_label_prefix(b).is_some()
        || parse_role_query(b).is_some()
    {
        None
    } else if let Some(attr_part) = b.strip_prefix('@') {
        Some(attr_to_css(attr_part))
    } else {
        Some(b.to_string())
    }
}

const SEMANTIC_JS_HELPERS: &str = r#"
function __bdImplicitRole(el) {
    const explicit = (el.getAttribute && el.getAttribute('role') || '').trim().toLowerCase();
    if (explicit) return explicit;
    const tag = (el.tagName || '').toLowerCase();
    if (tag === 'button') return 'button';
    if (tag === 'a' && el.hasAttribute('href')) return 'link';
    if (tag === 'area' && el.hasAttribute('href')) return 'link';
    if (tag === 'textarea') return 'textbox';
    if (tag === 'select') return el.multiple || (el.size && el.size > 1) ? 'listbox' : 'combobox';
    if (tag === 'img') return 'img';
    if (tag === 'li') return 'listitem';
    if (tag === 'ul' || tag === 'ol') return 'list';
    if (tag === 'nav') return 'navigation';
    if (tag === 'main') return 'main';
    if (tag === 'form') return 'form';
    if (tag === 'dialog') return 'dialog';
    if (tag === 'table') return 'table';
    if (tag === 'tr') return 'row';
    if (tag === 'td') return 'cell';
    if (tag === 'th') return 'columnheader';
    if (tag === 'option') return 'option';
    if (tag === 'progress') return 'progressbar';
    if (/^h[1-6]$/.test(tag)) return 'heading';
    if (tag === 'input') {
        const tp = (el.type || 'text').toLowerCase();
        if (tp === 'button' || tp === 'submit' || tp === 'reset' || tp === 'image') return 'button';
        if (tp === 'checkbox') return 'checkbox';
        if (tp === 'radio') return 'radio';
        if (tp === 'search') return 'searchbox';
        if (tp === 'number') return 'spinbutton';
        if (tp === 'range') return 'slider';
        if (tp === 'hidden') return '';
        return 'textbox';
    }
    if (el.getAttribute && el.getAttribute('contenteditable') === 'true') return 'textbox';
    return '';
}
function __bdAccName(el) {
    const doc = el.ownerDocument || document;
    const al = el.getAttribute && el.getAttribute('aria-label');
    if (al) return al.trim();
    const lb = el.getAttribute && el.getAttribute('aria-labelledby');
    if (lb) {
        const txt = lb.split(/\s+/).map(id => {
            const ref = doc.getElementById(id);
            return ref ? (ref.innerText || ref.textContent || '').trim() : '';
        }).filter(Boolean).join(' ').trim();
        if (txt) return txt;
    }
    if (el.labels && el.labels.length) {
        const lt = Array.from(el.labels).map(l => (l.innerText || l.textContent || '').trim()).filter(Boolean).join(' ').trim();
        if (lt) return lt;
    }
    const tag = (el.tagName || '').toLowerCase();
    if (tag === 'input' && (el.type === 'button' || el.type === 'submit' || el.type === 'reset') && el.value) {
        return String(el.value).trim();
    }
    if (tag === 'img' && el.alt) return el.alt.trim();
    const inner = (el.innerText || el.textContent || '').trim();
    if (inner) return inner;
    const ph = el.getAttribute && el.getAttribute('placeholder');
    if (ph) return ph.trim();
    const title = el.getAttribute && el.getAttribute('title');
    return title ? title.trim() : '';
}
function __bdMatchRoleAll(rootNode, wantRole, wantName, exact) {
    const out = [];
    const seen = new Set();
    const wantLow = wantName !== null ? wantName.trim().toLowerCase() : null;
    function walk(root) {
        for (const el of root.querySelectorAll('*')) {
            if (el.shadowRoot) walk(el.shadowRoot);
            if (__bdImplicitRole(el) !== wantRole) continue;
            if (wantLow !== null) {
                const nm = __bdAccName(el).toLowerCase();
                if (exact ? nm !== wantLow : !nm.includes(wantLow)) continue;
            }
            if (!seen.has(el)) { seen.add(el); out.push(el); }
        }
    }
    walk(rootNode);
    if (rootNode.shadowRoot) walk(rootNode.shadowRoot);
    return out;
}
function __bdMatchLabelAll(rootNode, wantLabel, exact) {
    const out = [];
    const seen = new Set();
    const add = (el) => { if (el && el.nodeType === 1 && !seen.has(el)) { seen.add(el); out.push(el); } };
    const wantLow = wantLabel.trim().toLowerCase();
    const matchStr = (s) => { const low = (s || '').trim().toLowerCase(); return exact ? low === wantLow : low.includes(wantLow); };
    function walk(root) {
        for (const el of root.querySelectorAll('*')) {
            if (el.shadowRoot) walk(el.shadowRoot);
            const tag = (el.tagName || '').toLowerCase();
            if (tag === 'label') {
                const txt = (el.innerText || el.textContent || '').trim();
                if (matchStr(txt)) {
                    if (el.control) add(el.control);
                    else {
                        const forId = el.getAttribute('for');
                        const target = forId ? (root.getElementById ? root.getElementById(forId) : document.getElementById(forId)) : el.querySelector('input,textarea,select');
                        if (target) add(target);
                    }
                }
            } else {
                const al = el.getAttribute && el.getAttribute('aria-label');
                if (al && matchStr(al)) add(el);
            }
        }
    }
    walk(rootNode);
    if (rootNode.shadowRoot) walk(rootNode.shadowRoot);
    return out;
}
"#;

fn sel_single_expr(s: &str) -> String {
    if is_xpath(s) {
        format!(
            "(() => {{ \
                const r = document.evaluate({}, document, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null); \
                return r.singleNodeValue; \
            }})()",
            js_str(s)
        )
    } else if let Some((role, name, exact)) = parse_role_query(s) {
        let name_js = match name {
            Some(ref n) => js_str(n),
            None => "null".to_string(),
        };
        format!(
            "(() => {{ {} const arr = __bdMatchRoleAll(document, {}, {}, {}); return arr[0] || null; }})()",
            SEMANTIC_JS_HELPERS,
            js_str(&role),
            name_js,
            exact
        )
    } else if let Some((lbl, exact)) = strip_label_prefix(s) {
        format!(
            "(() => {{ {} const arr = __bdMatchLabelAll(document, {}, {}); return arr[0] || null; }})()",
            SEMANTIC_JS_HELPERS,
            js_str(lbl),
            exact
        )
    } else if let Some((txt, exact)) = strip_text_prefix(s) {
        format!(
            "(() => {{ \
                const target = {}.trim().toLowerCase(); const exact = {}; \
                let best = null; \
                function walk(root) {{ \
                    for (const el of root.querySelectorAll('*')) {{ \
                        if (el.shadowRoot) walk(el.shadowRoot); \
                        if (el.getClientRects().length === 0) continue; \
                        const c = (el.innerText || el.textContent || el.value || el.getAttribute('aria-label') || '').trim().toLowerCase(); \
                        if (exact ? c === target : c.includes(target)) {{ \
                            if (!best || best.contains(el)) best = el; \
                        }} \
                    }} \
                }} \
                walk(document); \
                return best; \
            }})()",
            js_str(txt), exact
        )
    } else {
        let css_sel = branch_to_css(s).unwrap_or_else(|| s.to_string());
        format!(
            "(() => {{ \
                const sel = {}; \
                try {{ const direct = document.querySelector(sel); if (direct) return direct; }} catch (_) {{}} \
                function queryDeep(root, q) {{ \
                    for (const child of root.querySelectorAll('*')) {{ \
                        if (child.shadowRoot) {{ \
                            try {{ const direct = child.shadowRoot.querySelector(q); if (direct) return direct; }} catch (_) {{}} \
                            const found = queryDeep(child.shadowRoot, q); \
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

fn sel_single_all_expr(s: &str) -> String {
    if is_xpath(s) {
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
    } else if let Some((role, name, exact)) = parse_role_query(s) {
        let name_js = match name {
            Some(ref n) => js_str(n),
            None => "null".to_string(),
        };
        format!(
            "(() => {{ {} return __bdMatchRoleAll(document, {}, {}, {}); }})()",
            SEMANTIC_JS_HELPERS,
            js_str(&role),
            name_js,
            exact
        )
    } else if let Some((lbl, exact)) = strip_label_prefix(s) {
        format!(
            "(() => {{ {} return __bdMatchLabelAll(document, {}, {}); }})()",
            SEMANTIC_JS_HELPERS,
            js_str(lbl),
            exact
        )
    } else if let Some((txt, exact)) = strip_text_prefix(s) {
        format!(
            "(() => {{ \
                const target = {}.trim().toLowerCase(); const exact = {}; \
                const list = []; \
                function walk(root) {{ \
                    for (const el of root.querySelectorAll('*')) {{ \
                        if (el.shadowRoot) walk(el.shadowRoot); \
                        if (el.getClientRects().length === 0) continue; \
                        const c = (el.innerText || el.textContent || el.value || el.getAttribute('aria-label') || '').trim().toLowerCase(); \
                        if (exact ? c === target : c.includes(target)) list.push(el); \
                    }} \
                }} \
                walk(document); \
                return list; \
            }})()",
            js_str(txt), exact
        )
    } else {
        let css_sel = branch_to_css(s).unwrap_or_else(|| s.to_string());
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

/// Smart locator resolver supporting XPath, Text, Attributes, Shadow DOM, standard CSS, and comma-separated mixed branches.
pub fn sel_expr(selector: &str) -> String {
    let branches = split_locator_branches(selector);
    if branches.is_empty() {
        return "null".to_string();
    }
    if branches.len() == 1 {
        return sel_single_expr(branches[0]);
    }
    // Fast path: all branches are CSS or `@attr` -> combine into a single valid CSS selector list
    if let Some(css_parts) = branches.iter().map(|b| branch_to_css(b)).collect::<Option<Vec<_>>>() {
        return sel_single_expr(&css_parts.join(", "));
    }
    let mut body = String::from("(() => { ");
    for b in branches {
        body.push_str("try { const r = ");
        body.push_str(&sel_single_expr(b));
        body.push_str("; if (r) return r; } catch (_) {} ");
    }
    body.push_str("return null; })()");
    body
}

/// Multi-element smart locator resolver supporting XPath, Text, Attributes, Shadow DOM, CSS, and comma-separated mixed branches.
pub fn sel_all_expr(selector: &str) -> String {
    let branches = split_locator_branches(selector);
    if branches.is_empty() {
        return "[]".to_string();
    }
    if branches.len() == 1 {
        return sel_single_all_expr(branches[0]);
    }
    if let Some(css_parts) = branches.iter().map(|b| branch_to_css(b)).collect::<Option<Vec<_>>>() {
        return sel_single_all_expr(&css_parts.join(", "));
    }
    let mut body = String::from("(() => { const out = []; const seen = new Set(); ");
    for b in branches {
        body.push_str("try { for (const el of ");
        body.push_str(&sel_single_all_expr(b));
        body.push_str(") { if (el && !seen.has(el)) { seen.add(el); out.push(el); } } } catch (_) {} ");
    }
    body.push_str("return out; })()");
    body
}

const CHILD_XPATH_ONE_FN: &str = "function(xp) { \
    const r = document.evaluate(xp, this, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null); \
    const el = r.singleNodeValue; \
    return (el && el.nodeType === 1) ? el : null; \
}";

const CHILD_TEXT_ONE_FN: &str = "function(t) { \
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
}";

const CHILD_CSS_ONE_FN: &str = "function(sel) { \
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
}";

const CHILD_MIXED_ONE_FN: &str = "function(branches) { \
    const self = this; \
    function queryDeep(root, q) { \
        try { const d = root.querySelector(q); if (d) return d; } catch (_) {} \
        for (const c of root.querySelectorAll('*')) { \
            if (c.shadowRoot) { const f = queryDeep(c.shadowRoot, q); if (f) return f; } \
        } \
        return null; \
    } \
    for (const b of branches) { \
        try { \
            if (b.k === 'xpath') { \
                const r = document.evaluate(b.v, self, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null); \
                const el = r.singleNodeValue; \
                if (el && el.nodeType === 1) return el; \
            } else if (b.k === 'text') { \
                const target = b.v.trim().toLowerCase(); \
                let best = null; \
                const walk = (root) => { \
                    for (const el of root.querySelectorAll('*')) { \
                        if (el.shadowRoot) walk(el.shadowRoot); \
                        if (el.getClientRects().length === 0) continue; \
                        const c = (el.innerText || el.textContent || el.value || el.getAttribute('aria-label') || '').trim().toLowerCase(); \
                        if (c.includes(target)) { if (!best || best.contains(el)) best = el; } \
                    } \
                }; \
                walk(self); \
                if (self.shadowRoot) walk(self.shadowRoot); \
                if (best) return best; \
            } else { \
                const found = queryDeep(self, b.v) || (self.shadowRoot ? queryDeep(self.shadowRoot, b.v) : null); \
                if (found) return found; \
            } \
        } catch (_) {} \
    } \
    return null; \
}";

const CHILD_XPATH_ALL_FN: &str = "function(xp) { \
    const snap = document.evaluate(xp, this, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null); \
    const list = []; \
    for (let i = 0; i < snap.snapshotLength; i++) { \
        const el = snap.snapshotItem(i); \
        if (el && el.nodeType === 1) list.push(el); \
    } \
    return list; \
}";

const CHILD_TEXT_ALL_FN: &str = "function(t) { \
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
}";

const CHILD_CSS_ALL_FN: &str = "function(sel) { \
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
}";

const CHILD_MIXED_ALL_FN: &str = "function(branches) { \
    const self = this; \
    const out = []; \
    const seen = new Set(); \
    const add = (el) => { if (el && !seen.has(el)) { seen.add(el); out.push(el); } }; \
    function queryDeepAll(root, q) { \
        try { for (const el of root.querySelectorAll(q)) add(el); } catch (_) {} \
        for (const c of root.querySelectorAll('*')) { if (c.shadowRoot) queryDeepAll(c.shadowRoot, q); } \
    } \
    for (const b of branches) { \
        try { \
            if (b.k === 'xpath') { \
                const snap = document.evaluate(b.v, self, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null); \
                for (let i = 0; i < snap.snapshotLength; i++) { \
                    const el = snap.snapshotItem(i); \
                    if (el && el.nodeType === 1) add(el); \
                } \
            } else if (b.k === 'text') { \
                const target = b.v.trim().toLowerCase(); \
                const walk = (root) => { \
                    for (const el of root.querySelectorAll('*')) { \
                        if (el.shadowRoot) walk(el.shadowRoot); \
                        if (el.getClientRects().length === 0) continue; \
                        const c = (el.innerText || el.textContent || el.value || el.getAttribute('aria-label') || '').trim().toLowerCase(); \
                        if (c.includes(target)) add(el); \
                    } \
                }; \
                walk(self); \
                if (self.shadowRoot) walk(self.shadowRoot); \
            } else { \
                queryDeepAll(self, b.v); \
                if (self.shadowRoot) queryDeepAll(self.shadowRoot, b.v); \
            } \
        } catch (_) {} \
    } \
    return out; \
}";

const CHILD_ROLE_ONE_FN: &str = r#"function(spec) {
    function __bdImplicitRole(el) {
        const explicit = (el.getAttribute && el.getAttribute('role') || '').trim().toLowerCase();
        if (explicit) return explicit;
        const tag = (el.tagName || '').toLowerCase();
        if (tag === 'button') return 'button';
        if ((tag === 'a' || tag === 'area') && el.hasAttribute('href')) return 'link';
        if (tag === 'textarea') return 'textbox';
        if (tag === 'select') return el.multiple || (el.size && el.size > 1) ? 'listbox' : 'combobox';
        if (tag === 'img') return 'img';
        if (tag === 'li') return 'listitem';
        if (tag === 'ul' || tag === 'ol') return 'list';
        if (tag === 'nav') return 'navigation';
        if (tag === 'main') return 'main';
        if (tag === 'form') return 'form';
        if (tag === 'dialog') return 'dialog';
        if (tag === 'table') return 'table';
        if (tag === 'tr') return 'row';
        if (tag === 'td') return 'cell';
        if (tag === 'th') return 'columnheader';
        if (tag === 'option') return 'option';
        if (/^h[1-6]$/.test(tag)) return 'heading';
        if (tag === 'input') {
            const tp = (el.type || 'text').toLowerCase();
            if (tp === 'button' || tp === 'submit' || tp === 'reset' || tp === 'image') return 'button';
            if (tp === 'checkbox') return 'checkbox';
            if (tp === 'radio') return 'radio';
            if (tp === 'search') return 'searchbox';
            if (tp === 'number') return 'spinbutton';
            if (tp === 'range') return 'slider';
            if (tp === 'hidden') return '';
            return 'textbox';
        }
        if (el.getAttribute && el.getAttribute('contenteditable') === 'true') return 'textbox';
        return '';
    }
    function __bdAccName(el) {
        const doc = el.ownerDocument || document;
        const al = el.getAttribute && el.getAttribute('aria-label');
        if (al) return al.trim();
        const lb = el.getAttribute && el.getAttribute('aria-labelledby');
        if (lb) {
            const txt = lb.split(/\s+/).map(id => { const r = doc.getElementById(id); return r ? (r.innerText || r.textContent || '').trim() : ''; }).filter(Boolean).join(' ').trim();
            if (txt) return txt;
        }
        if (el.labels && el.labels.length) {
            const lt = Array.from(el.labels).map(l => (l.innerText || l.textContent || '').trim()).filter(Boolean).join(' ').trim();
            if (lt) return lt;
        }
        const tag = (el.tagName || '').toLowerCase();
        if (tag === 'input' && (el.type === 'button' || el.type === 'submit' || el.type === 'reset') && el.value) return String(el.value).trim();
        if (tag === 'img' && el.alt) return el.alt.trim();
        const inner = (el.innerText || el.textContent || '').trim();
        if (inner) return inner;
        return (el.getAttribute && (el.getAttribute('placeholder') || el.getAttribute('title')) || '').trim();
    }
    const wantLow = spec.name !== null ? spec.name.trim().toLowerCase() : null;
    const out = [];
    function walk(root) {
        for (const el of root.querySelectorAll('*')) {
            if (el.shadowRoot) walk(el.shadowRoot);
            if (__bdImplicitRole(el) !== spec.role) continue;
            if (wantLow !== null) {
                const nm = __bdAccName(el).toLowerCase();
                if (spec.exact ? nm !== wantLow : !nm.includes(wantLow)) continue;
            }
            out.push(el);
        }
    }
    walk(this);
    if (this.shadowRoot) walk(this.shadowRoot);
    return spec.all ? out : (out[0] || null);
}"#;

const CHILD_LABEL_FN: &str = r#"function(spec) {
    const out = [];
    const seen = new Set();
    const add = (el) => { if (el && el.nodeType === 1 && !seen.has(el)) { seen.add(el); out.push(el); } };
    const wantLow = spec.label.trim().toLowerCase();
    const matchStr = (s) => { const low = (s || '').trim().toLowerCase(); return spec.exact ? low === wantLow : low.includes(wantLow); };
    function walk(root) {
        for (const el of root.querySelectorAll('*')) {
            if (el.shadowRoot) walk(el.shadowRoot);
            if ((el.tagName || '').toLowerCase() === 'label') {
                if (matchStr(el.innerText || el.textContent || '')) {
                    if (el.control) add(el.control);
                    else {
                        const forId = el.getAttribute('for');
                        const target = forId ? (document.getElementById(forId)) : el.querySelector('input,textarea,select');
                        if (target) add(target);
                    }
                }
            } else {
                const al = el.getAttribute && el.getAttribute('aria-label');
                if (al && matchStr(al)) add(el);
            }
        }
    }
    walk(this);
    if (this.shadowRoot) walk(this.shadowRoot);
    return spec.all ? out : (out[0] || null);
}"#;

const CHILD_TEXT_EXACT_FN: &str = r#"function(spec) {
    const target = spec.text.trim().toLowerCase();
    const list = [];
    let best = null;
    function walk(root) {
        for (const el of root.querySelectorAll('*')) {
            if (el.shadowRoot) walk(el.shadowRoot);
            if (el.getClientRects().length === 0) continue;
            const c = (el.innerText || el.textContent || el.value || el.getAttribute('aria-label') || '').trim().toLowerCase();
            if (spec.exact ? c === target : c.includes(target)) {
                list.push(el);
                if (!best || best.contains(el)) best = el;
            }
        }
    }
    walk(this);
    if (this.shadowRoot) walk(this.shadowRoot);
    return spec.all ? list : best;
}"#;

fn encode_mixed_branches(branches: &[&str]) -> serde_json::Value {
    let items: Vec<serde_json::Value> = branches
        .iter()
        .map(|b| {
            if is_xpath(b) {
                serde_json::json!({ "k": "xpath", "v": *b })
            } else if let Some((txt, _)) = strip_text_prefix(b) {
                serde_json::json!({ "k": "text", "v": txt })
            } else {
                let css = branch_to_css(b).unwrap_or_else(|| (*b).to_string());
                serde_json::json!({ "k": "css", "v": css })
            }
        })
        .collect();
    serde_json::Value::Array(items)
}

/// Returns `(functionDeclaration, argument)` for querying a single child relative to `this` (including `this.shadowRoot`).
pub fn child_sel_fn(query: &str) -> (&'static str, serde_json::Value) {
    let branches = split_locator_branches(query);
    if branches.len() == 1 {
        let s = branches[0];
        if is_xpath(s) {
            return (CHILD_XPATH_ONE_FN, serde_json::Value::String(s.to_string()));
        } else if let Some((role, name, exact)) = parse_role_query(s) {
            return (
                CHILD_ROLE_ONE_FN,
                serde_json::json!({ "role": role, "name": name, "exact": exact, "all": false }),
            );
        } else if let Some((lbl, exact)) = strip_label_prefix(s) {
            return (
                CHILD_LABEL_FN,
                serde_json::json!({ "label": lbl, "exact": exact, "all": false }),
            );
        } else if let Some((txt, exact)) = strip_text_prefix(s) {
            if exact {
                return (
                    CHILD_TEXT_EXACT_FN,
                    serde_json::json!({ "text": txt, "exact": true, "all": false }),
                );
            }
            return (CHILD_TEXT_ONE_FN, serde_json::Value::String(txt.to_string()));
        } else {
            let css = branch_to_css(s).unwrap_or_else(|| s.to_string());
            return (CHILD_CSS_ONE_FN, serde_json::Value::String(css));
        }
    }
    if let Some(css_parts) = branches.iter().map(|b| branch_to_css(b)).collect::<Option<Vec<_>>>() {
        return (CHILD_CSS_ONE_FN, serde_json::Value::String(css_parts.join(", ")));
    }
    (CHILD_MIXED_ONE_FN, encode_mixed_branches(&branches))
}

/// Returns `(functionDeclaration, argument)` for querying all children relative to `this` (including `this.shadowRoot`).
pub fn child_sel_all_fn(query: &str) -> (&'static str, serde_json::Value) {
    let branches = split_locator_branches(query);
    if branches.len() == 1 {
        let s = branches[0];
        if is_xpath(s) {
            return (CHILD_XPATH_ALL_FN, serde_json::Value::String(s.to_string()));
        } else if let Some((role, name, exact)) = parse_role_query(s) {
            return (
                CHILD_ROLE_ONE_FN,
                serde_json::json!({ "role": role, "name": name, "exact": exact, "all": true }),
            );
        } else if let Some((lbl, exact)) = strip_label_prefix(s) {
            return (
                CHILD_LABEL_FN,
                serde_json::json!({ "label": lbl, "exact": exact, "all": true }),
            );
        } else if let Some((txt, exact)) = strip_text_prefix(s) {
            if exact {
                return (
                    CHILD_TEXT_EXACT_FN,
                    serde_json::json!({ "text": txt, "exact": true, "all": true }),
                );
            }
            return (CHILD_TEXT_ALL_FN, serde_json::Value::String(txt.to_string()));
        } else {
            let css = branch_to_css(s).unwrap_or_else(|| s.to_string());
            return (CHILD_CSS_ALL_FN, serde_json::Value::String(css));
        }
    }
    if let Some(css_parts) = branches.iter().map(|b| branch_to_css(b)).collect::<Option<Vec<_>>>() {
        return (CHILD_CSS_ALL_FN, serde_json::Value::String(css_parts.join(", ")));
    }
    (CHILD_MIXED_ALL_FN, encode_mixed_branches(&branches))
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

/// Splits a CSS selector on descendant whitespace while preserving spaces inside `[...]` and quotes.
fn split_css_descendants(css: &str) -> Vec<&str> {
    let bytes = css.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;

    for (i, &b) in bytes.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if (in_single || in_double) && b == b'\\' {
            escape = true;
            continue;
        }
        match b {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'[' if !in_single && !in_double => bracket_depth += 1,
            b']' if !in_single && !in_double => bracket_depth = bracket_depth.saturating_sub(1),
            b'(' if !in_single && !in_double => paren_depth += 1,
            b')' if !in_single && !in_double => paren_depth = paren_depth.saturating_sub(1),
            b' ' | b'\t' | b'\n' | b'\r'
                if !in_single && !in_double && bracket_depth == 0 && paren_depth == 0 =>
            {
                let part = css[start..i].trim();
                if !part.is_empty() && part != ">" {
                    parts.push(part);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let tail = css[start..].trim();
    if !tail.is_empty() && tail != ">" {
        parts.push(tail);
    }
    parts
}

fn find_pierced_single_branch(root: &serde_json::Value, s: &str, out: &mut Vec<i64>) {
    if s.is_empty() || is_xpath(s) {
        return;
    }

    if let Some((txt, exact)) = strip_text_prefix(s) {
        let needle = txt.trim().to_lowercase();
        fn walk_text(node: &serde_json::Value, needle: &str, exact: bool, out: &mut Vec<i64>) -> bool {
            let mut child_matched = false;
            if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
                for c in children {
                    if walk_text(c, needle, exact, out) {
                        child_matched = true;
                    }
                }
            }
            if let Some(shadows) = node.get("shadowRoots").and_then(|v| v.as_array()) {
                for sr in shadows {
                    if walk_text(sr, needle, exact, out) {
                        child_matched = true;
                    }
                }
            }
            if let Some(cd) = node.get("contentDocument") {
                if walk_text(cd, needle, exact, out) {
                    child_matched = true;
                }
            }
            if child_matched {
                return true;
            }
            if node.get("nodeType").and_then(|v| v.as_i64()) == Some(1) {
                let mut text = String::new();
                cdp_collect_text(node, &mut text);
                let low = text.trim().to_lowercase();
                if if exact { low == needle } else { low.contains(needle) } {
                    if let Some(bid) = node.get("backendNodeId").and_then(|v| v.as_i64()) {
                        if !out.contains(&bid) {
                            out.push(bid);
                        }
                        return true;
                    }
                }
            }
            false
        }
        walk_text(root, &needle, exact, out);
        return;
    }

    if let Some((want_role, want_name, exact)) = parse_role_query(s) {
        fn cdp_implicit_role(node: &serde_json::Value) -> &'static str {
            if let Some(r) = cdp_attr(node, "role") {
                match r.trim().to_ascii_lowercase().as_str() {
                    "button" => return "button",
                    "link" => return "link",
                    "textbox" => return "textbox",
                    "checkbox" => return "checkbox",
                    "radio" => return "radio",
                    "combobox" => return "combobox",
                    "heading" => return "heading",
                    "listitem" => return "listitem",
                    "tab" => return "tab",
                    "dialog" => return "dialog",
                    "menuitem" => return "menuitem",
                    "img" => return "img",
                    _ => {}
                }
            }
            let tag = node
                .get("localName")
                .or_else(|| node.get("nodeName"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            match tag.as_str() {
                "button" => "button",
                "a" | "area" if cdp_attr(node, "href").is_some() => "link",
                "textarea" => "textbox",
                "select" => "combobox",
                "img" => "img",
                "li" => "listitem",
                "dialog" => "dialog",
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
                "input" => match cdp_attr(node, "type").unwrap_or("text").to_ascii_lowercase().as_str() {
                    "button" | "submit" | "reset" | "image" => "button",
                    "checkbox" => "checkbox",
                    "radio" => "radio",
                    "search" => "searchbox",
                    "hidden" => "",
                    _ => "textbox",
                },
                _ => "",
            }
        }
        fn walk_role(node: &serde_json::Value, want_role: &str, want_name: Option<&str>, exact: bool, out: &mut Vec<i64>) {
            if node.get("nodeType").and_then(|v| v.as_i64()) == Some(1) {
                let explicit = cdp_attr(node, "role").map(|r| r.trim().to_ascii_lowercase());
                let matched_role = explicit.as_deref() == Some(want_role) || cdp_implicit_role(node) == want_role;
                if matched_role {
                    let name_ok = match want_name {
                        None => true,
                        Some(w) => {
                            let w_low = w.trim().to_lowercase();
                            let mut txt = String::new();
                            cdp_collect_text(node, &mut txt);
                            let cand = cdp_attr(node, "aria-label")
                                .or_else(|| cdp_attr(node, "placeholder"))
                                .or_else(|| cdp_attr(node, "alt"))
                                .or_else(|| cdp_attr(node, "title"))
                                .map(str::to_string)
                                .unwrap_or(txt)
                                .trim()
                                .to_lowercase();
                            if exact { cand == w_low } else { cand.contains(&w_low) }
                        }
                    };
                    if name_ok {
                        if let Some(bid) = node.get("backendNodeId").and_then(|v| v.as_i64()) {
                            if !out.contains(&bid) {
                                out.push(bid);
                            }
                        }
                    }
                }
            }
            if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
                for c in children { walk_role(c, want_role, want_name, exact, out); }
            }
            if let Some(shadows) = node.get("shadowRoots").and_then(|v| v.as_array()) {
                for sr in shadows { walk_role(sr, want_role, want_name, exact, out); }
            }
            if let Some(cd) = node.get("contentDocument") {
                walk_role(cd, want_role, want_name, exact, out);
            }
        }
        walk_role(root, &want_role, want_name.as_deref(), exact, out);
        return;
    }

    if strip_label_prefix(s).is_some() {
        return;
    }

    let css = branch_to_css(s).unwrap_or_else(|| s.to_string());

    // Support whitespace-separated descendant selectors (preserving quoted spaces inside `[...]`)
    let parts = split_css_descendants(&css);
    if parts.is_empty() {
        return;
    }

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
                    if !out.contains(&bid) {
                        out.push(bid);
                    }
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
                        if !out.contains(&bid) {
                            out.push(bid);
                        }
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
    walk_css(root, &parts, &mut ancestors, out);
}

/// Walks a `DOM.getDocument({ depth: -1, pierce: true })` tree (including `#shadow-root (closed)`)
/// and returns matching `backendNodeId`s for CSS, `@attr=val`, `text:`/`t:`, and comma-separated mixed queries.
pub fn find_pierced_backend_node_ids(root: &serde_json::Value, query: &str) -> Vec<i64> {
    let branches = split_locator_branches(query);
    let mut out = Vec::new();
    for b in branches {
        find_pierced_single_branch(root, b, &mut out);
    }
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
    fn split_locator_branches_respects_quotes_brackets_parens_and_text_commas() {
        assert_eq!(
            split_locator_branches("input[type='email'], @placeholder=Email"),
            vec!["input[type='email']", "@placeholder=Email"]
        );
        assert_eq!(
            split_locator_branches("input[placeholder='First, Last'], @name=full"),
            vec!["input[placeholder='First, Last']", "@name=full"]
        );
        assert_eq!(split_locator_branches(":is(.a, .b)"), vec![":is(.a, .b)"]);
        assert_eq!(
            split_locator_branches("text:Hello, world"),
            vec!["text:Hello, world"]
        );
        assert_eq!(
            split_locator_branches("text:Sign In, @id=login"),
            vec!["text:Sign In", "@id=login"]
        );
    }

    #[test]
    fn sel_expr_routes_all_locator_syntaxes() {
        assert!(sel_expr("//button[@id='ok']").contains("document.evaluate"));
        assert!(sel_expr("text:Sign In").contains("innerText"));
        assert!(sel_expr("t:Submit").contains("innerText"));
        assert!(sel_expr("@name=q").contains("[name=\\\"q\\\"]"));
        assert!(sel_expr("#login").contains("queryDeep"));
        // Mixed CSS + @attr merges cleanly into a single CSS selector list
        let mixed_css = sel_expr("input[type='email'], @placeholder=Email");
        assert!(mixed_css.contains("input[type='email'], [placeholder=\\\"Email\\\"]"));
        // Mixed CSS + text: evaluates both branches sequentially
        let mixed_text = sel_expr("#missing, text:Sign In");
        assert!(mixed_text.contains("#missing"));
        assert!(mixed_text.contains("Sign In"));
    }

    #[test]
    fn child_sel_fn_dispatches_correctly() {
        let (f_xp, arg_xp) = child_sel_fn(".//span");
        assert!(f_xp.contains("document.evaluate"));
        assert_eq!(arg_xp, json!(".//span"));

        let (f_txt, arg_txt) = child_sel_fn("text:Hello");
        assert!(f_txt.contains("shadowRoot"));
        assert_eq!(arg_txt, json!("Hello"));

        let (f_attr, arg_attr) = child_sel_fn("@data-id=42");
        assert!(f_attr.contains("queryDeep"));
        assert_eq!(arg_attr, json!("[data-id=\"42\"]"));

        let (f_mix_css, arg_mix_css) = child_sel_fn("input[type='email'], @placeholder=Email");
        assert!(f_mix_css.contains("queryDeep"));
        assert_eq!(arg_mix_css, json!("input[type='email'], [placeholder=\"Email\"]"));

        let (f_mix, arg_mix) = child_sel_fn("#none, text:Submit");
        assert!(f_mix.contains("b.k === 'text'"));
        assert_eq!(arg_mix, json!([{ "k": "css", "v": "#none" }, { "k": "text", "v": "Submit" }]));
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
                                "attributes": ["id", "cf-cb", "type", "checkbox", "placeholder", "Email"]
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
        assert_eq!(
            find_pierced_backend_node_ids(&doc, "input[type='email'], @placeholder=Email"),
            vec![21]
        );
        assert_eq!(
            find_pierced_backend_node_ids(&doc, "#missing, text:Verify you are human"),
            vec![20]
        );
    }

    #[test]
    fn pierced_css_preserves_spaces_inside_attribute_quotes() {
        let dom = json!({
            "nodeType": 9,
            "children": [{
                "nodeType": 1,
                "localName": "div",
                "attributes": ["id", "host"],
                "backendNodeId": 10,
                "shadowRoots": [{
                    "nodeType": 11,
                    "children": [{
                        "nodeType": 1,
                        "localName": "input",
                        "attributes": ["placeholder", "Enter full name", "type", "text"],
                        "backendNodeId": 42
                    }]
                }]
            }]
        });
        assert_eq!(find_pierced_backend_node_ids(&dom, "@placeholder=Enter full name"), vec![42]);
        assert_eq!(
            find_pierced_backend_node_ids(&dom, "div input[placeholder=\"Enter full name\"]"),
            vec![42]
        );
    }
}
