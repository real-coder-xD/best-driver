// src/input.rs — Rust 2021: biological mouse physics (Cubic Bézier, swipe S-curves) and keyboard tables
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Fast nanosecond-derived jitter without pulling in a heavy RNG dependency.
pub fn jitter(base_ms: u64, spread_ms: u64) -> Duration {
    if spread_ms == 0 {
        return Duration::from_millis(base_ms);
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0) as u64;
    Duration::from_millis(base_ms + nanos % spread_ms)
}

/// Generates a biological Cubic Bézier trajectory `(x, y, sleep_ms)` from `(sx, sy)` to `(x, y)`.
pub fn bezier_trajectory(sx: f64, sy: f64, x: f64, y: f64) -> Vec<(f64, f64, u64)> {
    let dist = ((x - sx).powi(2) + (y - sy).powi(2)).sqrt();
    if dist < 1.0 {
        return Vec::new();
    }

    let steps = ((dist / 25.0) as usize).clamp(12, 40);
    let dx = x - sx;
    let dy = y - sy;
    let nx = -dy / dist;
    let ny = dx / dist;

    let arc_magnitude = (dist * 0.15).min(50.0);
    let j = jitter(0, 1000).as_millis() as f64 / 1000.0 - 0.5;
    let ctrl_offset = arc_magnitude * (if j >= 0.0 { 1.0 } else { -1.0 }) * (0.6 + j.abs());

    let p1_x = sx + dx * 0.25 + nx * ctrl_offset * 0.8;
    let p1_y = sy + dy * 0.25 + ny * ctrl_offset * 0.8;
    let p2_x = sx + dx * 0.75 + nx * ctrl_offset * 0.4;
    let p2_y = sy + dy * 0.75 + ny * ctrl_offset * 0.4;

    let mut points = Vec::with_capacity(steps);
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let ease = t * t * (3.0 - 2.0 * t);
        let u = 1.0 - ease;
        let tt = ease * ease;
        let uu = u * u;
        let uuu = uu * u;
        let ttt = tt * ease;

        let mut bx = uuu * sx + 3.0 * uu * ease * p1_x + 3.0 * u * tt * p2_x + ttt * x;
        let mut by = uuu * sy + 3.0 * uu * ease * p1_y + 3.0 * u * tt * p2_y + ttt * y;

        if i < steps {
            let tx = (jitter(0, 1000).as_millis() as f64 / 1000.0 - 0.5) * 0.5;
            let ty = (jitter(0, 1000).as_millis() as f64 / 1000.0 - 0.5) * 0.5;
            bx += tx;
            by += ty;
        }

        let base_sleep = if !(0.2..=0.8).contains(&t) { 8 } else { 5 };
        points.push((bx, by, base_sleep));
    }
    points
}

/// Generates smooth S-curve drag points `(x, y)` for sliding captchas / swipe gestures.
pub fn swipe_trajectory(x1: f64, y1: f64, x2: f64, y2: f64) -> Vec<(f64, f64)> {
    let dist = ((x2 - x1).powi(2) + (y2 - y1).powi(2)).sqrt();
    let steps = ((dist / 15.0) as usize).clamp(15, 60);
    let mut points = Vec::with_capacity(steps);
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let ease = t * t * (3.0 - 2.0 * t);
        let cx = x1 + (x2 - x1) * ease;
        let cy = y1 + (y2 - y1) * ease + (jitter(0, 1000).as_millis() as f64 / 1000.0 - 0.5) * 1.5;
        points.push((cx, cy));
    }
    points
}

