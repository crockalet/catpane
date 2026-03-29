# CatPane

A lightweight, cross-platform logcat viewer with split panes and multi-window support. Built as a fast alternative to Android Studio's logcat (too resource-heavy) and Aya (no split pane/multi-window).

Built with Rust + [egui](https://github.com/emilk/egui) for minimal memory usage.

## Features

- **Split panes** — view multiple logcat streams side by side (vertical/horizontal splits)
- **Multi-window** — open additional windows with ⌘N / Ctrl+N
- **Tag filters** — Android Studio-style syntax with include, exclude, regex, and per-tag levels:
  - `tag:Name` — include only this tag
  - `tag-:Exclude` — hide this tag
  - `tag~:Regex` — regex match on tag
  - `CallManagerService:V *:E` — show verbose logs for one tag while still showing errors everywhere else
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

## Install with Homebrew

```sh
brew tap crockalet/catpane
brew install --cask crockalet/catpane/catpane
brew install --cask android-platform-tools
```

CatPane needs `adb` at runtime, so install `android-platform-tools` as well if you do not already have it.

## Build & Run

```sh
cargo run
# or for a release build:
cargo build --release
./target/release/catpane
```

## MCP Server

CatPane can also run as a headless stdio MCP server:

```sh
catpane mcp
```

If you are running from source during development:

```sh
cargo run -- mcp
```

This mode still requires `adb` in your `PATH` at runtime.

Available MCP tools:

- `list_devices`
- `get_logs`
- `clear_logs`
- `start_capture`
- `stop_capture`
- `get_status`

Example MCP client config using stdio transport:

```json
{
  "mcpServers": {
    "catpane-logcat": {
      "command": "/absolute/path/to/catpane",
      "args": ["mcp"]
    }
  }
}
```

Use `start_capture` to begin buffering logs for a device, then query them with `get_logs`. `clear_logs` resets the current buffer without stopping capture, and `get_status` shows active captures plus buffer state.

### Agent skill via `vercel-labs/skills`

CatPane also ships a reusable `catpane-logcat` agent skill in `.agents/skills/`. You can install it from your local checkout with [`vercel-labs/skills`](https://github.com/vercel-labs/skills):

```sh
# Install for GitHub Copilot in the current project
npx skills add . --agent github-copilot --skill catpane-logcat

# Or install it globally
npx skills add /absolute/path/to/catpane --agent github-copilot --skill catpane-logcat --global
```

Replace `github-copilot` with another supported agent if needed. This stays fully local to your machine; no hosted service is required. The skill teaches the agent when to call `get_status`, `start_capture`, `get_logs`, and related tools, but you still need the local CatPane MCP server configured as shown above.

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
