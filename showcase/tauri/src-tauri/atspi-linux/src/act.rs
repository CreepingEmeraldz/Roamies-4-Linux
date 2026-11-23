// Working an app over AT-SPI: press buttons and links (the control's own action, no pointer), fill fields, focus things.
// No pointer or keyboard is faked, so steps that need one (key, click_at, hover, scroll) say so and nothing runs.
use super::{walk, Bus, El, Read, Win};
use serde_json::Value;
use std::process::Command;
use std::time::{Duration, Instant};

const UNSUPPORTED: [&str; 3] = ["click_at", "hover", "drag"];
const PRESS: [&str; 10] = ["click", "press", "activate", "jump", "open", "toggle", "select", "check", "uncheck", "push"];

fn read(bus: &Bus, w: &Win) -> Read {
    let mut r = Read { controls: vec![], els: vec![], on: vec![], off: vec![], seen: Default::default(), nodes: 0, started: Instant::now() };
    walk(bus, &w.el, 0, &mut r);
    r
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

/// A browser builds a page's accessibility tree when first asked, so the first read can come back nearly empty (a
/// click then fails in a few milliseconds with "nothing is labelled"). A nearly empty page is read again, a few times.
fn read_built(bus: &Bus, w: &Win) -> Read {
    let mut r = read(bus, w);
    for _ in 0..3 {
        if r.nodes >= 40 {
            break;
        }
        std::thread::sleep(Duration::from_millis(400));
        r = read(bus, w);
    }
    r
}

/// The control whose label says `words`: the exact words first, then containing them; ones in view before ones scrolled away.
fn find(r: &Read, words: &str, roles: &[&str]) -> Option<usize> {
    let want = words.trim().to_lowercase();
    if want.is_empty() {
        return None;
    }
    let mut best: Option<(u8, usize)> = None;
    for (i, c) in r.controls.iter().enumerate() {
        if !roles.is_empty() && !roles.contains(&s(c, "role")) {
            continue;
        }
        let label = s(c, "label").to_lowercase();
        let rank = if label == want { 0 } else if label.contains(&want) { 2 } else { continue };
        let rank = rank + if c["offscreen"] == true { 1 } else { 0 };
        if best.map_or(true, |b| rank < b.0) {
            best = Some((rank, i));
        }
    }
    best.map(|b| b.1)
}

fn press(bus: &Bus, el: &El, role: &str) -> Result<(), String> {
    // A field is "clicked" by giving it the cursor; everything else has an action of its own.
    if matches!(role, "text field" | "text area" | "password field" | "combo box") {
        return focus(bus, el);
    }
    if let Some(p) = bus.proxy(el, "org.a11y.atspi.Action") {
        let acts: Vec<(String, String, String)> = p.call("GetActions", &()).unwrap_or_default();
        if !acts.is_empty() {
            let i = acts.iter().position(|a| PRESS.contains(&a.0.to_lowercase().as_str())).unwrap_or(0);
            let ok: bool = p.call("DoAction", &(i as i32,)).map_err(|e| e.to_string())?;
            return if ok { Ok(()) } else { Err("the app refused the click".into()) };
        }
    }
    focus(bus, el)
}

fn focus(bus: &Bus, el: &El) -> Result<(), String> {
    let p = bus.proxy(el, "org.a11y.atspi.Component").ok_or("that control can't take the cursor")?;
    let ok: bool = p.call("GrabFocus", &()).map_err(|e| e.to_string())?;
    if ok { Ok(()) } else { Err("the app wouldn't give it the cursor".into()) }
}


/// Keys go through `wtype` (the compositor's virtual keyboard), when it's installed: the accessibility bus can't press keys,
/// and Chromium fields offer no way to set their text either.
fn wtype(args: &[&str]) -> Result<(), String> {
    match Command::new("wtype").args(args).output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!("typing failed: {}", String::from_utf8_lossy(&o.stderr).trim().chars().take(160).collect::<String>())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err("typing and keys need wtype: sudo apt install wtype".into()),
        Err(e) => Err(e.to_string()),
    }
}

