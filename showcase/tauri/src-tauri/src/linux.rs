// Linux: which kind of session this is, and the parts of the shell that are GTK, X11 or D-Bus underneath.
//
// Every window is an X11 window, also on Wayland (through XWayland): a Wayland app can't place a window, keep it
// on top or see the cursor over other apps, and the overlay needs all three. So on a Wayland session GTK is told to
// use X11 before it starts, and the code that places windows is the same everywhere. What Wayland still takes away
// (the cursor over other apps, global shortcuts, grabbing the screen) is handled per feature below.
//
// The exception is a Wayland compositor with layer-shell (niri, sway, Hyprland, ...): there GTK stays on Wayland and
// the overlay is a layer surface (layer.rs), because an XWayland window can't be kept on top there. Only the
// overlay's placement changes: click areas, screenshots, the clipboard and the rest are the Wayland ones below.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::layer;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Session {
    X11,
    Wayland,
}

impl Session {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Session::X11 => "x11",
            Session::Wayland => "wayland",
        }
    }
}

/// Wayland when the session says so or a Wayland display is set (XWayland's DISPLAY is set there too, so it can't
/// decide). The voice helper uses the same rule (voice-win/src/keys.rs).
fn session_from(session_type: Option<&str>, wayland_display: Option<&str>) -> Session {
    let wayland = session_type.is_some_and(|s| s.eq_ignore_ascii_case("wayland")) || wayland_display.is_some_and(|s| !s.is_empty());
    if wayland { Session::Wayland } else { Session::X11 }
}

static SESSION: OnceLock<Session> = OnceLock::new();
static LAYER: OnceLock<bool> = OnceLock::new();

pub(crate) fn session() -> Session {
    *SESSION.get().unwrap_or(&Session::X11)
}

/// Whether GTK runs on Wayland itself and the overlay is a layer surface (layer.rs), not an XWayland window.
pub(crate) fn native_wayland() -> bool {
    *LAYER.get().unwrap_or(&false)
}

/// First thing in main, before GTK starts and before any other thread exists.
pub(crate) fn init() {
    let env = |k: &str| std::env::var(k).ok();
    let s = session_from(env("XDG_SESSION_TYPE").as_deref(), env("WAYLAND_DISPLAY").as_deref());
    let _ = SESSION.set(s);
    // A GDK_BACKEND the user set to x11 is a choice for XWayland, and layer-shell can't be had there.
    let native = s == Session::Wayland && layer::wanted() && env("GDK_BACKEND").as_deref().map_or(true, |b| b.contains("wayland"));
    if native {
        std::env::set_var("GDK_BACKEND", "wayland");
        // The compositor might not have the protocol after all (ROAMIES_LAYER=1 on GNOME). GDK can't change backend
        // once it has started, so this program starts again on XWayland, as it would have without layer-shell.
        if gtk::init().is_ok() && !layer::supported() {
            eprintln!("This compositor has no layer-shell: starting again on XWayland");
            use std::os::unix::process::CommandExt;
            let err = std::process::Command::new(std::env::current_exe().unwrap_or_else(|_| "roamies".into()))
                .args(std::env::args_os().skip(1))
                .env("ROAMIES_LAYER", "0")
                .env_remove("GDK_BACKEND")
                .exec();
            eprintln!("couldn't start again: {err}");
        }
        let _ = LAYER.set(true);
        return;
    }
    if s == Session::Wayland {
        if env("DISPLAY").is_some_and(|d| !d.is_empty()) {
            std::env::set_var("GDK_BACKEND", "x11");
        } else {
            eprintln!("Roamies needs XWayland on a Wayland session (no DISPLAY is set): windows can't be placed or kept on top without it");
        }
    }
}

fn trace() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ROAMIES_TRACE").is_some())
}

// ---------- click-through by region ----------

/// The page's click areas ([x, y, w, h, …] in CSS px) as whole-pixel rectangles: bad numbers and empty ones are
/// dropped, and a runaway list is cut off (a shape with thousands of pieces helps nobody).
fn hit_rects(flat: &[f64]) -> Vec<(i32, i32, i32, i32)> {
    const MAX: usize = 600;
    const LIMIT: f64 = 100_000.0;
    flat.chunks_exact(4)
        .filter(|r| r.iter().all(|v| v.is_finite() && v.abs() < LIMIT) && r[2] >= 1.0 && r[3] >= 1.0)
        .map(|r| (r[0].floor() as i32, r[1].floor() as i32, r[2].ceil() as i32, r[3].ceil() as i32))
        .take(MAX)
        .collect()
}

/// Makes the rectangles the overlay's input shape: clicks land on them and pass through everywhere else. No cursor
/// tracking is involved, so it works on Wayland too, where the cursor can't be seen over other apps.
pub(crate) fn set_hit_rects(window: &tauri::WebviewWindow, flat: &[f64]) {
    use gtk::cairo::{RectangleInt, Region};
    use gtk::prelude::WidgetExt;
    let rects = hit_rects(flat);
    // ROAMIES_TRACE=1: the shape goes to stderr, for checking click-through from outside (and for bug reports).
    if trace() {
        eprintln!("hit rects: {rects:?}");
    }
    let w = window.clone();
    let _ = window.run_on_main_thread(move || {
        let Some(win) = w.gtk_window().ok().and_then(|g| g.window()) else { return };
        // GDK works in the same logical pixels as the page.
        let list: Vec<RectangleInt> = rects.iter().map(|&(x, y, w, h)| RectangleInt::new(x, y, w, h)).collect();
        let region = Region::create_rectangles(&list);
        win.input_shape_combine_region(&region, 0, 0);
        // No compositor (a bare X11 window manager): the transparent parts would draw black over the whole screen,
        // so the window is cut down to the same rectangles.
        if !win.screen().is_composited() {
            win.shape_combine_region(Some(&region), 0, 0);
        }
    });
}

// ---------- COSMIC: an overlay no window manager touches ----------

