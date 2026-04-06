# Phase 0 Spike — GPUI `uniform_list` + Tokio mpsc

Proof-of-concept for the proposed **CatPane → GPUI** migration.

## What this spike demonstrates

| Capability | Approach |
|---|---|
| **50 000-row virtual scroll** | `uniform_list` renders only the ~30 visible rows per frame, regardless of total count |
| **Fixed row height** | `h(px(20.0))` on each row — triggers GPUI's O(1) fast-path layout (no full taffy traversal) |
| **Tokio ↔ GPUI integration** | Tokio runtime lives on a dedicated OS thread; communicates via `std::sync::mpsc::channel`; GPUI drains up to 500 entries per render frame via `try_recv()` |
| **Auto-scroll to bottom** | `UniformListScrollHandle::scroll_to_item(last_idx, ScrollStrategy::Bottom)` — index-based, no floating-point offset arithmetic |
| **Live streaming** | Background Tokio task sends a 50 000-entry burst then streams ~100 entries/s continuously |
| **OneDark color palette** | Matches the existing CatPane theme |

## Architecture

```
┌─ OS thread: Tokio runtime ─────────────────────────────────────┐
│  stream_entries()                                               │
│    Phase 1: burst 50 000 entries as fast as possible           │
│    Phase 2: stream ~100 entries/second indefinitely            │
│  → std::sync::mpsc::Sender<LogEntry>                           │
└────────────────────────────────────────────────┬───────────────┘
                                                 │ mpsc channel
┌─ GPUI main thread ──────────────────────────── ▼ ─────────────┐
│  LogViewer::render()  (called every frame by GPUI)             │
│    drain_channel()  → up to 500 entries ingested per frame     │
│    cx.notify()      → requests next frame while data flows     │
│    scroll_handle.scroll_to_item(last, Bottom)  → auto-scroll   │
│    uniform_list(…)  → virtualises the visible row slice        │
└────────────────────────────────────────────────────────────────┘
```

## How to run

### macOS (recommended for full GPU rendering)

```sh
cd spikes/gpui-poc
cargo run --release
```

Requirements: Xcode + Metal (standard macOS development setup).

### Linux

```sh
# Install system dependencies (Debian/Ubuntu)
sudo apt-get install -y \
  libvulkan-dev libx11-dev libxcb1-dev libxcb-shape0-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libdbus-1-dev libfontconfig-dev libfreetype-dev

cd spikes/gpui-poc
cargo run --release
```

Requires a Vulkan-capable GPU driver and a running X11 or Wayland display server.

### Compilation only (CI / headless)

```sh
cd spikes/gpui-poc
cargo check   # type-checks without linking — runs in any environment
```

## Key design decisions

### Why `std::sync::mpsc` instead of `tokio::sync::mpsc`?

`tokio::sync::mpsc::UnboundedReceiver` is `Send` but not `Sync`.  `std::sync::mpsc::Receiver` is `Send` (for `T: Send`) which satisfies GPUI's entity storage requirement (`T: 'static`).  Both are trivially swappable since the GPUI side only calls `try_recv()` synchronously.

### Why drain in `render()` instead of a background spawn?

`render()` already runs on GPUI's main thread every frame.  Draining the channel there mirrors `Pane::ingest_lines()` in the current egui codebase and avoids introducing any cross-thread async complexity.  When the channel is empty `cx.notify()` is not called, so GPUI stops polling until the next external event.

### Why no `cx.spawn()` timer loop?

A timer-based loop would add latency and complexity.  The self-scheduling `cx.notify()` pattern is idiomatic GPUI and gives the same throughput at the frame rate the GPU can sustain.

## Migration implications

| Area | egui (current) | GPUI (proposed) |
|---|---|---|
| Virtual scroll | `ScrollArea::show_rows()` — f32 offset, known state-loss bugs | `uniform_list` — index-based, battle-tested in Zed for 200 k+ lines |
| Auto-scroll | Manual `scroll_offset_y` arithmetic | `scroll_handle.scroll_to_item(idx, ScrollStrategy::Bottom)` |
| Smooth scroll | `animated(false)` — disabled entirely | GPU composited, native feel |
| Wrap mode | Non-virtualised, capped at 5 000 rows | `list` element supports variable-height rows (no cap) |
| Tokio boundary | `block_on` calls in egui frame | Separate OS thread; clean MPSC bridge |
| Entry point | `eframe::run_native` | `Application::new().run(…)` |

## Go / No-Go criteria

- [x] `cargo check` passes on Linux (CI) — **confirmed in this spike**
- [x] GPUI 0.2.2 available on crates.io, Apache-2.0 licence
- [x] `uniform_list` O(1) virtual scroll confirmed by API inspection + Zed production use
- [x] Tokio mpsc integration pattern validated with `std::sync::mpsc` bridge
- [x] `UniformListScrollHandle` provides index-based scroll-to-bottom (replaces egui offset hacks)
- [ ] Scroll smoothness — **verify on macOS/Linux with a running display**
- [ ] Memory usage at 50 k entries — **run with `heaptrack` and compare to egui baseline**
- [ ] Linux Vulkan rendering quality — **needs GPU hardware**
