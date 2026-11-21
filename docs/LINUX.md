# Roamies on Linux (this fork)

This fork of [KenKaiii/Roamies](https://github.com/KenKaiii/Roamies) exists to make Roamies work properly on Linux,
especially on tiling Wayland compositors (niri, sway, Hyprland and the like), where upstream's overlay didn't behave.
It follows upstream's releases (the `linux` branch is upstream plus the Linux patches, merged in for each version) and
builds its own Linux packages on this repo's Releases page.

**What has been tested, honestly:** daily use on **niri** (Zorin OS 18.1, a Framework 13 laptop). **sway** has been run in a
headless session only. **Hyprland, river and other layer-shell compositors** should work the same way but haven't been
tried. GNOME and KDE use upstream's path (see below). Bug reports with your compositor and `echo $XDG_SESSION_TYPE` help.

## What this fork changes

- **A proper overlay on layer-shell compositors.** On niri, sway, Hyprland and river the overlay is a real layer-shell
  surface (above everything, click-through except where the Roamie, its bubbles and cards are) instead of an XWayland window.
  `ROAMIES_LAYER=1` forces it on, `ROAMIES_LAYER=0` forces it off.
- **Drawings land where they should** on screens whose layout doesn't start at 0,0.
- **Far less idle battery use:** a 30 fps cap, a resting state when nothing has happened for 15 seconds, click-area checks that
  slow down at rest, and sound devices that stay closed until a sound plays.
- **Ask about an area** (`roamies-ctl --snap`): drag a box over a frozen screenshot and ask about just that part. Pressing the key
  again while the box or the "What about this?" question is still open starts over, so a half-finished one never leaves the key dead.
- **Quicker shortcuts:** the app writes `~/.local/bin/roamies-ctl`, a tiny script that tells the running app what to do over
  D-Bus in a few milliseconds instead of starting a second copy of Roamies (which, from an AppImage, takes much longer).
- **Readable rate-limit errors** for Groq and OpenRouter ("Groq allows 8000 tokens/min; I sent 12880") and fixes for
  Groq, OpenRouter and Gemini providers.
- **Only the newest screenshot** stays in the history sent to the model, so a small model can't describe an old screen.
- **Updates and downloads come from this fork**, not upstream.

## How it compares with macOS and Windows

|                                  | macOS / Windows              | Linux, this fork                                              |
| -------------------------------- | ---------------------------- | ------------------------------------------------------------- |
| Roamie on screen, click-through  | Yes                          | Yes (layer-shell; XWayland or X11 elsewhere)                  |
| Typed and spoken questions       | Yes                          | Yes                                                           |
| Memory, reminders, cards         | Yes                          | Yes                                                           |
| Screenshots and pointing         | Yes                          | Yes, through your desktop's screenshot portal                 |
| Ask about an area                | Yes                          | Yes, with a key you bind (new, still being tuned on niri)     |
| Roamie follows the cursor        | Yes                          | Only over its own windows on Wayland (see below)              |
| Marks snap to the words on screen | Yes                          | Yes, if tesseract is installed (see Install)                  |
| Circle things while talking      | Yes                          | No (needs the cursor)                                         |
| Read windows (controls and text) | Yes                          | Yes, read only, over the accessibility bus (see below)        |
| Click and type in other apps     | Yes                          | By label, over the accessibility bus (keys need wtype)        |
| Shortcuts                        | Global keys                  | Keys you bind in your compositor (X11 sessions get real ones) |
| Updates itself                   | Yes                          | No: download the new AppImage from Releases                   |

### Why the Roamie can't follow the cursor everywhere on Wayland

A Wayland app hears about the pointer only while it is over one of the app's own windows, and there is no way to ask where
it is. The overlay is click-through, so it gets no pointer events. On niri a helper (`roamies-pointer`) can read the
position from the compositor's screen-cast frames, but that needs PipeWire 1.4.8 or newer, which Ubuntu 24.04 based systems
(Zorin 18, Mint 22) don't have yet. The app starts the helper by itself on niri and it stays idle until PipeWire is new enough.

### Reading windows

`read_window` and `list_windows` use AT-SPI, the accessibility bus that GTK, Qt, Firefox and Chromium apps publish to, through
a small bundled helper (`roamies-atspi`). The AI gets an app's buttons, links and fields and all of its text, including what's
scrolled out of view, without a screenshot. Roamies switches the accessibility flag on by itself when it reads. Chromium
browsers (Brave, Chrome) only build their page tree once it is on, so a browser that was already running may need a restart
the first time. Apps that publish nothing (some games, some Electron apps) give an empty list, and the AI falls back to a screenshot.

### Working other apps

`act_in_app` presses buttons and links with the control's own action (no pointer needed), fills ordinary fields, and
picks options, all by label. Typing into web page fields (Brave, Chrome) and pressing keys need `wtype` (`sudo apt install wtype`,
it uses the compositor's virtual keyboard, so it depends on your compositor allowing that). There is no scrolling, hovering or
clicking at pixels in other apps: `read_window` already shows the whole page. Delete, pay, send and sign-out clicks still ask you first.

## Install

1. Download `Roamies_<version>_amd64.AppImage` (or the `.deb` / `.rpm`) from this repo's
   [Releases](https://github.com/CreepingEmeraldz/Roamies-4-Linux/releases).
2. Put the AppImage somewhere permanent (for example `~/Roamies/Roamies.AppImage`) and `chmod +x` it.
   AppImages need FUSE 2 (`libfuse2` or `libfuse2t64`).
3. Run it once. It writes `~/.local/bin/roamies-ctl` and the setup guide opens.
4. To update, download the new AppImage over the old one. Your settings, keys and memory are in `~/.gg` and are kept.

For drawings that land on the right words, install tesseract (`sudo apt install tesseract-ocr`, or `doas apt install tesseract-ocr`).
Roamies reads the screenshot with it to find a button's or link's exact box, as the macOS and Windows helpers do, instead of
trusting the model's guess at pixel positions. Without it, drawings still work but are only as exact as the model's guess.

Needs glibc 2.38 or newer (Ubuntu 24.04, Fedora 39, Debian 13 or newer). The builds aren't signed, so the in-app updater
has nothing to install: it looks at this repo's releases, not upstream's.

## Bind your keys

On Wayland, apps can't grab keys, so you bind commands in your compositor. The panel's **Behavior** page lists them with a
**Copy** button. They all go through `roamies-ctl` (replace `/home/YOU`):

| Command                                     | What it does                                    |
| ------------------------------------------- | ----------------------------------------------- |
| `/home/YOU/.local/bin/roamies-ctl --summon` | Show or hide the Roamie                         |
| `/home/YOU/.local/bin/roamies-ctl --talk`   | Start listening; press again (or pause) to send |
| `/home/YOU/.local/bin/roamies-ctl --ask`    | Ask by typing                                   |
| `/home/YOU/.local/bin/roamies-ctl --snap`   | Ask about an area of the screen                 |
| `/home/YOU/.local/bin/roamies-ctl --list`   | Show your list                                  |
| `/home/YOU/.local/bin/roamies-ctl --panel`  | Open the panel                                  |

If Roamies isn't running, `roamies-ctl` starts it. It needs `gdbus` (the `libglib2.0-bin` package, on any desktop that has GTK).

**niri** (`~/.config/niri/config.kdl`):

```kdl
binds {
    Mod+Space { spawn "/home/YOU/.local/bin/roamies-ctl" "--summon"; }
    Mod+Shift+A { spawn "/home/YOU/.local/bin/roamies-ctl" "--snap"; }
}
```

**sway** (`~/.config/sway/config`): `bindsym $mod+Shift+a exec /home/YOU/.local/bin/roamies-ctl --snap`

**Hyprland** (`~/.config/hypr/hyprland.conf`): `bind = SUPER SHIFT, A, exec, /home/YOU/.local/bin/roamies-ctl --snap`

**GNOME / KDE:** add a custom shortcut with the command (Settings › Keyboard › Custom Shortcuts on GNOME, System Settings ›
Keyboard › Shortcuts › Add New › Command on KDE).

## Settings for troubleshooting

Set these in the environment of the app (a niri `spawn-at-startup` line or a `.desktop` file's `Exec=env VAR=1 ...`).

| Variable                       | What it does                                                                          |
| ------------------------------ | ------------------------------------------------------------------------------------- |
| `ROAMIES_LAYER=1` or `0`       | Force the layer-shell overlay on or off                                               |
| `ROAMIES_TRACE=1`              | Print the click areas to stderr: shows whether the overlay is blocking your clicks    |
| `ROAMIES_PROFILE=1`            | Log what the page spends its time on to `profile.log`                                 |
| `ROAMIES_DMABUF=0`             | Turns WebKit's GPU painting off (slower, but a fallback if the overlay misbehaves) |
| `ROAMIES_FULLPAINT=0`          | On layer-shell compositors the whole overlay is repainted each frame (stops stale-frame flicker with GPU painting); this turns that off |
| `WEBKIT_DISABLE_DMABUF_RENDERER=1` | A black or missing overlay, often on NVIDIA  |

Each screenshot logs how long the desktop portal, the picture load and the resize took (`screenshot_time` in the app's log),
which is the first thing to look at when screenshots feel slow.

## Releases and keeping up with upstream

- The **Release (Linux)** workflow (`.github/workflows/release-linux.yml`) builds the AppImage, `.deb` and `.rpm` and attaches
  them to a release named `v<version>-linux`. Start it from the Actions tab, or push a tag that begins with `v`.
- To follow upstream: `git fetch` upstream's tag, merge it into `linux`, fix any conflicts, run the tests, push, and run the
  workflow. There is one long-lived branch (`linux`); the old per-version branches are gone.
- Upstream's own release workflows were removed here: they need signing keys this fork doesn't have.