fn have_wtype() -> bool {
    Command::new("wtype").arg("-h").output().is_ok()
}

/// "ctrl+a", "return", "shift+tab", "cmd+l" (cmd is ctrl here) as wtype arguments.
pub fn key_args(key: &str) -> Result<Vec<String>, String> {
    let parts: Vec<String> = key.to_lowercase().split('+').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
    let (name, mods) = parts.split_last().ok_or("no key given")?;
    let mut out = vec![];
    let mut held = vec![];
    for m in mods {
        let m = match m.as_str() { "cmd" | "command" | "control" | "ctrl" => "ctrl", "option" | "alt" => "alt", "shift" => "shift", "super" | "win" | "meta" => "logo", o => return Err(format!("unknown modifier {o:?}")) };
        out.extend(["-M".to_string(), m.to_string()]);
        held.push(m);
    }
    let k = match name.as_str() {
        "return" | "enter" => "Return", "tab" => "Tab", "escape" | "esc" => "Escape", "space" => "space", "backspace" => "BackSpace", "delete" => "Delete",
        "up" => "Up", "down" => "Down", "left" => "Left", "right" => "Right", "home" => "Home", "end" => "End", "pageup" => "Prior", "pagedown" => "Next",
        o if o.chars().count() == 1 => o,
        o => return Err(format!("unknown key {o:?}")),
    };
    out.extend(["-k".to_string(), k.to_string()]);
    for m in held.iter().rev() {
        out.extend(["-m".to_string(), m.to_string()]);
    }
    Ok(out)
}

fn type_keys(text: &str) -> Result<(), String> {
    wtype(&["--", text])
}

fn set_text(bus: &Bus, el: &El, text: &str) -> Result<(), String> {
    let editable = bus.interfaces(el).iter().any(|i| i.ends_with(".EditableText"));
    if editable {
        let p = bus.proxy(el, "org.a11y.atspi.EditableText").ok_or("that isn't a field you can type into")?;
        let whole: Result<bool, _> = p.call("SetTextContents", &(text,));
        if !matches!(whole, Ok(true)) {
            let _ = focus(bus, el);
            let n = bus.text(el).chars().count() as i32;
            if n > 0 {
                let _: Result<bool, _> = p.call("DeleteText", &(0i32, n));
            }
            let _: Result<bool, _> = p.call("InsertText", &(0i32, text, text.chars().count() as i32));
        }
    } else {
        // Chromium fields: give the field the cursor, select what's there and type over it.
        focus(bus, el)?;
        std::thread::sleep(Duration::from_millis(150));
        let a: Vec<String> = key_args("ctrl+a")?;
        let a: Vec<&str> = a.iter().map(String::as_str).collect();
        wtype(&a)?;
        type_keys(text)?;
    }
    std::thread::sleep(Duration::from_millis(300));
    let now = bus.text(el);
    if now.trim() == text.trim() { Ok(()) } else { Err(format!("the field didn't take the text (it says {:?})", super::logic::clean(&now, 60))) }
}

fn insert_at_cursor(bus: &Bus, el: &El, text: &str) -> Result<(), String> {
    let p = bus.proxy(el, "org.a11y.atspi.EditableText").ok_or("that isn't a field you can type into")?;
    let at: i32 = bus.proxy(el, "org.a11y.atspi.Text").and_then(|t| t.get_property("CaretOffset").ok()).unwrap_or(-1);
    let at = if at < 0 { bus.text(el).chars().count() as i32 } else { at };
    let ok: bool = p.call("InsertText", &(at, text, text.chars().count() as i32)).map_err(|e| e.to_string())?;
    if ok { Ok(()) } else { Err("the field wouldn't take the text".into()) }
}

fn by_id(r: &Read, v: &Value) -> Option<usize> {
    let id = v["id"].as_u64()? as usize;
    (id >= 1 && id <= r.controls.len()).then(|| id - 1)
}

