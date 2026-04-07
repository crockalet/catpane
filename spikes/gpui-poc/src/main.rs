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
    prelude::*, px, rems, rgb, rgba, size, uniform_list,
};
use std::sync::mpsc;
use std::time::Duration;
use tokio::io::AsyncBufReadExt;

// ── Design tokens (from reference HTML) ───────────────────────────────────────
const SURFACE: u32 = 0x0d131e;          // root background
const ON_SURFACE: u32 = 0xdde2f2;       // primary text
const ON_SURFACE_VAR: u32 = 0xbfc8cb;   // secondary text (debug/verbose messages)
const TIMESTAMP_DIM: u32 = 0x475569;    // slate-600 timestamps
const PRIMARY_CYAN: u32 = 0xa3dcec;     // tag color, accents
const TERTIARY_LAVENDER: u32 = 0xd7baff; // lavender accent (stick-to-bottom button)
const LEVEL_INFO: u32 = 0x4ade80;       // green-400
const LEVEL_DEBUG: u32 = 0x22d3ee;      // cyan-400
const LEVEL_WARN: u32 = 0xfacc15;       // yellow-400
const LEVEL_ERROR: u32 = 0xf87171;      // red-400
const LEVEL_FATAL: u32 = 0xc678dd;      // lavender (fatal)
const LEVEL_VERBOSE: u32 = 0x64748b;    // slate-500

// Panel border: rgba(163, 220, 236, 0.10) — ~10% opacity cyan
const BORDER_SUBTLE: u32 = 0xa3dcec1a;
// Scrollbar thumb: rgba(163, 220, 236, 0.20) — ~20% opacity cyan
const SCROLLBAR_THUMB: u32 = 0xa3dcec33;
// Log area inner bg: black/20
const LOG_AREA_BG: u32 = 0x00000033;
// Header bg overlay: 50% opacity of SURFACE_LOWEST
const HEADER_BG: u32 = 0x080e1980;
// Status dot green glow
const STATUS_GREEN: u32 = 0x22c55e;