/// `(key, code, windowsVirtualKeyCode, text)`
pub fn key_def(key: &str) -> Option<(&'static str, &'static str, i64, &'static str)> {
    Some(match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => ("Enter", "Enter", 13, "\r"),
        "tab" => ("Tab", "Tab", 9, ""),
        "escape" | "esc" => ("Escape", "Escape", 27, ""),
        "backspace" => ("Backspace", "Backspace", 8, ""),
        "delete" | "del" => ("Delete", "Delete", 46, ""),
        "insert" | "ins" => ("Insert", "Insert", 45, ""),
        "space" => (" ", "Space", 32, " "),
        "arrowup" | "up" => ("ArrowUp", "ArrowUp", 38, ""),
        "arrowdown" | "down" => ("ArrowDown", "ArrowDown", 40, ""),
        "arrowleft" | "left" => ("ArrowLeft", "ArrowLeft", 37, ""),
        "arrowright" | "right" => ("ArrowRight", "ArrowRight", 39, ""),
        "home" => ("Home", "Home", 36, ""),
        "end" => ("End", "End", 35, ""),
        "pageup" => ("PageUp", "PageUp", 33, ""),
        "pagedown" => ("PageDown", "PageDown", 34, ""),
        "f1" => ("F1", "F1", 112, ""),
        "f2" => ("F2", "F2", 113, ""),
        "f3" => ("F3", "F3", 114, ""),
        "f4" => ("F4", "F4", 115, ""),
        "f5" => ("F5", "F5", 116, ""),
        "f6" => ("F6", "F6", 117, ""),
        "f7" => ("F7", "F7", 118, ""),
        "f8" => ("F8", "F8", 119, ""),
        "f9" => ("F9", "F9", 120, ""),
        "f10" => ("F10", "F10", 121, ""),
        "f11" => ("F11", "F11", 122, ""),
        "f12" => ("F12", "F12", 123, ""),
        "control" | "ctrl" => ("Control", "ControlLeft", 17, ""),
        "shift" => ("Shift", "ShiftLeft", 16, ""),
        "alt" | "option" => ("Alt", "AltLeft", 18, ""),
        "meta" | "command" | "cmd" => ("Meta", "MetaLeft", 91, ""),
        _ => return None,
    })
}

/// Parsed single key or modifier chord (e.g. `"Enter"`, `"Control+a"`, `"Ctrl+Shift+Z"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyChord {
    /// CDP modifier bitmask: `Alt=1`, `Control=2`, `Meta=4`, `Shift=8`.
    pub modifiers: i64,
    /// Modifier keys to hold before and release after the primary key: `(key, code, vk)`.
    pub modifier_keys: Vec<(&'static str, &'static str, i64)>,
    pub key: String,
    pub code: String,
    pub vk: i64,
    pub text: String,
    /// Native Blink editing commands for cross-platform headless/headful accelerator fidelity.
    pub commands: Vec<&'static str>,
}

fn modifier_entry(token: &str) -> Option<(i64, (&'static str, &'static str, i64))> {
    match token.to_ascii_lowercase().as_str() {
        "alt" | "option" => Some((1, ("Alt", "AltLeft", 18))),
        "control" | "ctrl" => Some((2, ("Control", "ControlLeft", 17))),
        "meta" | "command" | "cmd" => Some((4, ("Meta", "MetaLeft", 91))),
        "shift" => Some((8, ("Shift", "ShiftLeft", 16))),
        _ => None,
    }
}

