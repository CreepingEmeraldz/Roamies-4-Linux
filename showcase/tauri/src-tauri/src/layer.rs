// Linux: the overlay as a Wayland layer-shell surface (wlr-layer-shell), on the compositors that have it: niri,
// sway, Hyprland, river, Wayfire, labwc and the like (not GNOME's Mutter).
//
// Why: the XWayland path (linux.rs) needs a window manager that lets an X11 window sit on top, unmanaged, at the
// size of the screen. niri runs X11 apps through xwayland-satellite as ordinary tiled windows, so the overlay would
// be one more tile. A layer surface is what a bar or a wallpaper is: the compositor puts it on its own output, over
// every window (fullscreen ones too), outside the tiling, and gives it no focus unless it asks. Click-through by
// region (linux.rs set_hit_rects) is the surface's input region on Wayland, so that part is the same code.
//
// What a layer surface can't do is see the pointer over other apps: a Wayland client hears about the pointer only
// while it is over one of its own surfaces. niri has no IPC for it either (as of niri-ipc 25.11). So on this
// backend the page gets no pointer samples (main.rs pump_cursor) and the sidecar doesn't offer the Cursor size.

use std::sync::OnceLock;

use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use tauri::WebviewWindow;

/// Desktops (XDG_CURRENT_DESKTOP) whose compositor implements zwlr_layer_shell_v1.
const KNOWN: [&str; 6] = ["niri", "sway", "hyprland", "river", "wayfire", "labwc"];

/// Whether to use layer-shell: `force` is ROAMIES_LAYER ("1" on, "0" off), else a compositor that is known to have it,
/// told by its desktop name or by the socket variable its own IPC sets (`own_ipc`: NIRI_SOCKET, SWAYSOCK or
/// HYPRLAND_INSTANCE_SIGNATURE), as XDG_CURRENT_DESKTOP is not always set by a hand-written session.
fn wanted_for(desktop: Option<&str>, force: Option<&str>, own_ipc: bool) -> bool {
    match force {
        Some("1") => true,
        Some("0") => false,
        _ => own_ipc || desktop.is_some_and(|d| d.split(':').any(|p| KNOWN.iter().any(|k| p.eq_ignore_ascii_case(k)))),
    }
}

/// Read once, before GTK starts (linux::init), because it decides which GDK backend GTK gets.
pub(crate) fn wanted() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let env = |k: &str| std::env::var(k).ok();
        let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
        let own_ipc = set("NIRI_SOCKET") || set("SWAYSOCK") || set("HYPRLAND_INSTANCE_SIGNATURE");
        wanted_for(env("XDG_CURRENT_DESKTOP").as_deref(), env("ROAMIES_LAYER").as_deref(), own_ipc)
    })
}

/// Whether the compositor really has the protocol. Needs GTK started on Wayland. When it hasn't, this process can't
/// go back to X11 (GDK picked its backend already), so the caller starts it again with ROAMIES_LAYER=0.
pub(crate) fn supported() -> bool {
    gtk_layer_shell::is_supported()
}

/// Turns the (still hidden, never shown) overlay into a layer surface on output `index` (the same order as
/// Tauri's monitors, which is GDK's): over everything, all four edges, so it covers the output whole, and under no bar's
/// reserved strip (exclusive zone -1). No keyboard until `keyboard` gives it. Main thread only. False when it can't be.
fn attach(window: &WebviewWindow, index: usize) -> bool {
    use gtk::prelude::{GtkWindowExt, WidgetExt};
    let Ok(gw) = window.gtk_window() else { return false };
    // The layer role has to be set before the window exists on the screen.
    if gw.is_realized() {
        return false;
    }
    // tao gives an undecorated window on Wayland an empty titlebar, to make GTK draw its own (shadowless) frame instead
    // of asking the compositor for one. A layer surface has no frame, and the extra shadow border makes GTK set the
    // whole window as the input region when it maps, which overrides the click-through shape.
    gw.set_titlebar(None::<&gtk::Widget>);
    gw.init_layer_shell();
    gw.set_layer(Layer::Overlay);
    gw.set_namespace("roamies");
    for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
        gw.set_anchor(edge, true);
    }
    gw.set_exclusive_zone(-1);
    gw.set_keyboard_mode(KeyboardMode::None);
    if let Some(monitor) = gtk::gdk::Display::default().and_then(|d| d.monitor(index as i32)) {
        gw.set_monitor(&monitor);
    }
    true
}

/// Makes the overlay a layer surface and shows it, with no input at all until the page reports its click areas
/// (linux::set_hit_rects): tao's own "ignore cursor events" needs a window that is already on the screen. Queued for
/// the main thread, where the window is built.
pub(crate) fn present(window: &WebviewWindow, index: usize) {
    let w = window.clone();
    let _ = window.run_on_main_thread(move || {
        use gtk::cairo::Region;
        use gtk::prelude::WidgetExt;
        if !attach(&w, index) {
            eprintln!("[layer] couldn't make the overlay a layer surface");
        }
        let Ok(gw) = w.gtk_window() else { return };
        gw.realize();
        if let Some(win) = gw.window() {
            win.input_shape_combine_region(&Region::create(), 0, 0);
        }
        gw.show_all();
    });
}

/// The keyboard for the overlay while the question field is open (exclusive: the field is up only for a moment and
/// the key press that opened it went to another app), then none again. Layer overlays only.
pub(crate) fn keyboard(window: &WebviewWindow, on: bool) {
    let w = window.clone();
    let _ = window.run_on_main_thread(move || {
        let Ok(gw) = w.gtk_window() else { return };
        if gw.is_layer_window() {
            gw.set_keyboard_mode(if on { KeyboardMode::Exclusive } else { KeyboardMode::None });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_shell_is_used_where_the_compositor_is_known_to_have_it() {
        assert!(wanted_for(Some("niri"), None, false));
        assert!(wanted_for(Some("sway"), None, false));
        assert!(wanted_for(Some("Hyprland"), None, false), "any case");
        assert!(wanted_for(Some("wlroots:river"), None, false), "a desktop list");
        assert!(!wanted_for(Some("GNOME"), None, false), "Mutter has no layer-shell");
        assert!(!wanted_for(Some("ubuntu:GNOME"), None, false));
        assert!(!wanted_for(Some("KDE"), None, false), "KDE keeps the XWayland path unless asked");
        assert!(!wanted_for(Some("niri-but-not"), None, false), "whole names only");
        assert!(!wanted_for(None, None, false));
    }

    #[test]
    fn a_compositors_own_socket_counts_when_the_desktop_name_is_missing() {
        assert!(wanted_for(None, None, true));
        assert!(wanted_for(Some("GNOME"), None, true));
    }

    #[test]
    fn roamies_layer_decides_when_set() {
        assert!(wanted_for(Some("KDE"), Some("1"), false), "switched on by hand");
        assert!(!wanted_for(Some("niri"), Some("0"), true), "switched off by hand");
        assert!(wanted_for(Some("niri"), Some("yes"), false), "anything else is no decision");
    }
}
