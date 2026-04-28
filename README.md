# CatPane

A lightweight, cross-platform device log viewer with split panes and multi-window support. CatPane captures Android `adb logcat` streams, logs from booted iOS simulators, and wired physical iOS devices on macOS, while keeping a fast desktop UI and a headless MCP surface.

Built with Rust + [egui](https://github.com/emilk/egui) for minimal memory usage.

## Features

- **Split panes** — view multiple log streams side by side (vertical/horizontal splits)
- **Multi-window** — open additional windows with ⌘N / Ctrl+N
- **Android + iOS targets** — capture from connected Android devices, booted iOS simulators, and wired physical iOS devices on macOS
- **Boot iOS simulators** — launch an available iOS simulator directly from CatPane on macOS
- **Network throttling presets** — apply `unthrottled`, `edge`, `3g`, or `offline` to Android emulators **and physical Android devices** (via the bundled CatPane Helper VPN app); iOS simulator support remains feature-flagged off by default until CatPane ships a signed Network Extension build. Physical Android targets also accept fully custom shaping (`delay_ms`, `jitter_ms`, `loss_pct`, `downlink_kbps`, `uplink_kbps`).
- **Tag filters** — Android Studio-style syntax with include, exclude, regex, and per-tag levels:
  - `tag:Name` — include only this tag
  - `tag-:Exclude` — hide this tag
  - `tag~:Regex` — regex match on tag
  - `CallManagerService:V *:E` — show verbose logs for one tag while still showing errors everywhere else
  - Combine multiple: `tag:MyApp tag-:Verbose tag~:Net.*`
- **Tag autocomplete** — suggests from tags seen in the log stream
- **Package filter** — Android-only package filtering with autocomplete from running packages
- **Process / subsystem / category filters** — iOS filtering for unified logging fields where available
- **Clean iOS capture mode** — iOS streams default to a cleaner source-side mode, with a raw escape hatch when you need the full device firehose
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
- For iOS simulator throttling on macOS: macOS 15+, plus a CatPane build signed for Network Extension use so the bundled helper and app-proxy extension can be installed by the OS. Release packaging needs matching host + extension provisioning profiles. The iOS path is hidden by default until signed builds are available; set `CATPANE_ENABLE_IOS_NETWORK_THROTTLING=1` to re-enable it locally.

## Install with Homebrew

```sh
brew tap crockalet/catpane
brew install --cask --no-quarantine crockalet/catpane/catpane
```

Use `--no-quarantine` for now until CatPane is signed and notarized.

For beta builds:

```sh
brew tap crockalet/catpane
brew install --cask --no-quarantine crockalet/catpane/catpane@beta
```

Pre-release tags publish only `catpane@beta`. Stable releases continue to update `catpane`, and when a stable release overtakes the current beta, the tap removes `catpane@beta` until the next beta ships.

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
- `set_network_condition`
- `clear_network_condition`
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

Use `start_capture` to begin buffering logs for a device or iOS capture target, then query them with `get_logs`. iOS captures now default to a cleaner source-side mode; pass `clean: false` if you need the raw device stream. Explicit `process`, `text`, and simulator `predicate` scope still give the highest-signal results and should be preferred whenever you know them up front. Physical iOS captures also auto-stop after 15 minutes of MCP inactivity by default, so agents should keep polling `get_logs`, `get_status`, or `get_watch_matches` during an active debugging session and explicitly `stop_capture` when done. Set `CATPANE_IOS_DEVICE_IDLE_TIMEOUT_SECS=0` to disable that timeout or use another positive value to tune it. `clear_logs` resets the current observation window, `get_status` shows active captures plus scope/buffer warnings, `get_crashes` surfaces structured crash reports, and `create_watch` + `get_watch_matches` let agents pin high-signal lines so they survive main-buffer overflow. When the default retained watch buffer is still too small for a noisy repro, `create_watch` also accepts `retainedCapacity` to enlarge just that pinned side buffer. `get_logs` also supports iOS-specific `process`, `subsystem`, and `category` filters, though physical-device logs may not populate every field.

`set_network_condition` applies a network condition to a supported target. It accepts either a named preset (`unthrottled`, `edge`, `3g`, `offline`) or — on physical Android devices — a `custom` object with any of `delay_ms`, `jitter_ms`, `loss_pct`, `downlink_kbps`, `uplink_kbps`. Android emulators support presets only. iOS support is limited to simulators, currently targets the Simulator host app as a whole, depends on a properly signed macOS build with matching host + extension provisioning profiles, and is feature-flagged off by default until those signed builds are available.

### Throttling physical Android devices

Physical Android throttling is implemented by a small companion app (**CatPane Helper**, package `dev.catpane.helper`) that uses `VpnService` to capture device traffic into a local TUN and shapes it (delay, jitter, loss, bandwidth). CatPane controls the helper over `adb forward` + a local TCP socket — no rooting, no extra permissions on the host beyond `adb`.

What ships:

- The signed-debug APK is bundled into the macOS app at `Resources/catpane-helper.apk`. Set `CATPANE_HELPER_APK=/path/to/your.apk` to override the lookup.
- CatPane installs and updates the helper for you via `adb install -r -g`. The first time you apply throttling on a device, you must open the helper app on the phone once and tap **Grant VPN permission** (one-time, per device).
- Default LAN exclusion is **ADB host only** so wireless debugging keeps working while the VPN is up. Toggle it from the helper UI to `Full LAN` (excludes the whole local subnet) or `None` (full tunnel — may break wireless ADB).

Manual test checklist (no automated coverage on real devices):

1. Plug in or pair a physical Android device.
2. In CatPane → Network tab, select **3G**. Confirm the helper installs, the device prompts for VPN permission, and after granting, traffic on the device feels throttled.
3. With wireless debugging active, switch the helper to **Full LAN** and confirm CatPane loses the device (expected) and recovers when you switch back to **ADB host only**.
4. Apply **Custom** with `loss_pct: 100` and confirm the device behaves as offline.
5. Click **Clear** and confirm normal traffic resumes; the helper notification disappears.

Known limitations:

- Only one VPN can be active on Android at a time. If the user already has another VPN running, CatPane returns `already_another_vpn_active`.
- Some OEM system traffic (Samsung/Xiaomi telemetry, push services) bypasses VPN routing.
- The v1 packet pump drains shaped packets without re-injection. This gives correct semantics for `offline`, `delay`, `jitter`, and `loss` plus a downlink rate cap, but a full forwarder (v2) is needed for richer mixed-traffic scenarios.
- Wireless ADB pairing must complete *before* the VPN is started; once shaping is on with `adb_host_only` exclusion, existing connections survive.


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

Set `CATPANE_ENABLE_IOS_NETWORK_THROTTLING=1` to re-enable the iOS Simulator network-throttling path for local signed-build testing. Android emulator throttling is unaffected.

## License

MIT
