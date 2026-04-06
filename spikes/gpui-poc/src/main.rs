//! Phase 0 spike: GPUI uniform_list rendering 50k log rows with Tokio mpsc integration.
//!
//! Goals
//! ─────
//! 1. Confirm GPUI's `uniform_list` virtualises large log lists without egui's scroll bugs.
//! 2. Demonstrate Tokio ↔ GPUI boundary via std::sync::mpsc.
//! 3. Show GPU-accelerated rendering with OneDark palette.
//! 4. Provide a Go/No-Go data point for the full CatPane → GPUI migration.
//!
//! Architecture
//! ────────────
//! ┌─ Tokio thread ──────────────────────────────────────────────────┐
//! │  stream_entries() generates 50 000 burst entries, then streams  │
//! │  ~100/s. Entries go through std::sync::mpsc::Sender.            │
//! └──────────────────────────────────────────────────────────┬──────┘
//!                                                            │ mpsc
//! ┌─ GPUI main thread ───────────────────────────────────────▼──────┐
//! │  LogViewer::render() drains up to 500 entries per frame,        │
//! │  requests next frame via cx.notify() while data is flowing,     │
//! │  and calls scroll_handle.scroll_to_item() for auto-scroll.      │
//! │  uniform_list renders ONLY the visible rows (virtual scroll).   │
//! └─────────────────────────────────────────────────────────────────┘

use gpui::{
    App, Application, Bounds, Context, Render, ScrollStrategy, UniformListScrollHandle, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, rgb, size, uniform_list,
};
use std::sync::mpsc;
use std::time::Duration;

// ── Data model ────────────────────────────────────────────────────────────────

/// A single log line, mirroring the CatPane `LogEntry` structure.
#[derive(Clone)]
struct LogEntry {
    timestamp: String,
    level: char,
    tag: String,
    message: String,
}

// ── GPUI entity ───────────────────────────────────────────────────────────────

/// Root view entity.  Owned by GPUI; never shared across threads.
struct LogViewer {
    entries: Vec<LogEntry>,
    /// Receiver from the Tokio background thread.
    /// `std::sync::mpsc::Receiver<T>: Send` for `T: Send`, so it can live in a
    /// GPUI entity (which requires `'static` but not `Send`).
    rx: mpsc::Receiver<LogEntry>,
    auto_scroll: bool,
    /// GPUI's built-in scroll handle for uniform_list.
    /// Supports programmatic scroll-to-item with ScrollStrategy::Bottom.
    scroll_handle: UniformListScrollHandle,
}

impl LogViewer {
    /// Drain up to 500 entries per frame, mirroring `Pane::ingest_lines()`.
    fn drain_channel(&mut self) -> usize {
        let mut count = 0;
        for _ in 0..500 {
            match self.rx.try_recv() {
                Ok(entry) => {
                    self.entries.push(entry);
                    count += 1;
                }
                Err(_) => break,
            }
        }
        count
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────────

impl Render for LogViewer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Drain new entries from the Tokio side on every frame.
        let added = self.drain_channel();
        if added > 0 {
            if self.auto_scroll {
                // GPU-scroll to the last entry — no floating-point row offset math,
                // no disappearing-content bugs: GPUI handles layout internally.
                self.scroll_handle.scroll_to_item(
                    self.entries.len().saturating_sub(1),
                    ScrollStrategy::Bottom,
                );
            }
            // Request another render next frame to pick up any remaining entries.
            cx.notify();
        }

        let count = self.entries.len();

        div()
            .size_full()
            .bg(rgb(0x282c34)) // OneDark background
            .flex()
            .flex_col()
            // ── Status bar ──────────────────────────────────────────────
            .child(
                div()
                    .h(px(28.0))
                    .flex()
                    .items_center()
                    .px_4()
                    .gap_4()
                    .bg(rgb(0x21252b))
                    .text_color(rgb(0x5c6370))
                    .text_sm()
                    .child(format!(
                        "CatPane GPUI Spike  │  {} entries  │  uniform_list + Tokio mpsc",
                        count
                    )),
            )
            // ── Virtualised log list ─────────────────────────────────────
            .child(
                uniform_list(
                    "log-entries",
                    count,
                    // cx.processor() captures Entity<LogViewer> and calls the closure
                    // with `this: &mut LogViewer` only for the visible row range.
                    // Memory: only ~30 rows of DOM exist at a time regardless of count.
                    cx.processor(|this: &mut LogViewer, range: std::ops::Range<usize>, _window, _cx| {
                        let mut rows = Vec::with_capacity(range.len());
                        for i in range {
                            let entry = &this.entries[i];
                            let level_color = match entry.level {
                                'E' | 'F' => rgb(0xe06c75), // red
                                'W' => rgb(0xe5c07b),       // yellow
                                'I' => rgb(0x98c379),       // green
                                'D' => rgb(0x61afef),       // blue
                                _ => rgb(0x5c6370),         // dim grey
                            };
                            rows.push(
                                div()
                                    .id(i) // ElementId required for interactive elements
                                    .w_full()
                                    .h(px(20.0)) // fixed row height → uniform_list fast path
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .px_2()
                                    // Timestamp column
                                    .child(
                                        div()
                                            .w(px(140.0))
                                            .flex_shrink_0()
                                            .text_color(rgb(0x5c6370))
                                            .text_sm()
                                            .child(entry.timestamp.clone()),
                                    )
                                    // Level column (single char)
                                    .child(
                                        div()
                                            .w(px(14.0))
                                            .flex_shrink_0()
                                            .text_color(level_color)
                                            .text_sm()
                                            .child(entry.level.to_string()),
                                    )
                                    // Tag column
                                    .child(
                                        div()
                                            .w(px(130.0))
                                            .flex_shrink_0()
                                            .text_color(rgb(0x61afef))
                                            .text_sm()
                                            .child(entry.tag.clone()),
                                    )
                                    // Message column (fills remaining width)
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_color(rgb(0xabb2bf))
                                            .text_sm()
                                            .child(entry.message.clone()),
                                    ),
                            );
                        }
                        rows
                    }),
                )
                .track_scroll(self.scroll_handle.clone())
                .flex_1(), // fill the remaining height below the status bar
            )
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    // std::sync::mpsc bridges the Tokio background thread and the GPUI main thread.
    // Tokio writes; GPUI's render loop drains with try_recv() each frame.
    let (tx, rx) = mpsc::channel::<LogEntry>();

