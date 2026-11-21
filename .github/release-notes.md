## Caught up with Roamies v0.1.49

This build includes everything from the main Roamies v0.1.49: the E20 fix, NVIDIA NIM's 32k context, subagents that answer in text only, and the smoother Linux capsule. Details are in the [main release notes](https://github.com/KenKaiii/Roamies/releases/tag/v0.1.49).

## Smoother drawing on slow laptops

While the AI is drawing, the overlay now repaints at most 30 times a second on Linux instead of as fast as it can. A profile from a Framework 13 showed each frame taking 20-30 ms, too slow for 60 fps, so this halves the painting work.

## GPU painting is back on, without the flicker

WebKit's GPU painting is on by default again on layer-shell compositors (niri, sway, Hyprland, river), which makes a fullscreen card and drawing much smoother. It could leave stale copies of the overlay on screen, so Roamies now repaints the whole screen each frame in that mode, which stops the flicker and streaks. `ROAMIES_DMABUF=0` turns the GPU path off, and `ROAMIES_FULLPAINT=0` turns the full repaint off.

## Drawings and pointing land on the right words

Roamies reads the words on your screenshot with tesseract, so rings and marks go where the text actually is instead of where the model guessed. Install it once: `sudo apt install tesseract-ocr`. Without it, drawing still works but is less accurate.

## A Needs page

Settings now lists what Roamies needs on Linux (tesseract, the layer-shell library and so on), shows what's missing, and gives the install command to copy. Roamies also tells you once if tesseract is missing.

## Speed experiments for slow laptops

`ROAMIES_DPR=1` draws the overlay at a lower resolution, and `ROAMIES_GLOW=0` turns off the glow. Try them if drawings feel laggy, and tell me what helps.

## Thinking keeps up with the model

The thinking text no longer trails behind what the model has already written.

## Update notice

Roamies tells you when a newer Linux release is out and opens its page. It doesn't install it for you yet.

Setup and shortcuts: [docs/LINUX.md](https://github.com/CreepingEmeraldz/Roamies-4-Linux/blob/linux/docs/LINUX.md)
