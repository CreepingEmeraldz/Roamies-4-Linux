// Linux, niri: where the pointer is, for the Cursor size.
//
// A Wayland client isn't told where the pointer is over other apps (layer.rs), and niri has no IPC for it. The bundled
// helper roamies-pointer (src-tauri/pointer-linux) gets it from a screen capture of each output, which carries the
// pointer as metadata, and prints one JSON line per move: {"x":..,"y":..}, global logical pixels in niri's layout.
// This module runs it, keeps the latest sample, and turns it into an overlay's own CSS pixels.
//
// Only on niri for now: that is the compositor whose capture service the helper speaks (org.gnome.Mutter.ScreenCast)
// and whose IPC gives it the layout. Elsewhere on Wayland the page still gets no pointer samples.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// The newest sample and how many have come (so a reader can tell a new one from the one it already used).
static LATEST: Mutex<(u64, Option<(f64, f64)>)> = Mutex::new((0, None));
static RUNNING: AtomicBool = AtomicBool::new(false);

/// {"x":812.5,"y":340.0} -> (812.5, 340.0)
fn parse(line: &str) -> Option<(f64, f64)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let (x, y) = (v.get("x")?.as_f64()?, v.get("y")?.as_f64()?);
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

/// Whether this session can have pointer samples: niri, and the helper is next to the app.
pub(crate) fn available() -> bool {
    std::env::var_os("NIRI_SOCKET").is_some_and(|v| !v.is_empty()) && super::helper_binary("roamies-pointer").is_some()
}

/// Starts the helper if this session can use it, and keeps it going: when it ends it is started again after a pause,
/// up to a few times in a row (a session without the capture service would fail every time).
pub(crate) fn start() {
    if !available() {
        return;
    }
    std::thread::spawn(|| {
        let mut quick = 0;
        while quick < 3 {
            let Some(bin) = super::helper_binary("roamies-pointer") else { return };
            let began = std::time::Instant::now();
            let child = Command::new(bin).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(super::log_stdio("pointer.log")).spawn();
            let Ok(mut child) = child else { return };
            RUNNING.store(true, Ordering::Relaxed);
            // The stdin pipe stays open as long as `child` does: when Roamies ends, the helper sees it close and ends too.
            let stdin = child.stdin.take();
            if let Some(out) = child.stdout.take() {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if let Some(p) = parse(&line) {
                        let mut l = LATEST.lock().unwrap();
                        *l = (l.0 + 1, Some(p));
                    }
                }
            }
            drop(stdin);
            let _ = child.wait();
            RUNNING.store(false, Ordering::Relaxed);
            *LATEST.lock().unwrap() = (0, None);
            quick = if began.elapsed().as_secs() < 30 { quick + 1 } else { 0 };
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        eprintln!("[pointer] the pointer helper keeps ending: no cursor mode (see ~/.gg/pointer.log)");
    });
}

/// The newest sample if it is newer than `seen` (the count the caller last used): (count, global x, global y).
pub(crate) fn since(seen: u64) -> Option<(u64, f64, f64)> {
    let l = LATEST.lock().unwrap();
    match l.1 {
        Some((x, y)) if l.0 != seen => Some((l.0, x, y)),
        _ => None,
    }
}

/// A pointer in the layout's pixels to the overlay's own CSS pixels: the overlay covers its output, whose top left
/// is `origin` in the layout, and one layout pixel is one CSS pixel.
pub(crate) fn in_overlay(global: (f64, f64), origin: (f64, f64)) -> (f64, f64) {
    (global.0 - origin.0, global.1 - origin.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_from_the_helper_is_a_position() {
        assert_eq!(parse(r#"{"x":812.5,"y":340.0}"#), Some((812.5, 340.0)));
        assert_eq!(parse(r#"{"x":-3,"y":2}"#), Some((-3.0, 2.0)));
        assert_eq!(parse("garbage"), None);
        assert_eq!(parse(r#"{"x":1}"#), None);
    }

    #[test]
    fn the_overlay_of_a_second_screen_is_offset_by_where_the_screen_sits() {
        assert_eq!(in_overlay((1746.0, -280.0), (1736.0, -300.0)), (10.0, 20.0));
        assert_eq!(in_overlay((5.0, 5.0), (0.0, 0.0)), (5.0, 5.0));
    }
}
