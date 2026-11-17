// roamies-pointer: where the pointer is on a Wayland desktop that doesn't say (niri).
//
// A Wayland app only hears about the pointer while it is over one of its own windows, and niri has no IPC for it.
// What niri does offer is a screen capture whose frames carry the pointer as metadata (the cursor is not drawn in
// the picture; its position rides along in SPA_META_Cursor). So this helper starts a capture of every screen and
// throws the pictures away: it only reads the cursor's position from each frame and prints it, one JSON line per
// change, in global logical coordinates (the same space niri lays its screens out in):
//
//     {"x":812.5,"y":340.0}
//
// The capture is started through niri's own org.gnome.Mutter.ScreenCast D-Bus interface, the one the GNOME portal
// uses, so no portal and no permission prompt is involved. Frames are capped at 30 a second, and niri only makes a
// frame when something on that screen changed.
//
// Everything is reported on stderr when ROAMIES_POINTER_DEBUG=1 (or with --probe): the raw numbers niri sends and the
// position worked out from them. The helper ends when its stdin closes (Roamies quit) or the stream fails.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::time::Duration;

use pipewire as pw;
use pw::spa;
use pw::spa::pod::{ChoiceValue, Object, Pod, Property, PropertyFlags, Value};
use pw::spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Fraction, Id, SpaTypes};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, Value as ZValue};

const BUS: &str = "org.gnome.Mutter.ScreenCast";
const MAX_FPS: i32 = 30;

/// One screen, in niri's layout: where it sits and how big it is in logical pixels.
#[derive(Clone, Debug, PartialEq)]
struct Output {
    name: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// The screens from niri's reply to the "Outputs" request (`{"Ok":{"Outputs":{name: {"logical": {...}}}}}`).
/// A screen that is off has no `logical` and is left out.
fn parse_outputs(reply: &str) -> Result<Vec<Output>, String> {
    let v: serde_json::Value = serde_json::from_str(reply).map_err(|e| format!("niri's reply isn't JSON: {e}"))?;
    if let Some(err) = v.get("Err") {
        return Err(format!("niri said: {err}"));
    }
    let outs = v.pointer("/Ok/Outputs").and_then(|o| o.as_object()).ok_or("niri's reply has no Outputs")?;
    let mut list: Vec<Output> = outs
        .iter()
        .filter_map(|(name, o)| {
            let l = o.get("logical")?;
            let n = |k: &str| l.get(k).and_then(|x| x.as_f64());
            Some(Output { name: name.clone(), x: n("x")?, y: n("y")?, w: n("width")?, h: n("height")? })
        })
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(list)
}

fn niri_outputs() -> Result<Vec<Output>, String> {
    let path = std::env::var("NIRI_SOCKET").map_err(|_| "NIRI_SOCKET isn't set: this only works on niri".to_string())?;
    let mut s = UnixStream::connect(&path).map_err(|e| format!("can't reach niri at {path}: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    s.write_all(b"\"Outputs\"\n").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).map_err(|e| format!("no answer from niri: {e}"))?;
    parse_outputs(&line)
}

/// The pointer in global logical pixels from a position in a frame of `buf` pixels (width, height) of `out`. The frame
/// is the screen at its real resolution, so the logical size over the frame size is the scale. None when the position
/// is outside the frame (the pointer is on another screen, or hidden).
fn to_global(out: &Output, buf: (u32, u32), px: i32, py: i32) -> Option<(f64, f64)> {
    let (bw, bh) = (buf.0 as f64, buf.1 as f64);
    if bw < 1.0 || bh < 1.0 || px < 0 || py < 0 || px as f64 > bw || py as f64 > bh {
        return None;
    }
    Some((out.x + px as f64 * out.w / bw, out.y + py as f64 * out.h / bh))
}

/// What a stream knows while it runs.
struct Ud {
    out: Output,
    size: (u32, u32),
    last: Option<(i32, i32)>,
    frames: u64,
    debug: bool,
}

fn pod_bytes(v: &Value) -> Vec<u8> {
    spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), v).expect("pod").0.into_inner()
}

fn int_range(key: u32, default: i32, min: i32, max: i32) -> Property {
    Property {
        key,
        flags: PropertyFlags::empty(),
        value: Value::Choice(ChoiceValue::Int(Choice(ChoiceFlags::empty(), ChoiceEnum::Range { default, min, max }))),
    }
}

/// The kind of a Meta param is an Id, not an Int: with an Int, PipeWire quietly never allocates the metadata.
fn id(key: u32, v: u32) -> Property {
    Property { key, flags: PropertyFlags::empty(), value: Value::Id(Id(v)) }
}

fn int(key: u32, v: i32) -> Property {
    Property { key, flags: PropertyFlags::empty(), value: Value::Int(v) }
}