/// Parses a single key name or `+`-separated shortcut chord (`"Control+a"`, `"Shift+Enter"`, `"Escape"`).
pub fn parse_key_chord(spec: &str) -> Option<KeyChord> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return None;
    }

    let parts: Vec<&str> = if trimmed == "+" {
        vec!["+"]
    } else if let Some(prefix) = trimmed.strip_suffix("++") {
        let mut p: Vec<&str> = prefix.split('+').map(str::trim).filter(|s| !s.is_empty()).collect();
        p.push("+");
        p
    } else {
        trimmed.split('+').map(str::trim).filter(|s| !s.is_empty()).collect()
    };
    if parts.is_empty() {
        return None;
    }

    let mut modifiers = 0i64;
    let mut modifier_keys = Vec::new();
    let main_token = *parts.last()?;

    for &tok in &parts[..parts.len() - 1] {
        let (bit, def) = modifier_entry(tok)?;
        if modifiers & bit == 0 {
            modifiers |= bit;
            modifier_keys.push(def);
        }
    }

    let (key, code, vk, base_text) = if let Some((k, c, vk, t)) = key_def(main_token) {
        (k.to_string(), c.to_string(), vk, t.to_string())
    } else {
        let mut chars = main_token.chars();
        let ch = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        let shift_held = (modifiers & 8) != 0;
        let out_ch = if shift_held && ch.is_ascii_lowercase() {
            ch.to_ascii_uppercase()
        } else {
            ch
        };
        let code = if ch.is_ascii_alphabetic() {
            format!("Key{}", ch.to_ascii_uppercase())
        } else if ch.is_ascii_digit() {
            format!("Digit{}", ch)
        } else {
            String::new()
        };
        let vk = if ch.is_ascii_alphabetic() {
            ch.to_ascii_uppercase() as i64
        } else if ch.is_ascii_digit() {
            ch as i64
        } else {
            0
        };
        let txt = if (modifiers & (1 | 2 | 4)) != 0 {
            String::new()
        } else {
            out_ch.to_string()
        };
        (out_ch.to_string(), code, vk, txt)
    };

    // Suppress printable text when Control/Meta/Alt is held (except Shift+Enter etc.)
    let text = if (modifiers & (1 | 2 | 4)) != 0 {
        String::new()
    } else {
        base_text
    };

    let mut commands = Vec::new();
    let accel = (modifiers & (2 | 4)) != 0 && (modifiers & 1) == 0;
    let shift = (modifiers & 8) != 0;
    if accel {
        match key.to_ascii_lowercase().as_str() {
            "a" if !shift => commands.push("selectAll"),
            "c" if !shift => commands.push("copy"),
            "x" if !shift => commands.push("cut"),
            "v" if !shift => commands.push("paste"),
            "z" if !shift => commands.push("undo"),
            "z" if shift => commands.push("redo"),
            "y" if !shift => commands.push("redo"),
            _ => {}
        }
    }

    Some(KeyChord {
        modifiers,
        modifier_keys,
        key,
        code,
        vk,
        text,
        commands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bezier_lands_exactly_on_target() {
        let pts = bezier_trajectory(10.0, 20.0, 410.0, 320.0);
        assert!(pts.len() >= 12 && pts.len() <= 40);
        let (lx, ly, _) = *pts.last().unwrap();
        assert!((lx - 410.0).abs() < 1e-6);
        assert!((ly - 320.0).abs() < 1e-6);
    }

    #[test]
    fn bezier_skips_tiny_distance() {
        assert!(bezier_trajectory(5.0, 5.0, 5.3, 5.2).is_empty());
    }

    #[test]
    fn key_def_maps_special_keys() {
        assert_eq!(key_def("ENTER").unwrap().2, 13);
        assert_eq!(key_def("esc").unwrap().2, 27);
        assert!(key_def("nonexistent_key").is_none());
    }

    #[test]
    fn parse_key_chord_supports_modifiers_and_commands() {
        let c = parse_key_chord("Control+a").unwrap();
        assert_eq!(c.modifiers, 2);
        assert_eq!(c.code, "KeyA");
        assert_eq!(c.vk, 65);
        assert_eq!(c.commands, vec!["selectAll"]);

        let redo = parse_key_chord("Ctrl+Shift+z").unwrap();
        assert_eq!(redo.modifiers, 10);
        assert_eq!(redo.commands, vec!["redo"]);

        let se = parse_key_chord("Shift+Enter").unwrap();
        assert_eq!(se.modifiers, 8);
        assert_eq!(se.key, "Enter");
        assert_eq!(se.vk, 13);

        assert_eq!(parse_key_chord("F5").unwrap().vk, 116);
        assert!(parse_key_chord("InvalidMod+a").is_none());
    }
}
