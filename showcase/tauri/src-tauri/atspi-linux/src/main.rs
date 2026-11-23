// roamies-atspi windows            the open windows: {"ok":true,"windows":[{id,app,title,frontApp?}]}
// roamies-atspi ui <spec> [controls]   one window's controls and text. spec: front | app:<name> | window:<id>
// roamies-atspi enable             tells the apps to publish their accessibility tree (see `enable`)
//
// Read only. Controls are numbered 1.. in the order they are read; the numbers only mean something to this one reading.
// Every JSON answer is one line on stdout; a failure is {"ok":false,"error":"..."} and exit code 1.
mod act;
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
        // No property cache: by default every new proxy subscribes to changes and reads all of an interface's properties
        // first, several round trips per node when each node is only asked once.
        zbus::blocking::proxy::Builder::new(&self.c).destination(el.dest.clone()).ok()?.path(el.path.clone()).ok()?.interface(iface).ok()?
            .cache_properties(zbus::proxy::CacheProperties::No).build().ok()
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

/// The title of the window the compositor has focused (niri only), which says which of an app's windows is in front:
/// an app's own "active" flag over the accessibility bus can be stale or set on several windows.
fn focused_title() -> Option<String> {
    std::env::var_os("NIRI_SOCKET").filter(|v| !v.is_empty())?;
    let out = std::process::Command::new("niri").args(["msg", "--json", "focused-window"]).output().ok()?;
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    v["title"].as_str().filter(|t| !t.is_empty()).map(String::from)
}

