# CatPane – Copilot Instructions

CatPane is a native desktop logcat viewer built with Rust 2024 edition, egui/eframe, and Tokio.

## Build & Run

```sh
cargo run -p catpane-cli   # debug build (recommended during dev)
cargo build --release -p catpane-cli  # optimized release build
./target/release/catpane   # run release binary
```

Requires `adb` in PATH to use the app at runtime; not needed to build.

## Tests

```sh
cargo test                                # all tests
cargo test parse_logcat_line              # single test by name
cargo test --lib log_entry                # tests in a specific module
```

Tests currently exist only in `src/log_entry.rs`.

## Architecture

The app is split into workspace crates:

**Core (no egui dependency):**
- `catpane-core/src/log_entry.rs` – `LogEntry` struct and `parse_logcat_line()` which parses `logcat -v threadtime` output (format: `MM-DD HH:MM:SS.mmm  PID  TID LEVEL TAG: message`)
- `catpane-core/src/filter.rs` – `Filter` and `TagFilter` (Include/Exclude/Regex); all filtering logic lives here, including vendor noise suppression
- `catpane-core/src/adb.rs` – all ADB interaction: device listing, logcat spawning (`spawn_logcat`), device tracking (`spawn_device_tracker`), wireless pairing via mDNS, QR code generation

**UI (egui/eframe):**
- `catpane-ui/src/app.rs` – `App` struct (pane map + pane tree + device list + session/history persistence); session saved to `~/.config/catpane/session.json`, tag history to `~/.config/catpane/tag_history.txt`
- `catpane-ui/src/pane.rs` – `Pane` (per-pane state: entries, filtered indices, search, selection) and `PaneNode` (binary split tree of `PaneId`s); max 50,000 entries per pane with compaction
- `catpane-ui/src/ui/` – rendering only; `mod.rs` walks the `PaneNode` tree and renders each leaf with `draw_pane_panel`; sub-modules handle toolbar, tag bar, log area, search bar, dialogs

**CLI / MCP entrypoints:**
- `catpane-cli/src/main.rs` – Clap CLI and the real `catpane` binary; launches the GUI by default and supports `catpane mcp`
- `catpane-mcp/src/` – standalone MCP stdio server crate that remains usable independently of the GUI binary

**Data flow:**
`catpane_core::adb::spawn_logcat` → async Tokio task → `mpsc::channel` → `Pane::ingest_lines()` (called each frame) → `Pane::entries` + `Pane::filtered_indices` → egui rendering

## Key Conventions

**Pane identity:** Each pane has a `PaneId` (`u64`) from a global atomic counter. The pane tree (`PaneNode`) is a separate binary tree of IDs; the actual `Pane` data lives in `App::panes: HashMap<PaneId, Pane>`. Always look up panes by ID from the map — never store `&Pane` references across mutable borrows.

**Filtered indices:** `Pane::filtered_indices` is a `Vec<usize>` of indices into `Pane::entries` that pass the current filter. UI always operates on filtered indices, never raw entries directly. When filter changes, call `pane.rebuild_filtered()`. Search match indices (`search_match_indices`) are filtered-index positions within `filtered_indices`.

**Filter logic (filter.rs):** Vendor noise suppression is bypassed when any `Include` or `Regex` tag filter is active. Tag filter parsing (`parse_tag_filters`) uses the prefix syntax `tag:`, `tag-:`, `tag~:` — multiple filters space-separated. `Filter::matches` is the single entry point for all per-entry filtering.

**Async/sync boundary:** The app runs on a Tokio multi-thread runtime but egui is synchronous. ADB calls that must happen during a frame (device list, package list) use `rt_handle.block_on(...)` in `catpane-cli/src/main.rs`. Background tasks (logcat, device tracker) communicate back via `mpsc` channels read with `try_recv()` each frame.

**macOS occlusion:** `window_should_update` skips polling and rendering when the window is minimized or occluded, to avoid burning CPU. Repaint is requested at ~33ms intervals only when visible (`VISIBLE_REPAINT_INTERVAL`).

**Auto-fork:** `catpane-cli/src/main.rs` re-spawns itself as a detached process (with `CATPANE_FORKED=1`) so terminal launch doesn't keep the terminal attached.

**Pane depth limit:** Splits are limited to depth 2 (max 4 panes) — enforced in `App::split_pane`.

**Session serialization:** `App` serializes the pane tree and per-pane filter state to `Session` / `SessionTree` (index-based, not ID-based) using serde_json. IDs are ephemeral; indices are stable for serialization.

**Theme:** `OneDark` (dark) and `OneLight` (light) color constants are defined in `src/ui/theme.rs`. Theme is detected once at startup via `defaults read -g AppleInterfaceStyle` on macOS, then fixed for the session.

**muda menus:** Menu item IDs are plain string `MenuId`s (e.g. `"new_window"`, `"copy"`, `"find"`). Menu events are dispatched in `CatPaneApp::handle_menu_event`. On macOS, `menu.init_for_nsapp()` must be called inside the eframe creation closure.