/// What the stream asks for once the format is known: buffers it can take, a header, and the cursor metadata (with
/// room for a small cursor bitmap, which nobody reads: a stream that doesn't ask gets no cursor at all).
fn buffer_params(w: u32, h: u32) -> Vec<Vec<u8>> {
    let stride = (w * 4) as i32;
    let buffers = Object {
        type_: SpaTypes::ObjectParamBuffers.as_raw(),
        id: spa::param::ParamType::Buffers.as_raw(),
        properties: vec![
            int_range(spa::sys::SPA_PARAM_BUFFERS_buffers, 4, 2, 8),
            int(spa::sys::SPA_PARAM_BUFFERS_blocks, 1),
            int(spa::sys::SPA_PARAM_BUFFERS_size, stride * h as i32),
            int(spa::sys::SPA_PARAM_BUFFERS_stride, stride),
            int(spa::sys::SPA_PARAM_BUFFERS_align, 16),
            int(spa::sys::SPA_PARAM_BUFFERS_dataType, (1 << spa::sys::SPA_DATA_MemFd) | (1 << spa::sys::SPA_DATA_MemPtr)),
        ],
    };
    let header = Object {
        type_: SpaTypes::ObjectParamMeta.as_raw(),
        id: spa::param::ParamType::Meta.as_raw(),
        properties: vec![
            id(spa::sys::SPA_PARAM_META_type, spa::sys::SPA_META_Header),
            int(spa::sys::SPA_PARAM_META_size, std::mem::size_of::<spa::sys::spa_meta_header>() as i32),
        ],
    };
    let cursor_size = |side: usize| (std::mem::size_of::<spa::sys::spa_meta_cursor>() + std::mem::size_of::<spa::sys::spa_meta_bitmap>() + side * side * 4) as i32;
    let cursor = Object {
        type_: SpaTypes::ObjectParamMeta.as_raw(),
        id: spa::param::ParamType::Meta.as_raw(),
        properties: vec![
            id(spa::sys::SPA_PARAM_META_type, spa::sys::SPA_META_Cursor),
            int_range(spa::sys::SPA_PARAM_META_size, cursor_size(64), cursor_size(1), cursor_size(256)),
        ],
    };
    [buffers, header, cursor].into_iter().map(|o| pod_bytes(&Value::Object(o))).collect()
}

/// The format the stream offers: any common 32-bit RGB at any size, at most MAX_FPS frames a second.
fn format_pod() -> Vec<u8> {
    use spa::param::format::{FormatProperties as F, MediaSubtype, MediaType};
    use spa::param::video::VideoFormat;
    let rect = |w, h| spa::utils::Rectangle { width: w, height: h };
    let frac = |n, d| Fraction { num: n, denom: d };
    let obj = spa::pod::object!(
        SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(F::MediaType, Id, MediaType::Video),
        spa::pod::property!(F::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(
            F::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::RGBx,
            VideoFormat::RGBA
        ),
        spa::pod::property!(F::VideoSize, Choice, Range, Rectangle, rect(1920, 1080), rect(1, 1), rect(16384, 16384)),
        spa::pod::property!(F::VideoFramerate, Choice, Range, Fraction, frac(0, 1), frac(0, 1), frac(120, 1)),
        spa::pod::property!(F::VideoMaxFramerate, Choice, Range, Fraction, frac(MAX_FPS as u32, 1), frac(1, 1), frac(MAX_FPS as u32, 1)),
    );
    pod_bytes(&Value::Object(obj))
}

/// Starts a capture of each screen through niri's ScreenCast D-Bus interface, with the cursor as metadata, and
/// returns the PipeWire node of each. The connection must stay open: the capture ends with it.
fn start_casts(outs: &[Output]) -> Result<(Connection, Vec<(Output, u32)>), String> {
    let e = |what: &str, err: zbus::Error| format!("{what}: {err}");
    let conn = Connection::session().map_err(|x| e("no session bus", x))?;
    let sc = Proxy::new(&conn, BUS, "/org/gnome/Mutter/ScreenCast", "org.gnome.Mutter.ScreenCast").map_err(|x| e("ScreenCast", x))?;
    let session: OwnedObjectPath = sc
        .call("CreateSession", &(HashMap::<&str, ZValue>::new(),))
        .map_err(|x| e("niri has no screen capture service (CreateSession)", x))?;
    let sess = Proxy::new(&conn, BUS, session.as_str().to_owned(), "org.gnome.Mutter.ScreenCast.Session").map_err(|x| e("Session", x))?;
    let (tx, rx) = mpsc::channel::<(Output, u32)>();
    for o in outs {
        // cursor-mode 2 = Metadata: the cursor is not painted into the picture, its position comes with each frame.
        let props = HashMap::<&str, ZValue>::from([("cursor-mode", ZValue::U32(2))]);
        let stream: OwnedObjectPath = sess.call("RecordMonitor", &(o.name.as_str(), props)).map_err(|x| e(&format!("RecordMonitor {}", o.name), x))?;
        let sp = Proxy::new(&conn, BUS, stream.as_str().to_owned(), "org.gnome.Mutter.ScreenCast.Stream").map_err(|x| e("Stream", x))?;
        // Subscribe before Start: the node arrives as a signal right after it.
        let signals = sp.receive_signal("PipeWireStreamAdded").map_err(|x| e("signal", x))?;
        let (tx, o) = (tx.clone(), o.clone());
        std::thread::spawn(move || {
            for msg in signals {
                if let Ok(node) = msg.body().deserialize::<u32>() {
                    let _ = tx.send((o, node));
                    break;
                }
            }
        });
    }
    sess.call::<_, _, ()>("Start", &()).map_err(|x| e("Start", x))?;
    drop(tx);
    let mut nodes = Vec::new();
    while nodes.len() < outs.len() {
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(n) => nodes.push(n),
            Err(_) => break,
        }
    }
    if nodes.is_empty() {
        return Err("niri started the capture but sent no PipeWire stream".into());
    }
    // Keep the session alive as long as the connection.
    std::mem::forget(sess);
    Ok((conn, nodes))
}

