// roamies-atspi windows            the open windows: {"ok":true,"windows":[{id,app,title,frontApp?}]}
// roamies-atspi ui <spec> [controls]   one window's controls and text. spec: front | app:<name> | window:<id>
// roamies-atspi enable             tells the apps to publish their accessibility tree (see `enable`)
//
// Read only. Controls are numbered 1.. in the order they are read; the numbers only mean something to this one reading.
// Every JSON answer is one line on stdout; a failure is {"ok":false,"error":"..."} and exit code 1.
mod logic;

use serde_json::{json, Value};
use std::time::{Duration, Instant};
use zbus::blocking::{connection::Builder, Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, Value as ZValue};

const ACC: &str = "org.a11y.atspi.Accessible";
const REGISTRY: (&str, &str) = ("org.a11y.atspi.Registry", "/org/a11y/atspi/accessible/root");

fn out(v: Value) -> ! {
    println!("{v}");
    std::process::exit(if v["ok"] == false { 1 } else { 0 });
}
fn fail(msg: &str) -> ! {
    out(json!({ "ok": false, "error": msg }))
}

/// One accessible object: the bus name of the app that owns it and its path.
#[derive(Clone, Debug)]
struct El {
    dest: String,
    path: String,
}

struct Bus {
    c: Connection,
}

impl Bus {
    /// The accessibility bus has its own address, which the session bus hands out.
    fn open() -> Result<Bus, String> {
        let session = Connection::session().map_err(|e| format!("no session bus: {e}"))?;
        let p = Proxy::new(&session, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus").map_err(|e| e.to_string())?;
        let addr: String = p.call("GetAddress", &()).map_err(|e| format!("the accessibility bus isn't running: {e}"))?;
        let c = Builder::address(addr.as_str())
            .map_err(|e| e.to_string())?
            .method_timeout(Duration::from_millis(1500))
            .build()
            .map_err(|e| format!("couldn't reach the accessibility bus: {e}"))?;
        Ok(Bus { c })
    }
    fn proxy(&self, el: &El, iface: &'static str) -> Option<Proxy<'_>> {
        Proxy::new(&self.c, el.dest.clone(), el.path.clone(), iface).ok()
    }
    fn children(&self, el: &El) -> Vec<El> {
        let Some(p) = self.proxy(el, ACC) else { return vec![] };
        let kids: Vec<(String, OwnedObjectPath)> = p.call("GetChildren", &()).unwrap_or_default();
        kids.into_iter().map(|(d, o)| El { dest: d, path: o.to_string() }).filter(|e| !e.dest.is_empty() && e.path != "/org/a11y/atspi/null").collect()
    }
    fn name(&self, el: &El) -> String {
        self.proxy(el, ACC).and_then(|p| p.get_property::<String>("Name").ok()).unwrap_or_default()
    }
    fn description(&self, el: &El) -> String {
        self.proxy(el, ACC).and_then(|p| p.get_property::<String>("Description").ok()).unwrap_or_default()
    }
    fn role(&self, el: &El) -> String {
        self.proxy(el, ACC).and_then(|p| p.call::<_, _, String>("GetRoleName", &()).ok()).unwrap_or_default()
    }
    fn state(&self, el: &El) -> u64 {
        let v: Vec<u32> = self.proxy(el, ACC).and_then(|p| p.call("GetState", &()).ok()).unwrap_or_default();
        v.first().copied().unwrap_or(0) as u64 | (v.get(1).copied().unwrap_or(0) as u64) << 32
    }
    fn interfaces(&self, el: &El) -> Vec<String> {
        self.proxy(el, ACC).and_then(|p| p.call("GetInterfaces", &()).ok()).unwrap_or_default()
    }
    fn text(&self, el: &El) -> String {
        let Some(p) = self.proxy(el, "org.a11y.atspi.Text") else { return String::new() };
        let n: i32 = p.get_property("CharacterCount").unwrap_or(0);
        if n <= 0 {
            return String::new();
        }
        p.call("GetText", &(0i32, n.min(4000))).unwrap_or_default()
    }
    fn value(&self, el: &El) -> Option<f64> {
        self.proxy(el, "org.a11y.atspi.Value")?.get_property("CurrentValue").ok()
    }
    /// x, y, width, height on screen (Wayland apps often can't say where their windows are: then 0, 0).
    fn extents(&self, el: &El) -> Option<[i32; 4]> {
        let (x, y, w, h): (i32, i32, i32, i32) = self.proxy(el, "org.a11y.atspi.Component")?.call("GetExtents", &(0u32,)).ok()?;
        Some([x, y, w, h])
    }
}

/// Apps that publish an accessibility tree switch it on when the bus says assistive tools are in use. Chromium (Brave,
/// Chrome, Electron apps) only builds its page tree then; GTK, Qt and Firefox publish regardless. A browser started
/// before this may need a restart to notice.
fn enable() -> Result<(), String> {
    let session = Connection::session().map_err(|e| e.to_string())?;
    let p = Proxy::new(&session, "org.a11y.Bus", "/org/a11y/bus", "org.freedesktop.DBus.Properties").map_err(|e| e.to_string())?;
    p.call::<_, _, ()>("Set", &("org.a11y.Status", "IsEnabled", ZValue::from(true))).map_err(|e| e.to_string())
}

struct Win {
    el: El,
    app: String,
    title: String,
    active: bool,
}

fn windows(bus: &Bus) -> Vec<Win> {
    let root = El { dest: REGISTRY.0.into(), path: REGISTRY.1.into() };
    let me = std::env::var("ROAMIES_ATSPI_SKIP").unwrap_or_else(|_| "roamies".into()).to_lowercase();
    let mut wins = vec![];
    for app in bus.children(&root) {
        let app_name = bus.name(&app);
        if app_name.is_empty() || app_name.to_lowercase() == me {
            continue;
        }
        for w in bus.children(&app) {
            let role = bus.role(&w);
            if !logic::is_window_role(&role) {
                continue;
            }
            let state = bus.state(&w);
            if logic::has(state, logic::DEFUNCT) {
                continue;
            }
            wins.push(Win { title: bus.name(&w), active: logic::has(state, logic::ACTIVE), el: w, app: app_name.clone() });
        }
    }
    wins
}

fn id_of(w: &El) -> String {
    format!("{}|{}", w.dest, w.path)
}

fn pick<'a>(wins: &'a [Win], spec: &str) -> Option<&'a Win> {
    if spec == "front" || spec.is_empty() {
        return wins.iter().find(|w| w.active).or(wins.first());
    }
    if let Some(id) = spec.strip_prefix("window:") {
        return wins.iter().find(|w| id_of(&w.el) == id);
    }
    let want = spec.strip_prefix("app:").unwrap_or(spec).to_lowercase();
    let hits: Vec<&Win> = wins.iter().filter(|w| w.app.to_lowercase().contains(&want) || w.title.to_lowercase().contains(&want)).collect();
    hits.iter().find(|w| w.active).or(hits.first()).copied()
}