/// COSMIC files every X11 window it manages with the apps: it ignores "keep above" (_NET_WM_STATE_ABOVE), so app
/// windows cover the overlay, and it rounds and tints it like an app. It draws override-redirect windows (ones no
/// window manager manages, like menus) above every app window, untouched. ROAMIES_UNMANAGED=1 or 0 forces it.
fn unmanaged_for(desktop: Option<&str>, force: Option<&str>) -> bool {
    match force {
        Some("1") => true,
        Some("0") => false,
        _ => desktop.is_some_and(|d| d.split(':').any(|p| p.eq_ignore_ascii_case("cosmic"))),
    }
}

pub(crate) fn unmanaged_overlay() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let env = |k: &str| std::env::var(k).ok();
        unmanaged_for(env("XDG_CURRENT_DESKTOP").as_deref(), env("ROAMIES_UNMANAGED").as_deref())
    })
}

/// Makes the overlay override-redirect (on) or an ordinary window again, and shows it. The flag is read when the
/// window is mapped, so it's taken down and put back up around the change. Only a managed window gets the keyboard
/// on COSMIC, so typing turns it back into one for the while: `focus` then asks for the keyboard once it's up.
pub(crate) fn set_unmanaged(window: &tauri::WebviewWindow, on: bool, focus: bool) {
    let w = window.clone();
    let _ = window.run_on_main_thread(move || {
        use gtk::prelude::{GtkWindowExt, WidgetExt};
        let Ok(gw) = w.gtk_window() else { return };
        gw.realize();
        let Some(win) = gw.window() else { return };
        if trace() {
            eprintln!("overlay unmanaged: {on}");
        }
        let (x, y) = gw.position();
        if gw.is_visible() {
            gw.hide();
        }
        win.set_override_redirect(on);
        gw.move_(x, y);
        gw.show_all();
        if focus {
            gw.present();
            activate(&w);
        }
    });
}

// ---------- keyboard focus ----------

/// _NET_ACTIVE_WINDOW's data for a request from a pager or taskbar (source 2), with no timestamp. GTK sends one
/// from the app (source 1), which KWin's focus-stealing prevention turns down once another app has had a keypress,
/// such as the shortcut that ran `roamies --ask`: the field shows but its keys go to the other app.
fn activation_request() -> [u32; 5] {
    [2, 0, 0, 0, 0]
}

/// Ask the window manager to give the overlay the keyboard, as a taskbar click would. Wayland sessions too: the
/// window is XWayland's.
pub(crate) fn activate(window: &tauri::WebviewWindow) {
    use gtk::glib::prelude::ObjectExt;
    use gtk::glib::translate::ToGlibPtr;
    use gtk::prelude::WidgetExt;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ClientMessageEvent, ConnectionExt, EventMask};
    extern "C" {
        fn gdk_x11_window_get_xid(window: *mut gtk::gdk::ffi::GdkWindow) -> std::os::raw::c_ulong;
    }
    static CONN: OnceLock<Option<(x11rb::rust_connection::RustConnection, u32, u32)>> = OnceLock::new();
    let Some(win) = window.gtk_window().ok().and_then(|g| g.window()) else { return };
    if win.display().type_().name() != "GdkX11Display" {
        return;
    }
    // SAFETY: an X11 GdkWindow (checked above), alive for the call.
    let xid = unsafe { gdk_x11_window_get_xid(win.to_glib_none().0) };
    let Some((conn, root, atom)) = CONN
        .get_or_init(|| {
            let (conn, screen) = x11rb::connect(None).ok()?;
            let root = conn.setup().roots.get(screen)?.root;
            let atom = conn.intern_atom(false, b"_NET_ACTIVE_WINDOW").ok()?.reply().ok()?.atom;
            Some((conn, root, atom))
        })
        .as_ref()
    else {
        return;
    };
    // GTK's connection must have told the server the window now takes focus before ours asks for it.
    if let Some(d) = gtk::gdk::Display::default() {
        d.sync();
    }
    let ev = ClientMessageEvent::new(32, xid as u32, *atom, activation_request());
    let _ = conn.send_event(false, *root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, ev);
    let _ = conn.flush();
}

// ---------- the pointer (X11 sessions) ----------

/// One X connection for the pointer and the keys: its root window and the keycodes of the modifier keys. None on
/// Wayland, where neither can be seen over other apps (the sidecar asks for no click waits and no tap shortcut there).
struct X {
    conn: x11rb::rust_connection::RustConnection,
    root: u32,
    modifiers: Vec<u8>,
}

fn x() -> Option<&'static X> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;
    static X11: OnceLock<Option<X>> = OnceLock::new();
    if session() != Session::X11 {
        return None;
    }
    X11.get_or_init(|| {
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        let modifiers = conn.get_modifier_mapping().ok().and_then(|c| c.reply().ok()).map(|r| r.keycodes).unwrap_or_default();
        Some(X { conn, root, modifiers })
    })
    .as_ref()
}

/// The pointer's button and modifier state, whichever window has the keyboard (0 when it can't be read).
fn pointer_mask() -> u16 {
    use x11rb::protocol::xproto::ConnectionExt;
    x().and_then(|x| x.conn.query_pointer(x.root).ok()?.reply().ok()).map_or(0, |r| u16::from(r.mask))
}

/// Whether the left mouse button is down right now, anywhere on screen: a drawing waiting for a click polls it.
/// X11 sessions only. On Wayland the pointer can't be seen over other apps, and the sidecar asks for no such waits.
pub(crate) fn left_button_down() -> bool {
    // Button1 in the pointer's button and modifier state.
    pointer_mask() & 0x100 != 0
}

/// X11's modifier state (Shift 1, Control 4, Mod1 = Alt 8, Mod4 = Super 64) as tap.rs bits. Mod5 (AltGr) is a
/// modifier no pair uses; Lock and Num Lock (Mod2) are states, not keys held.
fn mods_from_x11(mask: u16) -> u8 {
    use crate::tap::{COMMAND, CONTROL, OPTION, OTHER_MOD, SHIFT};
    [(0x04, CONTROL), (0x08, OPTION), (0x01, SHIFT), (0x40, COMMAND), (0x80, OTHER_MOD)]
        .iter()
        .filter(|(bit, _)| mask & bit != 0)
        .fold(0, |m, (_, b)| m | b)
}