fn main() {
    let debug = std::env::var_os("ROAMIES_POINTER_DEBUG").is_some() || std::env::args().any(|a| a == "--probe");
    if let Err(msg) = run(debug) {
        eprintln!("roamies-pointer: {msg}");
        std::process::exit(1);
    }
}

fn run(debug: bool) -> Result<(), String> {
    let outs = niri_outputs()?;
    if outs.is_empty() {
        return Err("niri reports no screens".into());
    }
    if debug {
        eprintln!("screens: {outs:?}");
    }
    let (_conn, nodes) = start_casts(&outs)?;
    if debug {
        eprintln!("capture nodes: {:?}", nodes.iter().map(|(o, n)| (&o.name, n)).collect::<Vec<_>>());
    }

    // Roamies quitting closes our stdin: that's the end.
    std::thread::spawn(|| {
        let mut sink = [0u8; 64];
        let mut stdin = std::io::stdin();
        while matches!(stdin.read(&mut sink), Ok(n) if n > 0) {}
        std::process::exit(0);
    });

    pw::init();
    let mainloop = pw::main_loop::MainLoop::new(None).map_err(|e| format!("pipewire: {e}"))?;
    let context = pw::context::Context::new(&mainloop).map_err(|e| format!("pipewire: {e}"))?;
    let core = context.connect(None).map_err(|e| format!("can't reach pipewire: {e}"))?;

    let mut keep = Vec::new();
    for (out, node) in nodes {
        let stream = pw::stream::Stream::new(
            &core,
            "roamies-pointer",
            pw::properties::properties! {
                *pw::keys::MEDIA_TYPE => "Video",
                *pw::keys::MEDIA_CATEGORY => "Capture",
                *pw::keys::MEDIA_ROLE => "Screen",
            },
        )
        .map_err(|e| format!("pipewire stream: {e}"))?;
        let ud = Ud { out, size: (0, 0), last: None, frames: 0, debug };
        let listener = stream
            .add_local_listener_with_user_data(ud)
            .state_changed(|_, ud, old, new| {
                if ud.debug {
                    eprintln!("[{}] stream {old:?} -> {new:?}", ud.out.name);
                }
                if let pw::stream::StreamState::Error(msg) = new {
                    eprintln!("roamies-pointer: stream {} failed: {msg}", ud.out.name);
                    std::process::exit(1);
                }
            })
            .param_changed(|stream, ud, id, param| {
                let Some(param) = param else { return };
                if id != spa::param::ParamType::Format.as_raw() {
                    return;
                }
                let mut info = spa::param::video::VideoInfoRaw::default();
                if info.parse(param).is_err() {
                    return;
                }
                ud.size = (info.size().width, info.size().height);
                if ud.debug {
                    eprintln!("[{}] format {:?} {}x{} max {}/{}", ud.out.name, info.format(), ud.size.0, ud.size.1, info.max_framerate().num, info.max_framerate().denom);
                }
                let bytes = buffer_params(ud.size.0, ud.size.1);
                let mut pods: Vec<&Pod> = bytes.iter().filter_map(|b| Pod::from_bytes(b)).collect();
                if let Err(e) = stream.update_params(&mut pods) {
                    eprintln!("roamies-pointer: can't ask for cursor metadata: {e}");
                }
            })
            .process(|stream, ud| unsafe { process(stream, ud) })
            .register()
            .map_err(|e| format!("pipewire listener: {e}"))?;
        let fmt = format_pod();
        let mut params = [Pod::from_bytes(&fmt).ok_or("bad format pod")?];
        stream
            .connect(spa::utils::Direction::Input, Some(node), pw::stream::StreamFlags::AUTOCONNECT, &mut params)
            .map_err(|e| format!("pipewire connect: {e}"))?;
        keep.push((stream, listener));
    }
    mainloop.run();
    Ok(())
}