// Layout
const HEADER_H: f32 = 36.0;
const ROW_HEIGHT: f32 = 22.0;
const SCROLLBAR_W: f32 = 4.0;
const SCROLLBAR_MIN_THUMB_H: f32 = 24.0;

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
            self.scroll_handle.scroll_to_item(
                self.entries.len().saturating_sub(1),
                ScrollStrategy::Bottom,
            );
        }

        let count = self.entries.len();
        let auto_scroll = self.auto_scroll;

        // ── Scrollbar geometry ────────────────────────────────────────
        let viewport_h = f32::from(window.viewport_size().height) - HEADER_H;
        let total_h = count as f32 * ROW_HEIGHT;
        let scroll_offset_y = -f32::from(self.scroll_handle.0.borrow().base_handle.offset().y);

        let thumb_h = if total_h > viewport_h && total_h > 0.0 {
            (viewport_h / total_h * viewport_h).max(SCROLLBAR_MIN_THUMB_H)
        } else {
            viewport_h
        };
        let max_scroll = (total_h - viewport_h).max(0.0);
        let thumb_top = if max_scroll > 0.0 {
            (scroll_offset_y.min(max_scroll) / max_scroll * (viewport_h - thumb_h)).max(0.0)
        } else {
            0.0
        };

        // ── Root layout ───────────────────────────────────────────────
        div()
            .size_full()
            .bg(rgb(SURFACE))
            .flex()
            .flex_col()
            .p_4()
            .child(
                // Glass panel
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgba(BORDER_SUBTLE))
                    .bg(rgba(0x1a202b99)) // surface-container at ~60% opacity
                    .overflow_hidden()
                    // ── Panel header ──────────────────────────────────
                    .child(
                        div()
                            .h(px(HEADER_H))
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .bg(rgba(HEADER_BG))
                            .border_b_1()
                            .border_color(rgba(0xffffff0d)) // white/5
                            .child(
                                // Left: status dot + device name
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div()
                                            .w(px(8.0))
                                            .h(px(8.0))
                                            .rounded_full()
                                            .bg(rgb(STATUS_GREEN)),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(PRIMARY_CYAN))
                                            .child("adb_logcat"),
                                    ),
                            )
                            .child(
                                // Right: entry count + auto indicator
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_4()
                                    .child(
                                        div()
                                            .text_size(rems(0.5625)) // 9px
                                            .text_color(rgb(TIMESTAMP_DIM))
                                            .child(format!("{} entries", count)),
                                    )
                                    .when(auto_scroll, |el| {
                                        el.child(
                                            div()
                                                .text_size(rems(0.5625))
                                                .text_color(rgb(LEVEL_INFO))
                                                .child("⬇ LIVE"),
                                        )
                                    }),
                            ),
                    )
                    // ── Log area (relative wrapper for absolute button) ──
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .flex()
                            .flex_row()
                            .bg(rgba(LOG_AREA_BG))
                            // Virtual log list
                            .child(
                                uniform_list(
                                    "log-entries",
                                    count,
                                    cx.processor(
                                        |this: &mut LogViewer,
                                         range: std::ops::Range<usize>,
                                         _window,
                                         _cx| {
                                            let mut rows = Vec::with_capacity(range.len());
                                            for i in range {
                                                let entry = &this.entries[i];
                                                let (level_label, level_color) = level_display(entry.level);
                                                let msg_color = match entry.level {
                                                    'D' | 'V' => ON_SURFACE_VAR,
                                                    'E' | 'F' => LEVEL_ERROR,
                                                    _ => ON_SURFACE,
                                                };
                                                rows.push(
                                                    div()
                                                        .id(i)
                                                        .w_full()
                                                        .h(px(ROW_HEIGHT))
                                                        .flex()
                                                        .items_center()
                                                        .gap_4()
                                                        .px_4()
                                                        .child(
                                                            // Timestamp
                                                            div()
                                                                .w(px(140.0))
                                                                .flex_shrink_0()
                                                                .text_size(rems(0.6875)) // 11px
                                                                .text_color(rgb(TIMESTAMP_DIM))
                                                                .child(format!("[{}]", entry.timestamp)),
                                                        )
                                                        .child(
                                                            // Level (full word)
                                                            div()
                                                                .w(px(56.0))
                                                                .flex_shrink_0()
                                                                .text_size(rems(0.6875))
                                                                .text_color(rgb(level_color))
                                                                .child(level_label),
                                                        )
                                                        .child(
                                                            // Tag
                                                            div()
                                                                .w(px(140.0))
                                                                .flex_shrink_0()
                                                                .text_size(rems(0.6875))
                                                                .text_color(rgb(PRIMARY_CYAN))
                                                                .overflow_hidden()
                                                                .child(entry.tag.clone()),
                                                        )
                                                        .child(
                                                            // Message
                                                            div()
                                                                .flex_1()
                                                                .text_size(rems(0.6875))
                                                                .text_color(rgb(msg_color))
                                                                .overflow_hidden()
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
                            // ── Scrollbar ────────────────────────────────
                            .child(
                                div()
                                    .w(px(SCROLLBAR_W))
                                    .h_full()
                                    .bg(rgba(0x0d131e80)) // dark track
                                    .flex()
                                    .flex_col()
                                    .child(div().h(px(thumb_top)))
                                    .child(
                                        div()
                                            .w_full()
                                            .h(px(thumb_h))
                                            .rounded_sm()
                                            .bg(rgba(SCROLLBAR_THUMB)),
                                    ),
                            )
                            // ── Stick-to-bottom button (absolute overlay) ─
                            .when(!auto_scroll, |area| {
                                area.child(
                                    div()
                                        .id("stick-to-bottom")
                                        .absolute()
                                        .bottom(px(16.0))
                                        .right(px(20.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .w(px(32.0))
                                        .h(px(32.0))
                                        .rounded_full()
                                        .bg(rgb(TERTIARY_LAVENDER))
                                        .text_color(rgb(SURFACE))
                                        .text_sm()
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
                            }),
                    ),
            )
    }
}

/// Map level char to (display label, color).
fn level_display(level: char) -> (&'static str, u32) {
    match level {
        'I' => ("INFO", LEVEL_INFO),
        'D' => ("DEBUG", LEVEL_DEBUG),
        'W' => ("WARN", LEVEL_WARN),
        'E' => ("ERROR", LEVEL_ERROR),
        'F' => ("FATAL", LEVEL_FATAL),
        'V' => ("VERBOSE", LEVEL_VERBOSE),
        _ => ("?", LEVEL_VERBOSE),
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