/// Any key held that isn't a modifier: X11's key bitmap (one bit per keycode), less the keycodes in its modifier map.
fn other_key(keymap: &[u8], modifiers: &[u8]) -> bool {
    (8..keymap.len() * 8).any(|k| keymap[k / 8] & (1 << (k % 8)) != 0 && !modifiers.contains(&(k as u8)))
}

/// For the type-to-ask tap (tap.rs): the modifiers held, and whether a mouse button is down or, when `keys` is
/// set (one more round trip, so only while a tap is under way), any other key. X11 sessions only.
pub(crate) fn keys_state(keys: bool) -> (u8, bool) {
    use x11rb::protocol::xproto::ConnectionExt;
    let mask = pointer_mask();
    // Buttons 1 to 5 are bits 8 to 12 of the state.
    let other = mask & 0x1f00 != 0
        || (keys && x().is_some_and(|x| x.conn.query_keymap().ok().and_then(|c| c.reply().ok()).is_some_and(|r| other_key(&r.keys, &x.modifiers))));
    (mods_from_x11(mask), other)
}

// ---------- screenshots ----------

/// Claude's own cap on a picture's long side: a bigger one is shrunk again on its way and its coordinates drift.
const SHOT_MAX: i32 = 1568;

/// A picture's size shrunk to fit `max` on its long side (never enlarged).
fn fit(w: i32, h: i32, max: i32) -> (i32, i32) {
    let long = w.max(h);
    if long <= max || long <= 0 {
        return (w, h);
    }
    let k = max as f64 / long as f64;
    (((w as f64 * k).round() as i32).max(1), ((h as f64 * k).round() as i32).max(1))
}

/// The rectangle round every screen, from each one's [x, y, w, h].
fn union_bounds(screens: &[[f64; 4]]) -> [f64; 4] {
    let Some(first) = screens.first() else { return [0.0; 4] };
    let (mut x0, mut y0, mut x1, mut y1) = (first[0], first[1], first[0] + first[2], first[1] + first[3]);
    for s in &screens[1..] {
        x0 = x0.min(s[0]);
        y0 = y0.min(s[1]);
        x1 = x1.max(s[0] + s[2]);
        y1 = y1.max(s[1] + s[3]);
    }
    [x0, y0, x1 - x0, y1 - y0]
}

/// The whole desktop in the overlay's points (the units of safe_js's origin): what a screenshot covers, so the
/// sidecar can turn a pixel of the picture into a place on screen.
fn desktop_bounds(app: &AppHandle) -> [f64; 4] {
    let screens: Vec<[f64; 4]> = app
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let s = m.scale_factor();
            [m.position().x as f64 / s, m.position().y as f64 / s, m.size().width as f64 / s, m.size().height as f64 / s]
        })
        .collect();
    union_bounds(&screens)
}

/// Where the desktop portal will answer a request made with this token: the caller's bus name (":1.42") without
/// its colon and with dots as underscores, then the token.
fn portal_request_path(unique_name: &str, token: &str) -> String {
    format!("/org/freedesktop/portal/desktop/request/{}/{token}", unique_name.trim_start_matches(':').replace('.', "_"))
}

/// The file a file:// link names (the portal answers with one), percent-escapes undone. None for anything else.
fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // An empty host or "localhost", then an absolute path.
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    if !path.starts_with('/') {
        return None;
    }
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(a), Some(b)) => {
                out.push((a * 16 + b) as u8);
                i += 3;
            }
            (c, _, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok().map(PathBuf::from)
}

/// Shrinks the picture to Claude's cap and writes it as a JPEG. Returns its final size in pixels.
fn save_fitted(shot: &gtk::gdk_pixbuf::Pixbuf, file: &str) -> Result<(i32, i32), String> {
    let (w, h) = fit(shot.width(), shot.height(), SHOT_MAX);
    let out = if (w, h) == (shot.width(), shot.height()) { Some(shot.clone()) } else { shot.scale_simple(w, h, gtk::gdk_pixbuf::InterpType::Bilinear) };
    let out = out.ok_or("the screenshot couldn't be resized")?;
    out.savev(file, "jpeg", &[("quality", "85")]).map_err(|e| e.to_string())?;
    Ok((w, h))
}

/// X11: the root window holds the whole screen as the user sees it. Main thread only.
fn grab_root() -> Result<gtk::gdk_pixbuf::Pixbuf, String> {
    use gtk::gdk::prelude::WindowExtManual;
    let root = gtk::gdk::Window::default_root_window();
    root.pixbuf(0, 0, root.width(), root.height()).ok_or_else(|| "the X server didn't hand over the screen".to_string())
}

