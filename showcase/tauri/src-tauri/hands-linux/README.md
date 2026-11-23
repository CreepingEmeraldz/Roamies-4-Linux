# Linux hands helper

`roamies-hands` is the Linux implementation of the hands helper. It speaks the same
command language as `hands/main.swift` and `hands-win/`, and adds the Linux window
and pointer commands used by `window_control`, `pointer_drag` and `desktop_snapshot`.

It is a Python script because it leans on the desktop's own tools: Hyprland for
windows and workspaces, AT-SPI for controls and text, and uinput for real drags.

## What it needs

- Python 3.10 or newer.
- Hyprland, with `hyprctl` on `PATH`. Window and workspace commands are Hyprland only.
- PyGObject with the Atspi typelib (`python-gobject`, `at-spi2-core` on Arch). Without
  it, reading controls falls back to OCR and text, and clicking by visible words still works.
- `ydotool` for clicks and keys, and `wtype` for typing anything ydotool can't send as ASCII.
- `wl-clipboard` (`wl-paste`, `wl-copy`), ImageMagick (`convert`, `identify`), and
  `grim` plus `slurp` for screen and area captures.
- `tesseract` for OCR of screenshots. Without it, text is read from AT-SPI only.
- `lz4` (the Python module) is optional: it makes screenshot reads faster.
- Access to `/dev/uinput` (usually the `input` group) for `pointer_drag`. Without it,
  clicks and keys still work through ydotool.

On Arch:

```bash
sudo pacman -S python-gobject at-spi2-core ydotool wtype wl-clipboard imagemagick grim slurp tesseract
sudo usermod -aG input "$USER"   # then log out and back in, for pointer_drag
```

## How the app uses it

`build.sh` copies the script to `src-tauri/binaries/roamies-hands-<triple>` on Linux and
Tauri bundles it as an external binary. In dev it is also copied beside the debug app.
The app finds it next to its own executable and passes the path to the sidecar as
`ROAMIES_HANDS`.

The sidecar keeps its own safety checks: risky clicks still ask first, commands still go
through the command classifier, and `run_command` refuses `hyprctl` so window changes go
through `window_control` and are visible to the user.

## Checks

```bash
./roamies-hands trusted
./roamies-hands windows
./roamies-hands wm '{"action":"monitors"}'
./roamies-hands selftest-type
./roamies-hands selftest-drag   # needs /dev/uinput
```

The commands print one JSON object on stdout. Live progress lines start with `@` on
stderr.
