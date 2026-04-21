# CatPane

A lightweight, cross-platform device log viewer with split panes and multi-window support. CatPane captures Android `adb logcat` streams, logs from booted iOS simulators, and wired physical iOS devices on macOS, while keeping a fast desktop UI and a headless MCP surface.

Built with Rust + [egui](https://github.com/emilk/egui) for minimal memory usage.

## Features

- **Split panes** — view multiple log streams side by side (vertical/horizontal splits)
- **Multi-window** — open additional windows with ⌘N / Ctrl+N
- **Android + iOS targets** — capture from connected Android devices, booted iOS simulators, and wired physical iOS devices on macOS
- **Boot iOS simulators** — launch an available iOS simulator directly from CatPane on macOS
- **Tag filters** — Android Studio-style syntax with include, exclude, regex, and per-tag levels:
  - `tag:Name` — include only this tag
  - `tag-:Exclude` — hide this tag
  - `tag~:Regex` — regex match on tag
  - `CallManagerService:V *:E` — show verbose logs for one tag while still showing errors everywhere else
  - Combine multiple: `tag:MyApp tag-:Verbose tag~:Net.*`
- **Tag autocomplete** — suggests from tags seen in the log stream
- **Package filter** — Android-only package filtering with autocomplete from running packages
- **Process / subsystem / category filters** — iOS filtering for unified logging fields where available
- **Log level filter** — minimum level selector (V/D/I/W/E/F)
- **Search** — ⌘F / Ctrl+F with match highlighting and navigation
- **Copy support** — click to select, shift+click for range, right-click context menu
- **Pause / Clear** — freeze the log stream or clear the buffer
- **Auto-scroll** — follows new logs, pauses when you scroll up
- **Session persistence** — saves pane layout, filters, and device selection on exit
- **Wireless debugging** — QR code pairing with mDNS auto-discovery
- **Auto-refresh devices** — refreshes connected Android devices plus available iOS capture targets
- **Noise filter** — hides common Android system tags and iOS simulator device/system logs by default
- **Dark / Light theme** — OneDark color scheme, auto-detects system preference

## Requirements

- Rust 2024 edition (1.85+)
- For Android capture: [ADB](https://developer.android.com/tools/adb) (Android Debug Bridge) in your PATH
- For iOS simulator capture on macOS: Xcode / CoreSimulator tooling available via `xcrun`
- For physical iOS capture on macOS: `idevicesyslog` from `libimobiledevice` in your PATH

## Install with Homebrew

```sh
brew tap crockalet/catpane
brew install --cask --no-quarantine crockalet/catpane/catpane
```

Use `--no-quarantine` for now until CatPane is signed and notarized.

### Optional runtime dependencies

CatPane discovers connected devices automatically, but each platform needs its own tooling installed:

| Platform | Dependency | Install |
|---|---|---|
| Android | `adb` (Android Debug Bridge) | `brew install --cask android-platform-tools` |
| iOS Simulator | Xcode / Xcode Command Line Tools | Ships with Xcode |
| **iOS Physical Device** | **`idevicesyslog` from `libimobiledevice`** | **`brew install libimobiledevice`** |

> **Note:** Without `libimobiledevice`, physical iOS devices will not appear in the device list — CatPane silently skips detection when `idevicesyslog` is missing. Physical iOS capture is currently scoped to wired (USB) devices.

## Build & Run

```sh
cargo run
# or for a release build:
cargo build --release -p catpane-cli
./target/release/catpane
```

The workspace is split into `catpane-core`, `catpane-ui`, `catpane-cli`, and `catpane-mcp`. The `catpane-cli` crate owns the user-facing `catpane` binary.

If you use [`just`](https://github.com/casey/just), the repo includes a `justfile` with handy dev commands:

```sh
just run
just run-release
just rerun
just rerun-release
```

`rerun` and `rerun-release` clean only the `catpane-*` workspace crates before rebuilding, while keeping cached artifacts for unrelated dependencies.

## MCP Server

CatPane can also run as a headless stdio MCP server:

```sh
catpane mcp
```

If you are running from source during development:

```sh
cargo run -p catpane-cli -- mcp
```

Android captures require `adb` in your `PATH` at runtime. iOS simulator captures require Apple simulator tooling via `xcrun`. Physical iOS captures require `idevicesyslog` from `libimobiledevice`.

Available MCP tools:

- `list_devices`
- `get_logs`
- `clear_logs`
- `start_capture`
- `stop_capture`
- `get_status`
- `get_crashes`
- `create_watch`
- `list_watches`
- `get_watch_matches`
- `delete_watch`

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

Use `start_capture` to begin buffering logs for a device or iOS capture target, then query them with `get_logs`. For iOS captures, prefer `process`, `text`, and simulator `predicate` scope so irrelevant logs do not churn the main buffer. `clear_logs` resets the current observation window, `get_status` shows active captures plus scope/buffer warnings, `get_crashes` surfaces structured crash reports, and `create_watch` + `get_watch_matches` let agents pin high-signal lines so they survive main-buffer overflow. `get_logs` also supports iOS-specific `process`, `subsystem`, and `category` filters, though physical-device logs may not populate every field.

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

Set `CATPANE_LOG_BUFFER_CAPACITY` to override the default in-memory per-pane log retention (`50_000`).

Set `CATPANE_INITIAL_LOG_BACKLOG` to override how many recent Android logcat lines CatPane loads before switching to live streaming. The default is `2_000`, and it is capped at the in-memory buffer capacity.

## License

MIT
