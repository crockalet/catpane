//! Phase 0 spike: GPUI uniform_list rendering real adb logcat data with Tokio mpsc integration.
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
//! │  stream_logcat() runs `adb logcat -v threadtime -T 1000`,      │
//! │  parses lines, sends LogEntry via std::sync::mpsc::Sender.     │
//! └──────────────────────────────────────────────────────────┬──────┘
//!                                                            │ mpsc
//! ┌─ GPUI main thread ───────────────────────────────────────▼──────┐
//! │  LogViewer::render() drains up to 500 entries per frame.        │
//! │  A background polling task calls cx.refresh() every 32ms to     │
//! │  schedule platform repaints (cx.notify() inside render is a     │
//! │  no-op in GPUI 0.2.2).                                         │
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
    App, Application, Bounds, ClickEvent, Context, Render, ScrollDelta, ScrollStrategy,
    ScrollWheelEvent, UniformListScrollHandle, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size, uniform_list,
};
use std::sync::mpsc;
use std::time::Duration;
use tokio::io::AsyncBufReadExt;

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
    /// Returns (entries_added, channel_alive). When the sender is dropped,
    /// channel_alive becomes false and we stop requesting frames.
    fn drain_channel(&mut self) -> (usize, bool) {
        let mut count = 0;
        let mut alive = true;
        for _ in 0..500 {
            match self.rx.try_recv() {
                Ok(entry) => {
                    self.entries.push(entry);
                    count += 1;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    alive = false;
                    break;
                }
            }
        }
        (count, alive)
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────────

impl Render for LogViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Drain new entries from the Tokio side on every frame.
        let (added, _channel_alive) = self.drain_channel();
        if added > 0 && self.auto_scroll {
            // Index-based scroll — no f32 offset arithmetic, no disappearing-content bugs.
            self.scroll_handle.scroll_to_item(
                self.entries.len().saturating_sub(1),
                ScrollStrategy::Bottom,
            );
        }
        // Wakeup is handled by a background polling task spawned in main() that
        // calls cx.refresh() every 32ms. cx.notify() from within render() is a
        // no-op in GPUI 0.2.2 because WindowInvalidator drops it when
        // draw_phase != DrawPhase::None.

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
        // elements/div.rs). This accesses current scroll position.
        // GPUI scroll offsets are negative (0 at top, -max at bottom), so we
        // negate to get a positive value for scrollbar math.
        let scroll_offset_y = -f32::from(self.scroll_handle.0.borrow().base_handle.offset().y);

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
                        // Only disable auto-follow when user scrolls UP (away from
                        // bottom). Scrolling down at the bottom is harmless. GPUI
                        // convention: positive delta.y = scroll up, negative = down.
                        .on_scroll_wheel(cx.listener(
                            |this, event: &ScrollWheelEvent, _w, cx| {
                                let dy = match event.delta {
                                    ScrollDelta::Pixels(pt) => f32::from(pt.y),
                                    ScrollDelta::Lines(pt) => pt.y,
                                };
                                if this.auto_scroll && dy > 0.0 {
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
        rt.block_on(stream_logcat(tx));
    });

    Application::new().run(|cx: &mut App| {
        let viewer = cx.new(|_| LogViewer {
            entries: Vec::with_capacity(51_000),
            rx,
            auto_scroll: true,
            scroll_handle: UniformListScrollHandle::new(),
        });

        // Background wakeup task: cx.notify() inside render() is a no-op in GPUI
        // 0.2.2 (WindowInvalidator drops it when draw_phase != None). Instead we
        // poll from GPUI's foreground executor which runs outside the draw phase.
        cx.spawn(async |cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(32)).await;
                if cx.refresh().is_err() {
                    break;
                }
            }
        })
        .detach();

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

// ── Real adb logcat data source ───────────────────────────────────────────────

/// Spawn `adb logcat -v threadtime` and pipe parsed entries through the channel.
async fn stream_logcat(tx: mpsc::Sender<LogEntry>) {
    let mut child = match tokio::process::Command::new("adb")
        .args(["logcat", "-v", "threadtime", "-T", "1000"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            eprintln!("Failed to spawn adb logcat: {e}");
            eprintln!("Make sure adb is in PATH and a device/emulator is connected.");
            return;
        }
    };

    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            eprintln!("Failed to capture adb stdout");
            return;
        }
    };

    let mut lines = tokio::io::BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(entry) = parse_logcat_line(&line) {
            if tx.send(entry).is_err() {
                break;
            }
        }
    }
}

/// Parse a logcat `-v threadtime` line.
/// Format: `MM-DD HH:MM:SS.mmm  PID  TID LEVEL TAG     : message`
fn parse_logcat_line(line: &str) -> Option<LogEntry> {
    if line.len() < 33 {
        return None;
    }
    let timestamp = line.get(0..18)?.trim().to_string();
    let rest = line.get(18..)?.trim_start();

    // Skip PID
    let (_, rest) = split_ws(rest)?;
    // Skip TID
    let (_, rest) = split_ws(rest)?;
    // Level
    let (level_str, rest) = split_ws(rest)?;
    let level = level_str.chars().next()?;
    if !matches!(level, 'V' | 'D' | 'I' | 'W' | 'E' | 'F') {
        return None;
    }
    // Tag : message
    let (tag, message) = if let Some(colon_pos) = rest.find(": ") {
        (
            rest[..colon_pos].trim().to_string(),
            rest[colon_pos + 2..].to_string(),
        )
    } else {
        (rest.trim().to_string(), String::new())
    };

    Some(LogEntry {
        timestamp,
        level,
        tag,
        message,
    })
}

/// Split at the first whitespace, returning (word, rest_trimmed).
fn split_ws(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    let end = s.find(char::is_whitespace)?;
    Some((&s[..end], s[end..].trim_start()))
}