pub fn run(bus: &Bus, w: &Win, steps: &Value) -> Result<Vec<String>, String> {
    let steps = steps.as_array().ok_or("no steps")?;
    // Refuse before touching anything when a later step could never run.
    for st in steps {
        let d = s(st, "do");
        if UNSUPPORTED.contains(&d) {
            return Err(format!("\"{d}\" isn't possible in other apps on Linux yet (no pointer to borrow). Use click_text, click (id), fill, choose or scroll, and press a submit button instead of a key; nothing was done."));
        }
    }
    if steps.iter().any(|st| matches!(s(st, "do"), "key" | "scroll")) && !have_wtype() {
        return Err("Pressing keys and scrolling in other apps need wtype (sudo apt install wtype); nothing was done. Press a button with click_text instead.".into());
    }
    // The page is read only when a step needs to find something on it, and again only after something changed it:
    // reading a big page's tree takes seconds, and keys, typing and waits never look at it.
    let mut cache: Option<Read> = None;
    let wt = have_wtype();
    // Keys, typing and waits don't look at the page.
    let blind = |st: &Value| { let d = s(st, "do"); d == "key" || d == "wait" || (d == "type" && wt) };
    let mut done = vec![];
    for (n, st) in steps.iter().enumerate() {
        let d = s(st, "do");
        let pick = |r: &Read, roles: &[&str]| -> Result<usize, String> {
            if st["id"].is_u64() {
                return by_id(r, st).ok_or_else(|| "The page changed since read_window, so that id is out of date. Call read_window again and use the new ids.".to_string());
            }
            let words = if !s(st, "field").is_empty() { s(st, "field") } else { s(st, "text") };
            find(r, words, roles).ok_or_else(|| format!("Nothing here is labelled {:?} ({} controls could be read in this window). Call read_window to see the exact words; the label may not exist on this page.", words, r.controls.len()))
        };
        let mut changed = false;
        if !blind(st) && cache.is_none() {
            cache = Some(read_built(bus, w));
        }
        // Words that aren't on a small page may be on one still being built (the tree of a page is made when first asked
        // for): read it once more before saying they aren't there.
        if !blind(st) && !st["id"].is_u64() {
            let words = if !s(st, "field").is_empty() { s(st, "field") } else { s(st, "text") };
            if !words.is_empty() && cache.as_ref().map_or(false, |r| r.nodes < 300 && find(r, words, &[]).is_none()) {
                std::thread::sleep(Duration::from_millis(500));
                cache = Some(read(bus, w));
            }
        }
        match d {
            "click_text" | "click" => {
                let r = cache.as_ref().unwrap();
                let i = pick(r, &[])?;
                let (role, label) = (s(&r.controls[i], "role").to_string(), s(&r.controls[i], "label").to_string());
                press(bus, &r.els[i], &role).map_err(|e| format!("couldn't click {label:?}: {e}"))?;
                done.push(format!("click {label:?}"));
                changed = true;
            }
            "fill" | "set" => {
                let r = cache.as_ref().unwrap();
                let i = pick(r, &["text field", "text area", "combo box"])?;
                let label = s(&r.controls[i], "label").to_string();
                set_text(bus, &r.els[i], s(st, "text")).map_err(|e| format!("{label:?}: {e}"))?;
                done.push(format!("fill {label:?}"));
            }
            "focus" => {
                let r = cache.as_ref().unwrap();
                let i = pick(r, &[])?;
                focus(bus, &r.els[i]).map_err(|e| e.to_string())?;
                done.push(format!("focus {:?}", s(&r.controls[i], "label")));
            }
            "key" => {
                let a = key_args(s(st, "key"))?;
                let a: Vec<&str> = a.iter().map(String::as_str).collect();
                wtype(&a)?;
                done.push(format!("key {}", s(st, "key")));
                changed = true;
            }
            "scroll" => {
                let r = cache.as_ref().unwrap();
                // Scrolling is Page Up/Down (or Home/End) in the window in front, so a text field with the cursor would take them.
                if r.controls.iter().any(|c| c["focused"] == true && matches!(s(c, "role"), "text field" | "text area")) {
                    return Err("A text field has the cursor, so the keys would go into it. click_text something on the page first, then scroll.".into());
                }
                if st["id"].is_u64() || !s(st, "text").is_empty() {
                    let i = pick(r, &[])?;
                    let _ = focus(bus, &r.els[i]);
                    std::thread::sleep(Duration::from_millis(120));
                }
                let dir = if s(st, "direction").is_empty() { "down" } else { s(st, "direction") };
                let (page, edge) = match dir { "up" => ("Prior", "Home"), "left" => ("Left", "Home"), "right" => ("Right", "End"), _ => ("Next", "End") };
                if st["to_end"] == true {
                    // A feed that loads more at the bottom: keep going to the end a few times, giving it a moment each time.
                    for _ in 0..4 {
                        wtype(&["-k", edge])?;
                        std::thread::sleep(Duration::from_millis(700));
                    }
                } else {
                    for _ in 0..st["amount"].as_u64().unwrap_or(1).clamp(1, 10) {
                        wtype(&["-k", page])?;
                        std::thread::sleep(Duration::from_millis(150));
                    }
                }
                done.push(format!("scroll {dir}"));
                changed = true;
            }
            "type" if have_wtype() => {
                type_keys(s(st, "text"))?;
                done.push("type".into());
            }
            "type" => {
                let r = cache.as_ref().unwrap();
                let i = r.controls.iter().position(|c| c["focused"] == true && matches!(s(c, "role"), "text field" | "text area")).ok_or("No field has the cursor. fill a field by its label instead.")?;
                insert_at_cursor(bus, &r.els[i], s(st, "text"))?;
                done.push("type".into());
            }
            "choose" => {
                let mut r = cache.take().unwrap();
                if !s(st, "field").is_empty() {
                    let i = find(&r, s(st, "field"), &[]).ok_or_else(|| format!("Nothing here is labelled {:?}.", s(st, "field")))?;
                    let role = s(&r.controls[i], "role").to_string();
                    press(bus, &r.els[i], &role)?;
                    std::thread::sleep(Duration::from_millis(600));
                    r = read(bus, w);
                }
                let i = find(&r, s(st, "option"), &["menu item", "list item", "tree item", "tab", "radio", "button", "link"]).ok_or_else(|| format!("No option {:?} is showing.", s(st, "option")))?;
                let role = s(&r.controls[i], "role").to_string();
                press(bus, &r.els[i], &role).map_err(|e| format!("couldn't choose {:?}: {e}", s(st, "option")))?;
                done.push(format!("choose {:?}", s(st, "option")));
                cache = Some(r);
                changed = true;
            }
            "wait" => {
                std::thread::sleep(Duration::from_millis((st["seconds"].as_f64().unwrap_or(1.0).clamp(0.0, 10.0) * 1000.0) as u64));
                done.push("wait".into());
                changed = true;
            }
            other => return Err(format!("\"{other}\" isn't a step this app can do on Linux.")),
        }
        // The page moves after a click: read it again before the next step finds anything.
        if changed && n + 1 < steps.len() {
            // After a click the page needs a moment; between keys and typing it needn't.
            let quick = blind(st) && blind(&steps[n + 1]);
            std::thread::sleep(Duration::from_millis(if quick { 120 } else { 500 }));
            cache = None;
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::key_args;
    #[test]
    fn keys() {
        assert_eq!(key_args("return").unwrap(), ["-k", "Return"]);
        assert_eq!(key_args("cmd+a").unwrap(), ["-M", "ctrl", "-k", "a", "-m", "ctrl"]);
        assert_eq!(key_args("Shift+Tab").unwrap(), ["-M", "shift", "-k", "Tab", "-m", "shift"]);
        assert!(key_args("ctrl+").is_err() || key_args("hyper+a").is_err());
        assert!(key_args("flurb").is_err());
    }
}