/// Wayland: only the desktop's own portal may read the screen. It writes a picture and answers with a link to
/// it; some desktops ask the user first, each time or once.
fn portal_screenshot() -> Result<PathBuf, String> {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value as V};
    static N: AtomicU32 = AtomicU32::new(0);
    const NO_PORTAL: &str = "This desktop has no screenshot portal (install xdg-desktop-portal and the one for your desktop, e.g. xdg-desktop-portal-gnome, -kde or -wlr).";
    let err = |e: zbus::Error| e.to_string();
    let conn = Connection::session().map_err(err)?;
    let token = format!("roamies_{}_{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed));
    let me = conn.unique_name().map(|n| n.to_string()).ok_or("no session bus name")?;
    // Listen for the answer before asking, so a quick one isn't missed.
    let request = Proxy::new(&conn, "org.freedesktop.portal.Desktop", portal_request_path(&me, &token), "org.freedesktop.portal.Request").map_err(err)?;
    let mut answers = request.receive_signal("Response").map_err(err)?;
    let portal = Proxy::new(&conn, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Screenshot").map_err(err)?;
    let mut options: HashMap<&str, V> = HashMap::new();
    options.insert("handle_token", V::from(token.as_str()));
    options.insert("interactive", V::from(false));
    options.insert("modal", V::from(false));
    let _handle: OwnedObjectPath = portal.call("Screenshot", &("", options)).map_err(|e| match e {
        zbus::Error::MethodError(..) | zbus::Error::InterfaceNotFound => NO_PORTAL.to_string(),
        e => format!("{NO_PORTAL} ({e})"),
    })?;
    // The answer can take as long as the user takes with the desktop's "allow this?" dialog.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(answers.next());
    });
    let msg = rx.recv_timeout(Duration::from_secs(60)).map_err(|_| "The desktop didn't answer the screenshot request.".to_string())?.ok_or("The screenshot request was dropped.")?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = msg.body().deserialize().map_err(err)?;
    if code != 0 {
        return Err(if code == 1 { "The screenshot wasn't allowed (the desktop asked, and the answer was no).".into() } else { "The desktop couldn't take the screenshot.".into() });
    }
    let uri = results.get("uri").and_then(|v| v.downcast_ref::<&str>().ok()).ok_or("the portal sent no picture")?;
    file_uri_path(uri).ok_or_else(|| "the portal sent a link that isn't a file".to_string())
}

/// Overlays go clear for a moment so the Roamie isn't in its own screenshot (macOS and Windows leave them out of
/// captures instead). Main thread only.
fn fade_overlays(app: &AppHandle, opacity: f64) {
    use gtk::prelude::WidgetExt;
    for (label, w) in app.webview_windows() {
        if crate::is_overlay(&label) {
            if let Ok(g) = w.gtk_window() {
                g.set_opacity(opacity);
            }
        }
    }
}

/// The sidecar's take_screenshot: the whole desktop into `file` (a JPEG, shrunk to Claude's cap). Replies
/// screenshot_done with the picture's size in pixels and the part of the desktop it covers (in the overlay's
/// points), or with what went wrong. Main thread only.
pub(crate) fn screenshot(app: &AppHandle, id: Value, file: String) {
    let done = |app: &AppHandle, id: &Value, r: Result<(i32, i32), String>, ms: Value| {
        let msg = match r {
            Ok((w, h)) => json!({ "type": "screenshot_done", "id": id, "ok": true, "pixels": [w, h], "bounds": desktop_bounds(app), "ms": ms }),
            Err(e) => json!({ "type": "screenshot_done", "id": id, "ok": false, "error": e }),
        };
        crate::to_sidecar(app, &msg);
    };
    // Long enough for the compositor to draw a frame without the overlays (a few frames at 60 Hz).
    const CLEAR: Duration = Duration::from_millis(80);
    fade_overlays(app, 0.0);
    let app = app.clone();
    if session() == Session::X11 {
        gtk::glib::timeout_add_local_once(CLEAR, move || {
            let shot = grab_root();
            fade_overlays(&app, 1.0);
            done(&app, &id, shot.and_then(|p| save_fitted(&p, &file)), Value::Null);
        });
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(CLEAR);
        let t0 = std::time::Instant::now();
        let shot = portal_screenshot();
        let t_portal = t0.elapsed();
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || fade_overlays(&app2, 1.0));
        let mut ms = json!({ "portal": t_portal.as_millis() as u64 });
        let r = shot.and_then(|path| {
            let t1 = std::time::Instant::now();
            let loaded = gtk::gdk_pixbuf::Pixbuf::from_file(&path).map_err(|e| e.to_string());
            let t_load = t1.elapsed();
            // The full-size picture stays next to the small one (file.full.png) for reading words off it (the sidecar's OCR);
            // the sidecar deletes it. Otherwise the portal's copy goes: it keeps one (in Pictures, on most desktops).
            let full = format!("{file}.full.png");
            let _ = std::fs::rename(&path, &full).or_else(|_| std::fs::copy(&path, &full).map(|_| ()));
            remove_portal_file(&path);
            let t2 = std::time::Instant::now();
            let saved = loaded.and_then(|p| save_fitted(&p, &file));
            // Where the time goes, for anyone timing a slow screenshot (the log has it as screenshot_time).
            ms = json!({ "portal": t_portal.as_millis() as u64, "load": t_load.as_millis() as u64, "save": t2.elapsed().as_millis() as u64 });
            saved
        });
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || done(&app2, &id, r, ms));
    });
}

// ---------- area select ----------
// `roamies --snap` (the sidecar's areaSnap): the screen is captured as for a screenshot, the overlays show it frozen and
// dimmed (host.js startPick), the user drags a box, and the part under it is cropped from the full-size picture into
// `file`. One pick at a time.
struct Pick {
    id: Value,
    full: PathBuf,
    bounds: [f64; 4],
    file: String,
}
static PICK: std::sync::Mutex<Option<Pick>> = std::sync::Mutex::new(None);
/// Which `select` is the newest: a picture that arrives for an older one (it was replaced while the desktop was taking it) is dropped.
static PICK_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Closes a picker that is up (or being set up) and tells the sidecar it was cancelled, so nothing stays waiting on it.
fn cancel_pick(app: &AppHandle) {
    let Some(old) = PICK.lock().unwrap().take() else { return };
    crate::to_overlays(app, &json!({ "kind": "pick_end" }));
    let _ = std::fs::remove_file(&old.full);
    crate::to_sidecar(app, &json!({ "type": "select_done", "id": old.id, "ok": false, "cancelled": true }));
}