fn win_info(w: &Win, bus: &Bus) -> Value {
    let mut v = json!({ "id": id_of(&w.el), "app": w.app, "title": w.title });
    if let Some(b) = bus.extents(&w.el).filter(|b| b[2] > 0 && b[3] > 0 && (b[0] != 0 || b[1] != 0)) {
        v["bounds"] = json!(b);
    }
    v
}

const MAX_NODES: usize = 6000;
/// A busy web page has thousands of links; the model gets the first screenfuls, like the Windows helper's 260.
const MAX_CONTROLS: usize = 260;
const MAX_DEPTH: usize = 40;

struct Read {
    controls: Vec<Value>,
    on: Vec<(f64, f64, String)>,
    off: Vec<(f64, String)>,
    seen: std::collections::HashSet<String>,
    nodes: usize,
    started: Instant,
}

fn walk(bus: &Bus, el: &El, depth: usize, r: &mut Read) {
    if depth > MAX_DEPTH || r.nodes >= MAX_NODES || r.controls.len() >= MAX_CONTROLS || r.started.elapsed() > Duration::from_secs(8) {
        return;
    }
    r.nodes += 1;
    let role = bus.role(el);
    let kind = logic::kind(&role);
    let mut stop = false; // a text control's children are its characters: the text is read from it, not from them
    if kind != logic::Kind::Skip {
        let state = bus.state(el);
        if logic::has(state, logic::DEFUNCT) {
            return;
        }
        let visible = logic::has(state, logic::VISIBLE) || logic::has(state, logic::SHOWING);
        let showing = logic::has(state, logic::SHOWING);
        // Not drawn at all (a hidden tab's contents): nothing under it either.
        if !visible && !logic::has(state, logic::MANAGES_DESCENDANTS) && kind != logic::Kind::Container {
            return;
        }
        // A "text" role you can't edit is words on the page (a paragraph in a document), not a field.
        let kind = if role == "text" && !logic::has(state, logic::EDITABLE) { logic::Kind::Text } else { kind };
        let ext = bus.extents(el);
        let (y, x) = ext.map(|e| (e[1] as f64, e[0] as f64)).unwrap_or((0.0, 0.0));
        match kind {
            logic::Kind::Control(word) => {
                let editable = logic::has(state, logic::EDITABLE);
                let multi = logic::has(state, logic::MULTI_LINE);
                let word = logic::control_word(&role, word, editable, multi);
                stop = matches!(word, "text field" | "text area" | "password field");
                let mut label = bus.name(el);
                let mut value = String::new();
                match word {
                    "text field" | "text area" | "password field" | "combo box" => {
                        if word != "password field" {
                            value = bus.text(el);
                        }
                    }
                    "slider" | "stepper" => value = bus.value(el).map(logic::num).unwrap_or_default(),
                    "checkbox" | "radio" => value = if logic::has(state, logic::CHECKED) { "on".into() } else { "off".into() },
                    "link" if label.is_empty() => label = bus.text(el),
                    _ => {}
                }
                if label.is_empty() {
                    label = bus.description(el);
                }
                if label.is_empty() && value.is_empty() && !matches!(word, "text field" | "text area" | "password field") {
                    // An unnamed button is still worth a line when its children name it (an icon with a tooltip).
                    let named = bus.children(el).into_iter().map(|c| bus.name(&c)).find(|n| !n.is_empty());
                    label = named.unwrap_or_default();
                }
                let label = logic::clean(&label, 160);
                let value = logic::clean(&value, 400);
                if !(label.is_empty() && value.is_empty() && word != "button") {
                    let mut c = json!({ "id": r.controls.len() + 1, "role": word, "label": label });
                    if !value.is_empty() {
                        c["value"] = json!(value);
                    }
                    if logic::has(state, logic::FOCUSED) {
                        c["focused"] = json!(true);
                    }
                    if !showing {
                        c["offscreen"] = json!(true);
                    }
                    r.controls.push(c);
                }
            }
            logic::Kind::Text => {
                let mut t = bus.name(el);
                if t.is_empty() && bus.interfaces(el).iter().any(|i| i.ends_with(".Text")) {
                    t = bus.text(el);
                }
                let t = logic::clean(&t, 600);
                if !t.is_empty() && r.seen.insert(t.clone()) {
                    if showing {
                        r.on.push((y, x, t));
                    } else {
                        r.off.push((y, t));
                    }
                }
            }
            _ => {}
        }
    }
    if stop {
        return;
    }
    for k in bus.children(el) {
        walk(bus, &k, depth + 1, r);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str);
    match arg(0) {
        Some("enable") => match enable() {
            Ok(()) => out(json!({ "ok": true })),
            Err(e) => fail(&e),
        },
        Some("windows") => {
            let _ = enable();
            let bus = Bus::open().unwrap_or_else(|e| fail(&e));
            let list: Vec<Value> = windows(&bus)
                .iter()
                .map(|w| {
                    let mut v = json!({ "id": id_of(&w.el), "app": w.app, "title": w.title });
                    if w.active {
                        v["frontApp"] = json!(true);
                    }
                    v
                })
                .collect();
            out(json!({ "ok": true, "windows": list }))
        }
        Some("ui") => {
            let _ = enable();
            let bus = Bus::open().unwrap_or_else(|e| fail(&e));
            let wins = windows(&bus);
            let Some(w) = pick(&wins, arg(1).unwrap_or("front")) else { fail("No matching window is open.") };
            let mut r = Read { controls: vec![], on: vec![], off: vec![], seen: Default::default(), nodes: 0, started: Instant::now() };
            walk(&bus, &w.el, 0, &mut r);
            let info = win_info(w, &bus);
            if arg(2) == Some("controls") {
                out(json!({ "ok": true, "window": info, "controls": r.controls }))
            }
            let note = if r.controls.len() >= MAX_CONTROLS {
                "Only the first controls are listed (this page has many more); its text is below. To use others, search for them by their words."
            } else if r.controls.is_empty() && r.on.is_empty() {
                "This app shares no controls. Chromium-based browsers need accessibility on: if Roamies just switched it on, restart the browser once. Otherwise use a screenshot."
            } else {
                ""
            };
            out(json!({ "ok": true, "window": info, "controls": r.controls, "texts": logic::page_text(r.on, 5000), "elsewhere": logic::offscreen_text(r.off, 4000), "note": note }))
        }
        _ => fail("usage: roamies-atspi windows | ui <front|app:NAME|window:ID> [controls] | enable"),
    }
}
