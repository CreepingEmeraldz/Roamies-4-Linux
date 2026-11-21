## Reading and working other apps' windows (Linux)

The AI can now list your open windows and read what's in one: its buttons, links, fields and all its text, including what's scrolled out of view. This uses the accessibility bus (AT-SPI), as the macOS and Windows builds use their own accessibility APIs, so it works for GTK, Qt, Firefox and Chromium browsers such as Brave without a screenshot. Roamies turns accessibility on by itself when it reads, and a browser that was already running may need one restart to notice.

It can also press buttons and links, fill fields and pick options in other apps by their labels. Typing into web pages and pressing keys need `wtype` (`sudo apt install wtype`), which the Needs page now lists. There's no scrolling or pixel clicking in other apps. Delete, pay, send and sign-out clicks still ask you first.

## It answers the first time

The first read of a browser used to come back empty while the browser was building its page tree, so the AI said it failed and tried again. The helper now waits a moment and looks again before answering. Very long pages list their first 260 controls plus all the text, so the answer stays a sensible size. The AI is also told to use `read_window` for "what's on this window", not shell commands like `niri msg`.

Setup and shortcuts: [docs/LINUX.md](https://github.com/CreepingEmeraldz/Roamies-4-Linux/blob/linux/docs/LINUX.md)
