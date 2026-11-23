// A GTK layer-shell host for the Roamies desktop page.
//
// The Tauri app runs under XWayland for window placement and pointer access, and Hyprland
// deliberately ignores input regions on toplevels, so a Tauri overlay cannot be click-through.
// Layer surfaces do honour the input region, so this small host runs as a native Wayland
// client, loads dist/desktop.html, and bridges the page to the Tauri app over stdin/stdout.
//
// stdin  (from the app): {"type":"host","msg":{...}} | {"type":"js","code":"..."} |
//                        {"type":"ptr","x":..,"y":..,"t":..} | {"type":"quit"}
// stdout (to the app):   {"type":"sidecar_send","msg":{...}}

#[cfg(target_os = "linux")]
mod imp {
    use std::io::{BufRead, Write};
    use std::sync::mpsc;

    use gtk::prelude::*;
    use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
    use javascriptcore::ValueExt;
    use webkit2gtk::{
        NavigationPolicyDecision, NavigationPolicyDecisionExt, PolicyDecisionExt,
        PolicyDecisionType, SecurityManagerExt, SettingsExt, URIRequestExt, URISchemeRequestExt,
        UserContentInjectedFrames, UserContentManager, UserContentManagerExt, UserScript,
        UserScriptInjectionTime, WebContextExt, WebViewExt, WebViewExtManual,
    };

    const SHIM: &str = r#"
    (function () {
      if (window.__TAURI__) return;
      window.__pending = {}; window.__rid = 0;
      window.__TAURI__ = {
        core: {
          invoke: function (cmd, args) {
            return new Promise(function (resolve, reject) {
              var id = ++window.__rid;
              window.__pending[id] = { resolve: resolve, reject: reject };
              window.webkit.messageHandlers.roamies.postMessage(JSON.stringify({ id: id, cmd: cmd, args: args || {} }));
            });
          }
        }
      };
      window.__roamieInvokeReply = function (id, ok, value) {
        var p = window.__pending[id]; if (!p) return; delete window.__pending[id];
        if (ok) p.resolve(value); else p.reject(new Error(value || 'invoke failed'));
      };
    })();
    "#;

    pub(super) fn main() {
        let mut dist = String::new();
        let mut extra = String::new();
        let mut check = false;
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            match a.as_str() {
                "--dist" => dist = args.next().unwrap_or_default(),
                "--init" => extra = args.next().unwrap_or_default(),
                "--supported" => check = true,
                other => eprintln!("roamies-overlay: ignoring argument {other}"),
            }
        }
        // The app asks first whether this compositor supports layer-shell. A Wayland display and GTK are
        // enough to answer; GNOME and some others say no, and the app keeps its ordinary Tauri overlay.
        if check {
            if gtk::init().is_err() {
                std::process::exit(1);
            }
            std::process::exit(if gtk_layer_shell::is_supported() {
                0
            } else {
                1
            });
        }
        if dist.is_empty() {
            eprintln!("roamies-overlay: --dist <path to dist> is required");
            std::process::exit(2);
        }

        gtk::init().expect("gtk init");
        let win = gtk::Window::new(gtk::WindowType::Toplevel);
        win.set_decorated(false);
        win.set_app_paintable(true);
        if let Some(screen) = WidgetExt::screen(&win) {
            if let Some(visual) = screen.rgba_visual() {
                win.set_visual(Some(&visual));
            }
        }

