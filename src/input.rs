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
        "space" => (" ", "Space", 32, " "),
        "arrowup" | "up" => ("ArrowUp", "ArrowUp", 38, ""),
        "arrowdown" | "down" => ("ArrowDown", "ArrowDown", 40, ""),
        "arrowleft" | "left" => ("ArrowLeft", "ArrowLeft", 37, ""),
        "arrowright" | "right" => ("ArrowRight", "ArrowRight", 39, ""),
        "home" => ("Home", "Home", 36, ""),
        "end" => ("End", "End", 35, ""),
        "pageup" => ("PageUp", "PageUp", 33, ""),
        "pagedown" => ("PageDown", "PageDown", 34, ""),
        _ => return None,
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
}