pub(crate) fn select(app: &AppHandle, id: Value, file: String) {
    let fail = |app: &AppHandle, id: &Value, e: String| crate::to_sidecar(app, &json!({ "type": "select_done", "id": id, "ok": false, "error": e }));
    if trace() {
        eprintln!("area select: asked (a picker is up: {})", PICK.lock().unwrap().is_some());
    }
    // Asking again while one is open replaces it, so a picker that got stuck can't block the next one.
    cancel_pick(app);
    let gen = PICK_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    const CLEAR: Duration = Duration::from_millis(80);
    fade_overlays(app, 0.0);
    let app = app.clone();
    if session() == Session::X11 {
        gtk::glib::timeout_add_local_once(CLEAR, move || {
            let shot = grab_root();
            fade_overlays(&app, 1.0);
            if let Err(e) = shot.and_then(|p| begin_pick(&app, &id, file, gen, &p, None)) {
                fail(&app, &id, e);
            }
        });
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(CLEAR);
        let shot = portal_screenshot();
        if trace() {
            eprintln!("area select: portal answered: {:?}", shot.as_ref().map(|p| p.display().to_string()));
        }
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || fade_overlays(&app2, 1.0));
        let r = shot.and_then(|path| {
            let loaded = gtk::gdk_pixbuf::Pixbuf::from_file(&path).map_err(|e| e.to_string());
            // begin_pick moves the portal's own file into place (no second PNG to write); whatever is left of it goes now.
            let r = loaded.and_then(|p| begin_pick(&app, &id, file, gen, &p, Some(&path)));
            remove_portal_file(&path);
            r
        });
        if let Err(e) = r {
            let app2 = app.clone();
            let _ = app.run_on_main_thread(move || fail(&app2, &id, e));
        }
    });
}

/// Keeps the full-size picture on disk for the crop, and shows the overlays a screen-sized copy to drag over.
fn begin_pick(app: &AppHandle, id: &Value, file: String, gen: u64, shot: &gtk::gdk_pixbuf::Pixbuf, png: Option<&Path>) -> Result<(), String> {
    use base64::Engine;
    if PICK_GEN.load(std::sync::atomic::Ordering::SeqCst) != gen {
        // A newer request replaced this one while the picture was being taken.
        crate::to_sidecar(app, &json!({ "type": "select_done", "id": id, "ok": false, "cancelled": true }));
        return Ok(());
    }
    let t0 = std::time::Instant::now();
    let bounds = desktop_bounds(app);
    let full = std::env::temp_dir().join(format!("roamies-pick-{}.png", std::process::id()));
    // The desktop already wrote the full-size picture as a PNG: keep that file (moving it is instant) instead of encoding another.
    let moved = png.is_some_and(|p| std::fs::rename(p, &full).is_ok() || (std::fs::copy(p, &full).is_ok() && std::fs::remove_file(p).is_ok()));
    if !moved {
        shot.savev(&full, "png", &[]).map_err(|e| e.to_string())?;
    }
    let t_keep = t0.elapsed();
    let (w, h) = (bounds[2].round().max(1.0) as i32, bounds[3].round().max(1.0) as i32);
    let preview = shot.scale_simple(w, h, gtk::gdk_pixbuf::InterpType::Bilinear).ok_or("the screenshot couldn't be resized")?;
    let jpeg = preview.save_to_bufferv("jpeg", &[("quality", "80")]).map_err(|e| e.to_string())?;
    let image = format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(jpeg));
    if trace() {
        eprintln!("area select: keep picture {} ms, preview {} ms ({} KB)", t_keep.as_millis(), (t0.elapsed() - t_keep).as_millis(), image.len() / 1024);
    }
    *PICK.lock().unwrap() = Some(Pick { id: id.clone(), full, bounds, file });
    let (app2, bounds) = (app.clone(), bounds);
    let _ = app.run_on_main_thread(move || crate::to_overlays(&app2, &json!({ "kind": "pick", "image": image, "bounds": bounds })));
    Ok(())
}

/// The page's answer: the box in desktop points, or None (cancelled).
pub(crate) fn select_done(app: &AppHandle, rect: Option<Vec<f64>>) {
    let Some(pick) = PICK.lock().unwrap().take() else { return };
    crate::to_overlays(app, &json!({ "kind": "pick_end" }));
    let reply = |app: &AppHandle, body: Value| {
        let mut msg = json!({ "type": "select_done", "id": pick.id });
        msg.as_object_mut().unwrap().extend(body.as_object().unwrap().clone());
        crate::to_sidecar(app, &msg);
    };
    let Some([x, y, w, h]) = rect.and_then(|r| <[f64; 4]>::try_from(r).ok()).filter(|r| r.iter().all(|v| v.is_finite())) else {
        let _ = std::fs::remove_file(&pick.full);
        reply(app, json!({ "ok": false, "cancelled": true }));
        return;
    };
    let app = app.clone();
    std::thread::spawn(move || {
        let r = crop_pick(&pick, [x, y, w, h]);
        let _ = std::fs::remove_file(&pick.full);
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || {
            let mut msg = json!({ "type": "select_done", "id": pick.id });
            match r {
                Ok(()) => msg["ok"] = json!(true),
                Err(e) => {
                    msg["ok"] = json!(false);
                    msg["error"] = json!(e);
                }
            }
            crate::to_sidecar(&app2, &msg);
        });
    });
}

/// The box (desktop points) cut out of the full-size picture, which covers the whole desktop.
fn crop_pick(pick: &Pick, [x, y, w, h]: [f64; 4]) -> Result<(), String> {
    let shot = gtk::gdk_pixbuf::Pixbuf::from_file(&pick.full).map_err(|e| e.to_string())?;
    let [bx, by, bw, bh] = pick.bounds;
    let (sx, sy) = (shot.width() as f64 / bw.max(1.0), shot.height() as f64 / bh.max(1.0));
    let left = (((x - bx) * sx).round() as i32).clamp(0, shot.width() - 1);
    let top = (((y - by) * sy).round() as i32).clamp(0, shot.height() - 1);
    let width = ((w * sx).round() as i32).clamp(1, shot.width() - left);
    let height = ((h * sy).round() as i32).clamp(1, shot.height() - top);
    let part = shot.new_subpixbuf(left, top, width, height);
    part.savev(&pick.file, "png", &[]).map_err(|e| e.to_string())
}