        win.init_layer_shell();
        win.set_namespace("roamies");
        win.set_layer(Layer::Overlay);
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            win.set_anchor(edge, true);
        }
        win.set_exclusive_zone(-1);
        win.set_keyboard_mode(KeyboardMode::None);

        let manager = UserContentManager::new();
        manager.register_script_message_handler("roamies");
        let mut init_js = format!(
            "{SHIM}\nwindow.ROAMIE_LINUX='wayland';window.ROAMIE_ONLY=['sidekick'];window.ROAMIE_AUTOSOUND=true;window.ROAMIE_UNIT=400;\n{extra}"
        );
        init_js.push_str("\nwindow.addEventListener('load',function(){[1200,3000,6000].forEach(function(ms){setTimeout(function(){window.__roamieHost&&window.__roamieHost({kind:'summon'});},ms);});});");
        manager.add_script(&UserScript::new(
            &init_js,
            UserContentInjectedFrames::AllFrames,
            UserScriptInjectionTime::Start,
            &[],
            &[],
        ));

        // Serve the page through our own scheme instead of file://, so the page can never read arbitrary local
        // files. The Tauri overlay uses its own protocol for the same reason.
        let context = webkit2gtk::WebContext::default().expect("webkit web context");
        if let Some(security) = context.security_manager() {
            security.register_uri_scheme_as_local("roamies");
            security.register_uri_scheme_as_secure("roamies");
        }
        let dist_files =
            std::fs::canonicalize(&dist).unwrap_or_else(|_| std::path::PathBuf::from(&dist));
        let root = dist_files.clone();
        context.register_uri_scheme("roamies", move |req| {
            let path = req.path().map(|p| p.to_string()).unwrap_or_default();
            let file = root.join(path.trim_start_matches('/'));
            let data = file
                .canonicalize()
                .ok()
                .filter(|p| p.starts_with(&root))
                .and_then(|p| std::fs::read(p).ok());
            let (body, mime) = match data {
                Some(bytes) => {
                    let mime = match file.extension().and_then(|e| e.to_str()).unwrap_or("") {
                        "html" => "text/html",
                        "js" | "mjs" => "text/javascript",
                        "css" => "text/css",
                        "json" => "application/json",
                        "png" => "image/png",
                        "gif" => "image/gif",
                        "svg" => "image/svg+xml",
                        "woff2" => "font/woff2",
                        "woff" => "font/woff",
                        "ttf" => "font/ttf",
                        "jpg" | "jpeg" => "image/jpeg",
                        _ => "application/octet-stream",
                    };
                    (bytes, mime)
                }
                None => (Vec::new(), "text/plain"),
            };
            let bytes = gtk::glib::Bytes::from(&body);
            let stream = gtk::gio::MemoryInputStream::from_bytes(&bytes);
            req.finish(&stream, body.len() as i64, Some(mime));
        });
        let web =
            webkit2gtk::WebView::new_with_context_and_user_content_manager(&context, &manager);
        web.set_background_color(&gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
        if let Some(settings) = WebViewExt::settings(&web) {
            settings.set_allow_file_access_from_file_urls(false);
            settings.set_allow_universal_access_from_file_urls(false);
        }
        web.connect_load_changed(|_, event| {
            if event == webkit2gtk::LoadEvent::Finished {
                eprintln!("[roamies-overlay] page loaded");
            }
        });
        web.connect_load_failed(|_, _event, uri, err| {
            eprintln!("[roamies-overlay] couldn't load {uri}: {err}");
            false
        });
        // Nothing but our own pages may be navigated to, and no page may open another window.
        web.connect_decide_policy(move |_, decision, kind| {
            match kind {
                PolicyDecisionType::NavigationAction => {
                    if let Some(nav) = decision.downcast_ref::<NavigationPolicyDecision>() {
                        if let Some(req) = nav.navigation_action().and_then(|a| a.request()) {
                            let uri = req.uri().unwrap_or_default();
                            if !uri.starts_with("roamies://app/") && !uri.starts_with("about:") {
                                decision.ignore();
                                return true;
                            }
                        }
                    }
                }
                PolicyDecisionType::NewWindowAction => {
                    decision.ignore();
                    return true;
                }
                _ => {}
            }
            false
        });
        win.add(&web);

        win.connect_realize(|w| {
            eprintln!("[roamies-overlay] layer={}", w.is_layer_window());
            apply_shape(w, &[]);
        });

        // The page's invoke() calls.
        let web2 = web.clone();
        let win2 = win.clone();
        manager.connect_script_message_received(Some("roamies"), move |_m, result| {
            let Some(value) = result.js_value() else {
                return;
            };
            let text = value.to_str();
            let Ok(msg) = serde_json::from_str::<serde_json::Value>(&text) else {
                return;
            };
            let id = msg.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
            let cmd = msg.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
            let args = msg.get("args").cloned().unwrap_or(serde_json::Value::Null);
            let reply = |ok: bool, value: serde_json::Value| {
                let js = format!(
                    "window.__roamieInvokeReply&&__roamieInvokeReply({id},{},{})",
                    ok,
                    serde_json::to_string(&value).unwrap_or_else(|_| "null".into())
                );
                let _ = web2.evaluate_javascript(
                    &js,
                    None,
                    None,
                    None::<&gtk::gio::Cancellable>,
                    |_| {},
                );
            };
            match cmd {
                "sidecar_send" => {
                    let msg = args.get("msg").cloned().unwrap_or(serde_json::Value::Null);
                    let line = serde_json::json!({ "type": "sidecar_send", "msg": msg });
                    println!("{line}");
                    let _ = std::io::stdout().flush();
                    reply(true, serde_json::Value::Null);
                }
                "set_hit_rects" => {
                    let rects: Vec<f64> = args
                        .get("rects")
                        .and_then(|v| v.as_array())
                        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
                        .unwrap_or_default();
                    apply_shape(&win2, &rects);
                    reply(true, serde_json::Value::Null);
                }
                "set_typing" => {
                    let on = args.get("on").and_then(|v| v.as_bool()).unwrap_or(false);
                    win2.set_keyboard_mode(if on {
                        KeyboardMode::Exclusive
                    } else {
                        KeyboardMode::None
                    });
                    if on {
                        win2.present();
                    }
                    reply(true, serde_json::Value::Null);
                }
                // Not used on Linux: clicks go through by region (set_hit_rects), and the buddy overlay is macOS only.
                "set_hot" | "esc_watch" => reply(true, serde_json::Value::Null),
                "buddy" => reply(true, serde_json::Value::Bool(false)),
                _ => reply(false, serde_json::json!("unknown command")),
            }
        });

        // The handler must exist before the page starts using window.webkit.messageHandlers.
        web.load_uri("roamies://app/desktop.html");

        // The app's messages, evaluated on the GTK main loop.
        let (tx, rx) = mpsc::channel::<String>();
        let tx_ptr = tx.clone();
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
            // The app closed the pipe (Quit, an update, a relaunch): leave, instead of staying on screen as a ghost.
            let _ = tx.send(r#"{"type":"quit"}"#.to_string());
        });

        let web3 = web.clone();
        gtk::glib::timeout_add_local(std::time::Duration::from_millis(8), move || {
            while let Ok(line) = rx.try_recv() {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                match value.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                    "host" => {
                        let msg = value.get("msg").cloned().unwrap_or(serde_json::Value::Null);
                        let js = format!(
                            "window.__roamieHost&&window.__roamieHost({})",
                            serde_json::to_string(&msg).unwrap_or_else(|_| "null".into())
                        );
                        let _ = web3.evaluate_javascript(
                            &js,
                            None,
                            None,
                            None::<&gtk::gio::Cancellable>,
                            |_| {},
                        );
                    }
                    "js" => {
                        if let Some(code) = value.get("code").and_then(|v| v.as_str()) {
                            let _ = web3.evaluate_javascript(
                                code,
                                None,
                                None,
                                None::<&gtk::gio::Cancellable>,
                                |_| {},
                            );
                        }
                    }
                    "ptr" => {
                        let x = value.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let y = value.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let t = value.get("t").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let js = format!("window.__roamiePtr&&__roamiePtr({x},{y},{t})");
                        let _ = web3.evaluate_javascript(
                            &js,
                            None,
                            None,
                            None::<&gtk::gio::Cancellable>,
                            |_| {},
                        );
                    }
                    "quit" => gtk::main_quit(),
                    _ => {}
                }
            }
            gtk::glib::ControlFlow::Continue
        });

        // Pointer position for the page (XWayland's cursor is the same global pointer).
        if let Ok((conn, screen_num)) = x11rb::connect(None) {
            use x11rb::connection::Connection;
            use x11rb::protocol::xproto::ConnectionExt;
            let root = conn.setup().roots[screen_num].root;
            std::thread::spawn(move || loop {
                if let Ok(cookie) = conn.query_pointer(root) {
                    if let Ok(p) = cookie.reply() {
                        let t = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs_f64() * 1000.0)
                            .unwrap_or(0.0);
                        let line = serde_json::json!({ "type": "ptr", "x": p.root_x, "y": p.root_y, "t": t });
                        if tx_ptr.send(line.to_string()).is_err() {
                            break;
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(16));
            });
        }

        win.show_all();

        gtk::main();
    }

    fn apply_shape(window: &gtk::Window, flat: &[f64]) {
        let Some(gdk) = window.window() else { return };
        let rects: Vec<gtk::cairo::RectangleInt> = flat
            .chunks_exact(4)
            .filter(|r| r[2] >= 1.0 && r[3] >= 1.0 && r.iter().all(|v| v.is_finite()))
            .map(|r| {
                gtk::cairo::RectangleInt::new(
                    r[0].floor() as i32,
                    r[1].floor() as i32,
                    r[2].ceil() as i32,
                    r[3].ceil() as i32,
                )
            })
            .take(600)
            .collect();
        let region = gtk::cairo::Region::create_rectangles(&rects);
        gdk.input_shape_combine_region(&region, 0, 0);
    }
}

#[cfg(target_os = "linux")]
fn main() {
    imp::main()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("roamies-overlay is a Linux-only helper");
    std::process::exit(1);
}
