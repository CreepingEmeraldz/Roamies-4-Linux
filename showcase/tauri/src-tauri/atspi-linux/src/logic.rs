// The parts that need no accessibility bus: which roles count as controls and text, what to call them, and how the text
// is ordered and trimmed. `cargo test` runs these anywhere.

// AT-SPI state bits (the first of the two words GetState returns).
pub const ACTIVE: u64 = 1;
pub const CHECKED: u64 = 4;
pub const DEFUNCT: u64 = 6;
pub const EDITABLE: u64 = 7;
pub const FOCUSED: u64 = 12;
pub const MULTI_LINE: u64 = 17;
pub const SHOWING: u64 = 25;
pub const VISIBLE: u64 = 30;
pub const MANAGES_DESCENDANTS: u64 = 31;

pub fn has(state: u64, bit: u64) -> bool {
    state >> bit & 1 == 1
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    /// A control the model can name: the role word as the macOS and Windows helpers say it.
    Control(&'static str),
    /// Words on the page.
    Text,
    /// Holds others (a panel, a document): no line of its own, but its children are read.
    Container,
    /// Nothing to say, and nothing under it worth reading (a scroll bar, a separator).
    Skip,
}

pub fn is_window_role(role: &str) -> bool {
    matches!(role, "frame" | "window" | "dialog" | "alert" | "file chooser" | "color chooser" | "font chooser")
}

pub fn kind(role: &str) -> Kind {
    use Kind::*;
    match role {
        "push button" | "toggle button" | "button" => Control("button"),
        "check box" | "check menu item" | "switch" => Control("checkbox"),
        "radio button" | "radio menu item" => Control("radio"),
        "entry" | "text" | "password text" => Control("text field"),
        "combo box" => Control("dropdown"),
        "link" => Control("link"),
        "slider" => Control("slider"),
        "spin button" => Control("stepper"),
        "page tab" => Control("tab"),
        "menu item" => Control("menu item"),
        "list item" => Control("list item"),
        "tree item" => Control("tree item"),
        "label" | "static" | "heading" | "paragraph" | "caption" | "title bar" | "block quote" | "footnote" => Text,
        "scroll bar" | "separator" | "image" | "icon" | "animation" | "progress bar" | "ruler" | "tool tip" => Skip,
        _ => Container,
    }
}

/// The word for a control given what its state says (a text field you can type many lines into is a text area).
pub fn control_word(role: &str, word: &'static str, editable: bool, multi: bool) -> &'static str {
    match (role, word) {
        ("password text", _) => "password field",
        ("text", "text field") if !editable => "text field",
        (_, "text field") if multi => "text area",
        ("combo box", _) if editable => "combo box",
        _ => word,
    }
}

pub fn word_of(k: Kind) -> &'static str {
    if let Kind::Control(w) = k { w } else { "" }
}

pub fn num(v: f64) -> String {
    if v.fract() == 0.0 { format!("{}", v as i64) } else { format!("{v:.2}") }
}

/// One line, trimmed, at most `max` characters.
pub fn clean(s: &str, max: usize) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max { one } else { format!("{}…", one.chars().take(max).collect::<String>()) }
}

/// On-screen text lines (y, x, text), top to bottom, capped.
pub fn page_text(mut found: Vec<(f64, f64, String)>, cap: usize) -> Vec<String> {
    found.sort_by(|a, b| if (a.0 - b.0).abs() > 4.0 { a.0.total_cmp(&b.0) } else { a.1.total_cmp(&b.1) });
    let (mut out, mut total) = (vec![], 0);
    for (k, (_, _, t)) in found.iter().enumerate() {
        let n = t.chars().count();
        if total + n > cap {
            out.push(format!("…({} more lines further down this screen; read_window shows them all)", found.len() - k));
            break;
        }
        out.push(t.clone());
        total += n + 1;
    }
    out
}

/// Text scrolled out of view, in the order it was read (top to bottom). Long: the start and the end, with a note between.
pub fn offscreen_text(found: Vec<(f64, String)>, cap: usize) -> Vec<String> {
    let lines: Vec<String> = found.into_iter().map(|f| f.1).collect();
    if lines.iter().map(|l| l.chars().count() + 1).sum::<usize>() <= cap {
        return lines;
    }
    let (mut head, mut tail, mut h, mut t) = (vec![], vec![], 0, 0);
    for l in &lines {
        if h + l.chars().count() > cap / 2 {
            break;
        }
        head.push(l.clone());
        h += l.chars().count() + 1;
    }
    for l in lines.iter().rev() {
        if t + l.chars().count() > cap / 2 {
            break;
        }
        tail.insert(0, l.clone());
        t += l.chars().count() + 1;
    }
    let mid = lines.len() - head.len() - tail.len();
    head.push(format!("…({mid} more lines in between; point_at or click_text with their words scrolls there)"));
    head.extend(tail);
    head
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles() {
        assert_eq!(kind("push button"), Kind::Control("button"));
        assert_eq!(kind("label"), Kind::Text);
        assert_eq!(kind("panel"), Kind::Container);
        assert_eq!(kind("scroll bar"), Kind::Skip);
        assert!(is_window_role("frame") && !is_window_role("panel"));
    }
    #[test]
    fn words() {
        assert_eq!(control_word("password text", "text field", true, false), "password field");
        assert_eq!(control_word("entry", "text field", true, true), "text area");
        assert_eq!(control_word("combo box", "dropdown", true, false), "combo box");
        assert_eq!(control_word("combo box", "dropdown", false, false), "dropdown");
    }
    #[test]
    fn state_bits() {
        let s = (1u64 << SHOWING) | (1 << FOCUSED);
        assert!(has(s, SHOWING) && has(s, FOCUSED) && !has(s, EDITABLE));
    }
    #[test]
    fn trimming() {
        assert_eq!(clean("  a \n b  ", 10), "a b");
        assert_eq!(clean("abcdef", 3), "abc…");
        assert_eq!(num(3.0), "3");
        assert_eq!(num(0.5), "0.50");
    }
    #[test]
    fn text_order() {
        let t = page_text(vec![(30.0, 5.0, "b".into()), (10.0, 50.0, "a2".into()), (11.0, 5.0, "a1".into())], 100);
        assert_eq!(t, vec!["a1", "a2", "b"]);
    }
}