    // Spawn a dedicated OS thread that owns the Tokio runtime.
    // This keeps GPUI's main-thread executor fully independent of Tokio.
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build Tokio runtime");
        rt.block_on(stream_entries(tx));
    });

    Application::new().run(|cx: &mut App| {
        let viewer = cx.new(|_| LogViewer {
            entries: Vec::with_capacity(51_000), // matches CatPane's buffer capacity
            rx,
            auto_scroll: true,
            scroll_handle: UniformListScrollHandle::new(),
        });

        let bounds = Bounds::centered(None, size(px(1200.0), px(700.0)), cx);

        // Pass the existing entity as the root view (no double-allocation).
        let viewer_for_window = viewer.clone();
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, _cx| viewer_for_window,
        )
        .unwrap();

        cx.activate(true);
    });
}

// ── Tokio background log generator ───────────────────────────────────────────

/// Simulates adb logcat output: 50 000-entry burst followed by ~100 entries/s.
async fn stream_entries(tx: mpsc::Sender<LogEntry>) {
    const LEVELS: [char; 5] = ['V', 'D', 'I', 'W', 'E'];
    const TAGS: [&str; 7] = [
        "MainActivity",
        "NetworkManager",
        "DatabaseHelper",
        "UIController",
        "ServiceLocator",
        "EventBus",
        "CrashReporter",
    ];

    // Phase 1 — burst: fill the initial 50 000 entries as fast as possible.
    for i in 0usize..50_000 {
        let level = LEVELS[i % LEVELS.len()];
        let tag = TAGS[i % TAGS.len()];
        if tx.send(make_entry(i, level, tag, "Message")).is_err() {
            return;
        }
    }

    // Phase 2 — live stream: ~100 entries/second, indefinitely.
    let mut i = 50_000usize;
    loop {
        let level = LEVELS[i % LEVELS.len()];
        let tag = TAGS[i % TAGS.len()];
        if tx.send(make_entry(i, level, tag, "Live")).is_err() {
            break;
        }
        i += 1;
        // Sleep 1 s every 100 entries ≈ 100 entries/second.
        if i % 100 == 0 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

fn make_entry(i: usize, level: char, tag: &str, kind: &str) -> LogEntry {
    LogEntry {
        timestamp: format!(
            "03-{:02} {:02}:{:02}:{:02}.{:03}",
            (i / 86_400) % 28 + 1,
            (i / 3_600) % 24,
            (i / 60) % 60,
            i % 60,
            i % 1_000,
        ),
        level,
        tag: tag.to_string(),
        message: format!("{kind} #{i}: example log output from {tag}"),
    }
}