fn pick<'a>(wins: &'a [Win], spec: &str) -> Option<&'a Win> {
    let focus = focused_title();
    let best = |hits: Vec<&'a Win>| -> Option<&'a Win> {
        focus.as_deref().and_then(|f| hits.iter().find(|w| logic::same_title(&w.title, f)))
            .or_else(|| hits.iter().find(|w| w.active))
            .or(hits.first())
            .copied()
    };
    if spec == "front" || spec.is_empty() {
        return best(wins.iter().collect());
    }
    if let Some(id) = spec.strip_prefix("window:") {
        return wins.iter().find(|w| id_of(&w.el) == id);
    }
    let want = spec.strip_prefix("app:").unwrap_or(spec).to_lowercase();
    best(wins.iter().filter(|w| w.app.to_lowercase().contains(&want) || w.title.to_lowercase().contains(&want)).collect())
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
    /// The element behind each control, in the same order (control id n is els[n-1]).
    els: Vec<El>,
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
                    r.els.push(el.clone());
                }
            }
            logic::Kind::Text => {
                let mut t = bus.name(el);
                if t.is_empty() && bus.interfaces(el).iter().any(|i| i.ends_with(".Text")) {
                    t = bus.text(el);
                }
                let t = logic::clean(&t, 600);
                if !t.is_empty() && r.seen.insert(t.clone()) {
                    // Where it sits, only for the words that are kept (it is a round trip of its own).
                    let (y, x) = bus.extents(el).map(|e| (e[1] as f64, e[0] as f64)).unwrap_or((0.0, 0.0));
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
    walk_kids(bus, bus.children(el), depth + 1, r);
}

/// Each round trip to the bus waits on the app, so the children of a node are read side by side (up to PAR threads at a
/// time) and joined back in order: the result is the one a single walk would give, found sooner.
const PAR: usize = 8;
static ACTIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn fresh(started: Instant) -> Read {
    Read { controls: vec![], els: vec![], on: vec![], off: vec![], seen: Default::default(), nodes: 0, started }
}

/// What a side walk found, added after what's there (ids renumbered, repeated text dropped, the same limits).
fn absorb(r: &mut Read, c: Read) {
    r.nodes += c.nodes;
    for (mut ctl, el) in c.controls.into_iter().zip(c.els) {
        if r.controls.len() >= MAX_CONTROLS {
            break;
        }
        ctl["id"] = json!(r.controls.len() + 1);
        r.controls.push(ctl);
        r.els.push(el);
    }
    for t in c.on {
        if r.seen.insert(t.2.clone()) {
            r.on.push(t);
        }
    }
    for t in c.off {
        if r.seen.insert(t.1.clone()) {
            r.off.push(t);
        }
    }
}

fn walk_kids(bus: &Bus, kids: Vec<El>, depth: usize, r: &mut Read) {
    use std::sync::atomic::Ordering::SeqCst;
    if kids.len() < 2 || depth > 12 {
        for k in &kids {
            walk(bus, k, depth, r);
        }
        return;
    }
    let started = r.started;
    std::thread::scope(|sc| {
        let jobs: Vec<_> = kids
            .iter()
            .map(|k| {
                if ACTIVE.fetch_add(1, SeqCst) < PAR {
                    Some(sc.spawn(move || {
                        let mut c = fresh(started);
                        walk(bus, k, depth, &mut c);
                        ACTIVE.fetch_sub(1, SeqCst);
                        c
                    }))
                } else {
                    ACTIVE.fetch_sub(1, SeqCst);
                    None
                }
            })
            .collect();
        for (k, job) in kids.iter().zip(jobs) {
            match job {
                Some(j) => {
                    if let Ok(c) = j.join() {
                        if r.nodes < MAX_NODES && r.controls.len() < MAX_CONTROLS {
                            absorb(r, c);
                        }
                    }
                }
                None => walk(bus, k, depth, r),
            }
        }
    });
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str);
    match arg(0) {
        Some("enable") => match enable() {
            Ok(()) => out(json!({ "ok": true })),
            Err(e) => fail(&e),
        },
        // The window with the cursor right now, cheap enough to ask before every question: {"ok":true,"app","title"}.
        Some("front") => {
            let bus = Bus::open().unwrap_or_else(|e| fail(&e));
            let wins = windows(&bus);
            let focus = focused_title();
            match focus.as_deref().and_then(|f| wins.iter().find(|w| logic::same_title(&w.title, f))).or_else(|| wins.iter().find(|w| w.active)) {
                Some(w) => out(json!({ "ok": true, "app": w.app, "title": w.title })),
                None => out(json!({ "ok": true })),
            }
        }
        Some("windows") => {
            let _ = enable();
            let bus = Bus::open().unwrap_or_else(|e| fail(&e));
            let wins = windows(&bus);
            // The compositor's focused window when it names one of these, else the bus's own "active" flag.
            let focus = focused_title().filter(|f| wins.iter().any(|w| logic::same_title(&w.title, f)));
            let list: Vec<Value> = wins
                .iter()
                .map(|w| {
                    let mut v = json!({ "id": id_of(&w.el), "app": w.app, "title": w.title });
                    if focus.as_deref().map_or(w.active, |f| logic::same_title(&w.title, f)) {
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
            let mut r = Read { controls: vec![], els: vec![], on: vec![], off: vec![], seen: Default::default(), nodes: 0, started: Instant::now() };
            walk(&bus, &w.el, 0, &mut r);
            // Chromium builds its page tree only once accessibility is on, which the first read just switched on: wait and look again.
            if r.controls.is_empty() && r.on.is_empty() {
                std::thread::sleep(Duration::from_millis(1500));
                r = Read { controls: vec![], els: vec![], on: vec![], off: vec![], seen: Default::default(), nodes: 0, started: Instant::now() };
                walk(&bus, &w.el, 0, &mut r);
            }
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
        Some("act") => {
            let _ = enable();
            let bus = Bus::open().unwrap_or_else(|e| fail(&e));
            let Some(plan) = arg(1).and_then(|p| serde_json::from_str::<Value>(p).ok()) else { fail("usage: roamies-atspi act <json plan>") };
            let spec = if let Some(w) = plan["window"].as_str() { format!("window:{w}") } else if let Some(a) = plan["app"].as_str() { format!("app:{a}") } else { "front".into() };
            let wins = windows(&bus);
            let Some(w) = pick(&wins, &spec) else { fail("No matching window is open.") };
            match act::run(&bus, w, &plan["steps"]) {
                Ok(done) => {
                    // Let the page settle before it is read for the result: a click that opens another page changes the
                    // window's title a moment later, and reading at once showed the old page (so it was clicked again).
                    // The title is looked up again too: the one from before the steps was reported as "now showing".
                    let was = w.title.clone();
                    let (mut title, mut last) = (was.clone(), String::new());
                    for i in 0..10 {
                        std::thread::sleep(Duration::from_millis(250));
                        let now = windows(&bus).into_iter().find(|x| id_of(&x.el) == id_of(&w.el)).map(|x| x.title).unwrap_or_else(|| title.clone());
                        title = now.clone();
                        if now != was {
                            if now == last { break; }
                            last = now;
                        } else if i >= 1 {
                            break;
                        }
                    }
                    let mut r = Read { controls: vec![], els: vec![], on: vec![], off: vec![], seen: Default::default(), nodes: 0, started: Instant::now() };
                    walk(&bus, &w.el, 0, &mut r);
                    let mut info = win_info(w, &bus);
                    info["title"] = json!(title);
                    out(json!({ "ok": true, "done": done, "window": info, "controls": r.controls, "texts": logic::page_text(r.on, 3000) }))
                }
                Err(e) => fail(&e),
            }
        }
        _ => fail("usage: roamies-atspi windows | front | ui <front|app:NAME|window:ID> [controls] | act <json> | enable"),
    }
}