fn remove_portal_file(path: &Path) {
    if path.is_file() {
        let _ = std::fs::remove_file(path);
    }
}

// ---------- clipboard ----------

/// Copies text through GTK (no xclip or wl-copy needed; XWayland hands it on to Wayland apps). Main thread only.
pub(crate) fn set_clipboard(text: &str) {
    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
    clipboard.set_text(text);
    clipboard.store();
}

// ---------- Open at Login, updates ----------

/// The AppImage this copy runs from (the AppImage runtime sets APPIMAGE to the file), if it is one.
pub(crate) fn appimage() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE").map(PathBuf::from).filter(|p| p.is_file())
}

/// A program path as one quoted argument of a desktop entry's Exec line. Inside the quotes, `"`, `` ` ``, `$` and
/// `\` take a backslash, and every backslash is doubled again because Exec is a string value; `%` starts a field
/// code, so it's doubled too. None for a path a desktop entry can't hold (a line break).
fn exec_quoted(path: &str) -> Option<String> {
    if path.is_empty() || path.contains(['\n', '\r']) {
        return None;
    }
    let mut out = String::from("\"");
    for c in path.chars() {
        match c {
            '"' | '`' | '$' => {
                out.push_str("\\\\");
                out.push(c);
            }
            '\\' => out.push_str("\\\\\\\\"),
            '%' => out.push_str("%%"),
            c => out.push(c),
        }
    }
    out.push('"');
    Some(out)
}

/// The XDG autostart entry that starts Roamies when the user logs in.
fn autostart_entry(program: &str) -> Option<String> {
    let exec = exec_quoted(program)?;
    Some(format!("[Desktop Entry]\nType=Application\nName=Roamies\nComment=AI desktop companions\nExec={exec}\nIcon=roamies\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"))
}

/// Whether an autostart entry is switched on: desktops' own "startup apps" settings switch one off by adding
/// Hidden=true (or GNOME's own key) rather than deleting it.
fn autostart_on(entry: &str) -> bool {
    !entry.lines().map(|l| l.trim().to_ascii_lowercase().replace(' ', "")).any(|l| l == "hidden=true" || l == "x-gnome-autostart-enabled=false")
}

fn autostart_file() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("autostart").join("roamies.desktop"))
}

/// Open at Login: an entry in ~/.config/autostart, which GNOME, KDE and the other XDG desktops all run. It starts
/// the AppImage file when this is one (its inner binary lives in a mount that's gone after it quits), else this
/// binary. `set`: None just reports. Returns (supported, on).
pub(crate) fn login_item(set: Option<bool>, installed: bool) -> (bool, bool) {
    let program = appimage().or_else(|| std::env::current_exe().ok());
    let (Some(file), Some(entry)) = (autostart_file(), program.and_then(|p| autostart_entry(&p.to_string_lossy()))) else { return (false, false) };
    if !installed {
        return (false, false);
    }
    match set {
        Some(true) => {
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&file, entry) {
                eprintln!("open at login: {e}");
            }
        }
        Some(false) => {
            let _ = std::fs::remove_file(&file);
        }
        None => {}
    }
    (true, std::fs::read_to_string(&file).is_ok_and(|e| autostart_on(&e)))
}

/// The program as a shell word, for commands people bind to keys (`<this> --talk`): plain `roamies` when that's
/// what the package installed on PATH, else the AppImage or binary's full path, single-quoted if it needs it.
fn launch_word(program: &str) -> String {
    if program == "/usr/bin/roamies" || program == "/usr/local/bin/roamies" {
        return "roamies".into();
    }
    if program.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c)) {
        return program.into();
    }
    format!("'{}'", program.replace('\'', "'\\''"))
}

/// Where the app's own D-Bus address is: the single-instance plugin serves `<identifier>.SingleInstance` at that
/// name as a path. The identifier is checked against tauri.conf.json by a test.
const BUS_ID: &str = "com.kenkai.roamies";

/// The text of the shortcut helper: `roamies-ctl --summon` (and the other actions) sends the request to the running
/// app over D-Bus, a few milliseconds, instead of starting a second copy of the app to say it, which takes far longer
/// (most of all from an AppImage, which has to mount itself first). When the app isn't running, or there is no gdbus,
/// it starts the app as the plain command would.
fn ctl_script(program_word: &str) -> String {
    let path = format!("/{}/SingleInstance", BUS_ID.replace('.', "/"));
    format!(
        "#!/bin/sh\n# Written by Roamies each time it starts; changes here are overwritten.\ncase \"$1\" in\n  --summon|--list|--talk|--ask|--panel|--snap) ;;\n  *) exec {program_word} \"$@\" ;;\nesac\ngdbus call --session --dest {BUS_ID}.SingleInstance --object-path {path} --method org.SingleInstance.DBus.ExecuteCallback \"['roamies', '$1']\" / >/dev/null 2>&1 || exec {program_word} \"$@\"\n"
    )
}

/// Writes the helper to ~/.local/bin/roamies-ctl when it is missing or out of date, and returns its path.
fn write_ctl(program_word: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let dir = PathBuf::from(std::env::var_os("HOME")?).join(".local/bin");
    let file = dir.join("roamies-ctl");
    let script = ctl_script(program_word);
    if std::fs::read_to_string(&file).is_ok_and(|have| have == script) {
        return Some(file);
    }
    std::fs::create_dir_all(&dir).ok()?;
    let tmp = dir.join(format!(".roamies-ctl.{}", std::process::id()));
    std::fs::write(&tmp, &script).ok()?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).ok()?;
    std::fs::rename(&tmp, &file).ok()?;
    Some(file)
}

pub(crate) fn launch_command() -> String {
    let program = appimage().or_else(|| std::env::current_exe().ok()).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "roamies".into());
    let word = launch_word(&program);
    // The commands people bind to keys go through the helper, which is quick; if it can't be written, the plain command.
    match write_ctl(&word) {
        Some(ctl) => launch_word(&ctl.to_string_lossy()),
        None => word,
    }
}