/// One frame: ignore the picture, read the cursor metadata. The stream owns the buffer; it goes straight back.
unsafe fn process(stream: &pw::stream::StreamRef, ud: &mut Ud) {
    let raw = stream.as_raw_ptr();
    let b = pw::sys::pw_stream_dequeue_buffer(raw);
    if b.is_null() {
        return;
    }
    let sb = (*b).buffer;
    ud.frames += 1;
    if ud.debug && (ud.frames <= 3 || ud.frames % 300 == 0) && !sb.is_null() {
        let kinds: Vec<u32> = (0..(*sb).n_metas as usize).map(|i| (*(*sb).metas.add(i)).type_).collect();
        eprintln!("[{}] frame {} with metas {kinds:?} (cursor is {})", ud.out.name, ud.frames, spa::sys::SPA_META_Cursor);
    }
    if !sb.is_null() && !(*sb).metas.is_null() {
        for i in 0..(*sb).n_metas as usize {
            let m = (*sb).metas.add(i);
            if (*m).type_ == spa::sys::SPA_META_Cursor && !(*m).data.is_null() && (*m).size as usize >= std::mem::size_of::<spa::sys::spa_meta_cursor>() {
                let c = (*m).data as *const spa::sys::spa_meta_cursor;
                cursor(ud, (*c).id, (*c).position.x, (*c).position.y, (*c).hotspot.x, (*c).hotspot.y);
            }
        }
    }
    pw::sys::pw_stream_queue_buffer(raw, b);
}

fn cursor(ud: &mut Ud, id: u32, px: i32, py: i32, hx: i32, hy: i32) {
    let global = if id == 0 { None } else { to_global(&ud.out, ud.size, px, py) };
    if ud.debug {
        eprintln!("[{}] cursor id={id} pos=({px},{py}) hotspot=({hx},{hy}) frame={}x{} -> {global:?}", ud.out.name, ud.size.0, ud.size.1);
    }
    let Some((x, y)) = global else { return };
    if ud.last == Some((px, py)) {
        return;
    }
    ud.last = Some((px, py));
    let mut out = std::io::stdout().lock();
    if writeln!(out, "{{\"x\":{x:.1},\"y\":{y:.1}}}").and_then(|_| out.flush()).is_err() {
        std::process::exit(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLY: &str = r#"{"Ok":{"Outputs":{
        "eDP-1":{"name":"eDP-1","make":"BOE","logical":{"x":0,"y":0,"width":1736,"height":1157,"scale":1.3,"transform":"Normal"}},
        "HDMI-A-1":{"name":"HDMI-A-1","logical":{"x":1736,"y":-300,"width":1920,"height":1080,"scale":1.0,"transform":"Normal"}},
        "DP-9":{"name":"DP-9","logical":null}}}}"#;

    #[test]
    fn screens_come_from_nirs_reply_and_a_screen_that_is_off_is_left_out() {
        let outs = parse_outputs(REPLY).unwrap();
        assert_eq!(outs.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(), ["HDMI-A-1", "eDP-1"]);
        assert_eq!(outs[0], Output { name: "HDMI-A-1".into(), x: 1736.0, y: -300.0, w: 1920.0, h: 1080.0 });
    }

    #[test]
    fn an_error_from_niri_is_reported() {
        assert!(parse_outputs(r#"{"Err":"nope"}"#).unwrap_err().contains("nope"));
        assert!(parse_outputs("not json").is_err());
    }

    #[test]
    fn a_frame_position_becomes_a_global_logical_one() {
        let outs = parse_outputs(REPLY).unwrap();
        let edp = outs.iter().find(|o| o.name == "eDP-1").unwrap();
        // The panel is 2256x1504 real pixels at scale 1.3: the middle of the frame is the middle of the layout.
        let (x, y) = to_global(edp, (2256, 1504), 1128, 752).unwrap();
        assert!((x - 868.0).abs() < 0.01 && (y - 578.5).abs() < 0.01, "{x} {y}");
        // Another screen is offset by where it sits.
        let hdmi = outs.iter().find(|o| o.name == "HDMI-A-1").unwrap();
        assert_eq!(to_global(hdmi, (1920, 1080), 10, 20), Some((1746.0, -280.0)));
    }

    #[test]
    fn a_position_outside_the_frame_is_nothing() {
        let o = Output { name: "x".into(), x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        assert_eq!(to_global(&o, (100, 100), -1, 5), None);
        assert_eq!(to_global(&o, (100, 100), 5, 101), None);
        assert_eq!(to_global(&o, (0, 0), 0, 0), None, "no frame size yet");
    }
}
