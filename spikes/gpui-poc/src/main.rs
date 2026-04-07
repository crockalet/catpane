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
//!
//! UI Layout
//! ─────────
//! ┌───────────────────────────────────────────────────────────────┐
//! │ status bar (28px)                                             │
//! ├──────────────────────────────────────────────────┬──────┬────┤
//! │                                                  │  ▓   │    │  ← scrollbar
//! │  uniform_list (virtual, ~30 rows rendered)       │  ▓   │    │    thumb
//! │                                                  │      │    │
//! │                                           ╭────╮ │      │    │  ← stick-to-bottom
//! │                                           │  ↓ │ │      │    │    button (when !auto)
//! └───────────────────────────────────────────╰────╯─┴──────┴────┘
//!                                             list   scrbar  (6px)

use gpui::{
    App, Application, Bounds, ClickEvent, Context, Render, ScrollStrategy,
    ScrollWheelEvent, UniformListScrollHandle, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size, uniform_list,
};
use std::sync::mpsc;
use std::time::Duration;

// Layout constants
const STATUS_BAR_H: f32 = 28.0;
const ROW_HEIGHT: f32 = 20.0;
const SCROLLBAR_W: f32 = 6.0;
const SCROLLBAR_MIN_THUMB_H: f32 = 24.0;
// Stick-to-bottom button offset from the bottom-right of the log area.
const BUTTON_OFFSET_X: f32 = 50.0; // pulled left (negative margin) so it sits over the list
const BUTTON_OFFSET_Y: f32 = 48.0; // pulled up (negative margin) into the log area

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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Drain new entries from the Tokio side on every frame.
        let added = self.drain_channel();
        if added > 0 {
            if self.auto_scroll {
                // Index-based scroll — no f32 offset arithmetic, no disappearing-content bugs.
                self.scroll_handle.scroll_to_item(
                    self.entries.len().saturating_sub(1),
                    ScrollStrategy::Bottom,
                );
            }
            // Request another render next frame to pick up any remaining entries.
            cx.notify();
        }

        let count = self.entries.len();
        let auto_scroll = self.auto_scroll;

        // ── Scrollbar geometry ────────────────────────────────────────
        // `f32::from(Pixels)` is stable via `impl From<Pixels> for f32` in
        // gpui 0.2.2 (geometry.rs). The `Pixels` inner field is `pub(crate)`
        // so direct `.0` access is unavailable outside the crate.
        let viewport_h = f32::from(window.viewport_size().height) - STATUS_BAR_H;
        let total_h = count as f32 * ROW_HEIGHT;
        // `UniformListScrollHandle(pub Rc<RefCell<…>>)` exposes `.0` as a public
        // tuple field. `base_handle: ScrollHandle` and `ScrollHandle::offset()`
        // are both public in gpui 0.2.2 (elements/uniform_list.rs,
        // elements/scroll_handle.rs). This accesses current scroll position.
        let scroll_offset_y = f32::from(self.scroll_handle.0.borrow().base_handle.offset().y);

        let thumb_h = if total_h > viewport_h && total_h > 0.0 {
            (viewport_h / total_h * viewport_h).max(SCROLLBAR_MIN_THUMB_H)
        } else {
            viewport_h // whole track when everything fits
        };
        let max_scroll = (total_h - viewport_h).max(0.0);
        let thumb_top = if max_scroll > 0.0 {
            (scroll_offset_y.min(max_scroll) / max_scroll * (viewport_h - thumb_h)).max(0.0)
        } else {
            0.0
        };

        // ── Layout ────────────────────────────────────────────────────
        div()
            .size_full()
            .bg(rgb(0x282c34)) // OneDark background
            .flex()
            .flex_col()
            // ── Status bar ──────────────────────────────────────────────
            .child(
                div()
                    .h(px(STATUS_BAR_H))
                    .flex()
                    .items_center()
                    .px_4()
                    .gap_4()
                    .bg(rgb(0x21252b))
                    .text_color(rgb(0x5c6370))
                    .text_sm()
                    .child(format!(
                        "CatPane GPUI Spike  │  {} entries  │  uniform_list + Tokio mpsc{}",
                        count,
                        if auto_scroll { "  │  ⬇ auto" } else { "" }
                    )),
            )
            // ── Log area: list + scrollbar side-by-side ──────────────────
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    // Virtual log list
                    .child(
                        uniform_list(
                            "log-entries",
                            count,
                            // cx.processor() renders only the visible row slice.
                            // Memory: ~30 DOM rows regardless of count.
                            cx.processor(
                                |this: &mut LogViewer,
                                 range: std::ops::Range<usize>,
                                 _window,
                                 _cx| {
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
                                                .id(i)
                                                .w_full()
                                                .h(px(ROW_HEIGHT))
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .px_2()
                                                .child(
                                                    div()
                                                        .w(px(140.0))
                                                        .flex_shrink_0()
                                                        .text_color(rgb(0x5c6370))
                                                        .text_sm()
                                                        .child(entry.timestamp.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .w(px(14.0))
                                                        .flex_shrink_0()
                                                        .text_color(level_color)
                                                        .text_sm()
                                                        .child(entry.level.to_string()),
                                                )
                                                .child(
                                                    div()
                                                        .w(px(130.0))
                                                        .flex_shrink_0()
                                                        .text_color(rgb(0x61afef))
                                                        .text_sm()
                                                        .child(entry.tag.clone()),
                                                )
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
                                },
                            ),
                        )
                        .track_scroll(self.scroll_handle.clone())
                        .flex_1()
                        .h_full()
                        // Any manual scroll gesture disables auto-follow.
                        // The user re-enables it via the ↓ button.
                        .on_scroll_wheel(cx.listener(
                            |this, _event: &ScrollWheelEvent, _w, cx| {
                                if this.auto_scroll {
                                    this.auto_scroll = false;
                                    cx.notify();
                                }
                            },
                        )),
                    )
                    // ── Scrollbar ────────────────────────────────────────────
                    .child(
                        div()
                            .w(px(SCROLLBAR_W))
                            .h_full()
                            .bg(rgb(0x1e2127)) // dark track
                            .flex()
                            .flex_col()
                            // spacer above thumb
                            .child(div().h(px(thumb_top)))
                            // thumb
                            .child(
                                div()
                                    .w_full()
                                    .h(px(thumb_h))
                                    .rounded_md()
                                    .bg(rgb(0x4b5263)), // medium-grey thumb
                            ),
                    ),
            )
            // ── Stick-to-bottom button (floats over list when !auto_scroll) ─
            // Rendered last so it layers above the list and scrollbar.
            .when(!auto_scroll, |root| {
                root.child(
                    div()
                        .id("stick-to-bottom")
                        // Absolute position: bottom-right of the window, above scrollbar.
                        // Negative bottom margin lifts it into the log area.
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(32.0))
                        .h(px(32.0))
                        .ml(px(-BUTTON_OFFSET_X)) // pull left to sit over list, not below
                        .mt(px(-BUTTON_OFFSET_Y)) // pull up into the log area
                        .rounded_full()
                        .bg(rgb(0x61afef)) // OneDark blue
                        .text_color(rgb(0xffffff))
                        .text_base()
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.auto_scroll = true;
                            this.scroll_handle.scroll_to_item(
                                this.entries.len().saturating_sub(1),
                                ScrollStrategy::Bottom,
                            );
                            cx.notify();
                        }))
                        .child("↓"),
                )
            })
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