/// Where a copy installed from a .deb or .rpm gets its update: the package manager owns those files, so the app
/// can't swap itself as an AppImage can.
pub(crate) fn open_releases_page() {
    let _ = std::process::Command::new("xdg-open").arg("https://github.com/CreepingEmeraldz/Roamies-4-Linux/releases/latest").stdin(std::process::Stdio::null()).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shortcut_helper_talks_to_the_apps_own_dbus_address() {
        // The single-instance plugin's name is the identifier plus .SingleInstance; the helper must match it.
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"].as_str(), Some(BUS_ID));
        let s = ctl_script("'/home/me/Roamies Apps/Roamies.AppImage'");
        assert!(s.starts_with("#!/bin/sh\n"));
        assert!(s.contains("--dest com.kenkai.roamies.SingleInstance --object-path /com/kenkai/roamies/SingleInstance"));
        assert!(s.contains("org.SingleInstance.DBus.ExecuteCallback"));
        // Anything that isn't one of the known actions never reaches the D-Bus call (it is a quoted string there).
        assert!(s.contains("*) exec '/home/me/Roamies Apps/Roamies.AppImage' \"$@\" ;;"));
        // Not running: it starts the app.
        assert!(s.trim_end().ends_with("|| exec '/home/me/Roamies Apps/Roamies.AppImage' \"$@\""));
    }

    #[test]
    fn the_box_is_cut_from_the_full_size_picture_by_desktop_points() {
        use gtk::gdk_pixbuf::{Colorspace, Pixbuf};
        // A 400x200 picture of a desktop 200x100 points big at (100, 50): two pixels a point. Left half red, right half blue.
        let shot = Pixbuf::new(Colorspace::Rgb, false, 8, 400, 200).unwrap();
        for x in 0..400 {
            for y in 0..200 {
                shot.put_pixel(x, y, if x < 200 { 255 } else { 0 }, 0, if x < 200 { 0 } else { 255 }, 255);
            }
        }
        let dir = std::env::temp_dir();
        let (full, out) = (dir.join("roamies-test-full.png"), dir.join("roamies-test-out.png"));
        shot.savev(&full, "png", &[]).unwrap();
        let pick = Pick { id: Value::Null, full: full.clone(), bounds: [100.0, 50.0, 200.0, 100.0], file: out.to_string_lossy().into_owned() };
        // The right half of the desktop: x 200..300, all of its height.
        crop_pick(&pick, [200.0, 50.0, 100.0, 100.0]).unwrap();
        let got = Pixbuf::from_file(&out).unwrap();
        assert_eq!((got.width(), got.height()), (200, 200));
        assert_eq!(unsafe { got.pixels()[..3].to_vec() }, vec![0, 0, 255], "the right half is blue");
        // Past the edges: kept inside the picture.
        crop_pick(&pick, [250.0, 100.0, 500.0, 500.0]).unwrap();
        let got = Pixbuf::from_file(&out).unwrap();
        assert_eq!((got.width(), got.height()), (100, 100));
        let _ = std::fs::remove_file(full);
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn the_overlay_is_unmanaged_on_cosmic_only() {
        assert!(unmanaged_for(Some("COSMIC"), None));
        assert!(unmanaged_for(Some("pop:COSMIC"), None), "a desktop list");
        assert!(!unmanaged_for(Some("GNOME"), None));
        assert!(!unmanaged_for(Some("KDE"), None));
        assert!(!unmanaged_for(None, None));
        assert!(!unmanaged_for(Some("COSMIC"), Some("0")), "switched off by hand");
        assert!(unmanaged_for(Some("KDE"), Some("1")), "switched on by hand");
    }

    #[test]
    fn x11_key_state_maps_to_the_tap_bits() {
        use crate::tap::{COMMAND, CONTROL, OPTION, OTHER_MOD, SHIFT};
        assert_eq!(mods_from_x11(0x04 | 0x01), CONTROL | SHIFT);
        assert_eq!(mods_from_x11(0x08 | 0x40), OPTION | COMMAND);
        assert_eq!(mods_from_x11(0x04 | 0x80), CONTROL | OTHER_MOD, "AltGr is a modifier no pair uses");
        // Caps Lock (2), Num Lock (16) and the mouse buttons (0x100 and up) are not modifiers held.
        assert_eq!(mods_from_x11(0x02 | 0x10 | 0x100), 0);
        // Keycode 37 (left Control) is in the modifier map; keycode 38 ("a") is not.
        let mut keymap = [0u8; 32];
        keymap[37 / 8] |= 1 << (37 % 8);
        assert!(!other_key(&keymap, &[37, 50]));
        keymap[38 / 8] |= 1 << (38 % 8);
        assert!(other_key(&keymap, &[37, 50]));
    }

    #[test]
    fn the_autostart_entry_quotes_the_program() {
        let e = autostart_entry("/home/kai/Apps/Roamies 0.1.33.AppImage").expect("an entry");
        assert!(e.starts_with("[Desktop Entry]\nType=Application\nName=Roamies\n"));
        assert!(e.contains("\nExec=\"/home/kai/Apps/Roamies 0.1.33.AppImage\"\n"), "{e}");
        assert_eq!(exec_quoted("/usr/bin/roamies").as_deref(), Some("\"/usr/bin/roamies\""));
        // Characters a shell or the desktop would act on stay literal.
        assert_eq!(exec_quoted("/opt/a$b/50%/q\"t/b`t").as_deref(), Some("\"/opt/a\\\\$b/50%%/q\\\\\"t/b\\\\`t\""));
        assert_eq!(exec_quoted("/opt/back\\slash").as_deref(), Some("\"/opt/back\\\\\\\\slash\""));
        assert_eq!(exec_quoted("/opt/two\nlines"), None);
        assert_eq!(autostart_entry(""), None);
    }

    #[test]
    fn the_command_to_bind_names_this_copy() {
        assert_eq!(launch_word("/usr/bin/roamies"), "roamies", "the package's own command");
        assert_eq!(launch_word("/home/kai/Apps/Roamies_0.1.33_amd64.AppImage"), "/home/kai/Apps/Roamies_0.1.33_amd64.AppImage");
        assert_eq!(launch_word("/home/kai/My Apps/Roamies.AppImage"), "'/home/kai/My Apps/Roamies.AppImage'");
        assert_eq!(launch_word("/home/kai/it's/Roamies"), "'/home/kai/it'\\''s/Roamies'");
    }

    #[test]
    fn an_entry_the_desktop_switched_off_counts_as_off() {
        assert!(autostart_on(&autostart_entry("/usr/bin/roamies").unwrap()));
        assert!(!autostart_on("[Desktop Entry]\nExec=roamies\nHidden=true\n"));
        assert!(!autostart_on("[Desktop Entry]\nExec=roamies\nX-GNOME-Autostart-enabled=false\n"));
        assert!(!autostart_on("[Desktop Entry]\nHidden = True\n"));
        assert!(autostart_on("[Desktop Entry]\nHidden=false\n"));
    }

    #[test]
    fn screenshots_shrink_to_the_cap_and_never_grow() {
        assert_eq!(fit(1920, 1080, 1568), (1568, 882));
        assert_eq!(fit(3840, 2160, 1568), (1568, 882));
        assert_eq!(fit(1080, 2400, 1568), (706, 1568), "a tall one is capped by its height");
        assert_eq!(fit(1568, 900, 1568), (1568, 900));
        assert_eq!(fit(800, 600, 1568), (800, 600), "small ones stay as they are");
        assert_eq!(fit(5000, 1, 1568), (1568, 1), "never down to nothing");
        assert_eq!(fit(0, 0, 1568), (0, 0));
    }

    #[test]
    fn the_desktop_is_the_rectangle_round_every_screen() {
        assert_eq!(union_bounds(&[[0.0, 0.0, 1920.0, 1080.0]]), [0.0, 0.0, 1920.0, 1080.0]);
        assert_eq!(union_bounds(&[[0.0, 0.0, 1920.0, 1080.0], [1920.0, 0.0, 2560.0, 1440.0]]), [0.0, 0.0, 4480.0, 1440.0]);
        assert_eq!(union_bounds(&[[0.0, 0.0, 1920.0, 1080.0], [-1280.0, 56.0, 1280.0, 1024.0]]), [-1280.0, 0.0, 3200.0, 1080.0], "a screen to the left");
        assert_eq!(union_bounds(&[]), [0.0; 4]);
    }

    #[test]
    fn the_portal_answers_on_a_path_made_from_the_bus_name_and_token() {
        assert_eq!(portal_request_path(":1.42", "roamies_7_0"), "/org/freedesktop/portal/desktop/request/1_42/roamies_7_0");
        assert_eq!(portal_request_path(":1.1024", "t"), "/org/freedesktop/portal/desktop/request/1_1024/t");
    }

    #[test]
    fn only_file_links_become_paths() {
        assert_eq!(file_uri_path("file:///home/kai/Pictures/Screenshot%20from%202026.png"), Some(PathBuf::from("/home/kai/Pictures/Screenshot from 2026.png")));
        assert_eq!(file_uri_path("file://localhost/tmp/a.png"), Some(PathBuf::from("/tmp/a.png")));
        assert_eq!(file_uri_path("file:///tmp/100%.png"), Some(PathBuf::from("/tmp/100%.png")), "a stray percent sign stays");
        assert_eq!(file_uri_path("file:///tmp/caf%C3%A9.png"), Some(PathBuf::from("/tmp/café.png")));
        assert_eq!(file_uri_path("https://example.com/a.png"), None);
        assert_eq!(file_uri_path("file://otherhost/tmp/a.png"), None);
        assert_eq!(file_uri_path("/tmp/a.png"), None);
    }

    #[test]
    fn click_areas_become_whole_pixel_rectangles() {
        assert_eq!(hit_rects(&[10.4, 20.6, 30.2, 40.0, 0.0, 0.0, 5.0, 5.0]), vec![(10, 20, 31, 40), (0, 0, 5, 5)]);
        assert_eq!(hit_rects(&[]), vec![]);
        // A trailing partial rectangle, an empty one and bad numbers are dropped.
        assert_eq!(hit_rects(&[1.0, 2.0, 3.0, 4.0, 9.0, 9.0]), vec![(1, 2, 3, 4)]);
        assert_eq!(hit_rects(&[1.0, 2.0, 0.0, 4.0, f64::NAN, 0.0, 5.0, 5.0, 0.0, 0.0, f64::INFINITY, 5.0, 0.0, 0.0, 1e9, 5.0]), vec![]);
        assert_eq!(hit_rects(&[-8.0, -8.0, 40.0, 40.0]), vec![(-8, -8, 40, 40)], "a pad can reach past the edge");
        assert_eq!(hit_rects(&[1.0; 4 * 1000]).len(), 600);
    }

    #[test]
    fn focus_is_asked_for_as_a_taskbar_would() {
        assert_eq!(activation_request()[0], 2, "source 2 (pager), which focus-stealing prevention lets through");
        assert_eq!(activation_request()[1], 0, "no timestamp");
    }

    #[test]
    fn the_session_is_wayland_when_either_sign_says_so() {
        assert_eq!(session_from(Some("x11"), None), Session::X11);
        assert_eq!(session_from(None, None), Session::X11, "Xvfb and bare X servers set nothing");
        assert_eq!(session_from(Some("x11"), Some("")), Session::X11);
        assert_eq!(session_from(Some("wayland"), Some("wayland-0")), Session::Wayland);
        assert_eq!(session_from(None, Some("wayland-1")), Session::Wayland);
        assert_eq!(session_from(Some("Wayland"), None), Session::Wayland);
        assert_eq!(session_from(Some("tty"), Some("wayland-0")), Session::Wayland, "a compositor started from a console");
    }
}
