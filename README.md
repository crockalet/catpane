# CatPane

A lightweight, cross-platform logcat viewer with split panes and multi-window support. Built as a fast alternative to Android Studio's logcat (too resource-heavy) and Aya (no split pane/multi-window).

Built with Rust + [egui](https://github.com/emilk/egui) for minimal memory usage.

## Features

- **Split panes** — view multiple logcat streams side by side (vertical/horizontal splits)
- **Multi-window** — open additional windows with ⌘N / Ctrl+N
- **Tag filters** — Android Studio-style syntax with include, exclude, and regex:
  - `tag:Name` — include only this tag
  - `tag-:Exclude` — hide this tag
  - `tag~:Regex` — regex match on tag
  - Combine multiple: `tag:MyApp tag-:Verbose tag~:Net.*`
- **Tag autocomplete** — suggests from tags seen in the log stream
- **Package filter** — filter by app package name with autocomplete from running packages
- **Log level filter** — minimum level selector (V/D/I/W/E/F)
- **Search** — ⌘F / Ctrl+F with match highlighting and navigation
- **Copy support** — click to select, shift+click for range, right-click context menu
- **Pause / Clear** — freeze the log stream or clear the buffer
- **Auto-scroll** — follows new logs, pauses when you scroll up
- **Session persistence** — saves pane layout, filters, and device selection on exit
- **Wireless debugging** — QR code pairing with mDNS auto-discovery
- **Auto-refresh devices** — monitors `adb track-devices` for live device updates
- **Vendor noise filter** — one-click toggle to hide common system tags
- **Dark / Light theme** — OneDark color scheme, auto-detects system preference

## Requirements

- [ADB](https://developer.android.com/tools/adb) (Android Debug Bridge) in your PATH
- Rust 2024 edition (1.85+)

## Build & Run

```sh
cargo run
# or for a release build:
cargo build --release
./target/release/catpane
```

## Keyboard Shortcuts

| Shortcut | Action |
|---|---|
| ⌘D / Ctrl+D | Split pane right |
| ⌘⇧D / Ctrl+Shift+D | Split pane down |
| ⌘W / Ctrl+W | Close pane |
| ⌘N / Ctrl+N | New window |
| ⌘F / Ctrl+F | Find in logs |
| ⌘C / Ctrl+C | Copy selected log lines |
| Tab | Cycle pane focus |
| F1 | Toggle keyboard shortcuts help |
| Right-click | Tag context menu (include/exclude/regex) |

## Wireless Debugging

Click the 📡 button next to the device selector (or use the menu: Window → Wireless Debug):

1. Click **Generate QR Code**
2. On your Android device: **Developer Options → Wireless debugging → Pair device with QR code**
3. Scan the QR — CatPane auto-discovers and pairs via mDNS

Manual pairing and connect options are available as fallbacks.

## Config

Session and tag history are saved to:
- `~/.config/catpane/session.json` — pane layout, filters, device selection
- `~/.config/catpane/tag_history.txt` — recent tag filter expressions

## License

MIT
