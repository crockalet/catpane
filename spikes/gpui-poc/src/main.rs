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

use catpane_core::log_entry::{LogEntry, LogLevel, LogPlatform, parse_logcat_line};
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
                                                    LogLevel::Debug | LogLevel::Verbose => ON_SURFACE_VAR,
                                                    LogLevel::Error | LogLevel::Fatal => LEVEL_ERROR,
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

/// Map LogLevel to (display label, color).
fn level_display(level: LogLevel) -> (&'static str, u32) {
    match level {
        LogLevel::Info => ("INFO", LEVEL_INFO),
        LogLevel::Debug => ("DEBUG", LEVEL_DEBUG),
        LogLevel::Warn => ("WARN", LEVEL_WARN),
        LogLevel::Error => ("ERROR", LEVEL_ERROR),
        LogLevel::Fatal => ("FATAL", LEVEL_FATAL),
        LogLevel::Verbose => ("VERBOSE", LEVEL_VERBOSE),
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

// stream_logcat now uses catpane_core::log_entry::parse_logcat_line (imported at top)

/// Spawn `adb logcat -v threadtime` and pipe parsed entries through the channel.
/// Falls back to demo data if adb is unavailable or no device is connected.
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
            eprintln!("adb logcat failed ({e}), falling back to demo data");
            stream_demo(tx).await;
            return;
        }
    };

    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            eprintln!("Failed to capture adb stdout, falling back to demo data");
            stream_demo(tx).await;
            return;
        }
    };

    let mut lines = tokio::io::BufReader::new(stdout).lines();

    // Read with a timeout — if adb connects but no lines arrive within 3s
    // (e.g. no device), fall back to demo data.
    let first_line = tokio::time::timeout(
        Duration::from_secs(3),
        lines.next_line(),
    )
    .await;

    match first_line {
        Ok(Ok(Some(line))) => {
            if let Some(entry) = parse_logcat_line(&line) {
                if tx.send(entry).is_err() {
                    return;
                }
            }
        }
        _ => {
            eprintln!("No logcat output within 3s, falling back to demo data");
            stream_demo(tx).await;
            return;
        }
    }

    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(entry) = parse_logcat_line(&line) {
            if tx.send(entry).is_err() {
                break;
            }
        }
    }
}

// ── Demo data fallback ───────────────────────────────────────────────────────

const TAGS: [&str; 10] = [
    "MainActivity", "NetworkManager", "DatabaseHelper", "UIController",
    "ServiceLocator", "EventBus", "CrashReporter", "OkHttp", "GC", "ViewRootImpl",
];

const MESSAGES: [&[&str]; 10] = [
    &["onCreate", "onResume", "onPause", "onStop", "setContentView completed"],
    &["GET https://api.example.com/v2/users 200 (142ms)", "POST /auth/refresh 401", "socket timeout after 30000ms"],
    &["query SELECT * FROM logs WHERE ts > ? returned 847 rows (12ms)", "beginTransaction", "VACUUM completed in 340ms"],
    &["measure/layout pass: 4.2ms", "draw frame: 6.1ms", "invalidate requested"],
    &["resolve<AuthService>", "resolve<Logger>", "circular dependency detected in AnalyticsModule"],
    &["dispatch USER_LOGIN", "dispatch FETCH_FEED", "3 subscribers notified for DATA_REFRESH"],
    &["FATAL EXCEPTION: main", "java.lang.NullPointerException", "ANR in com.example.app (5012ms)"],
    &["<-- 200 OK https://cdn.example.com/img/banner.webp (87ms)", "TLS handshake completed"],
    &["GC_CONCURRENT freed 2408K, 18% free 14312K/17408K", "Background GC freed 1204K"],
    &["performTraversals: 8.3ms", "Choreographer: Skipped 3 frames!", "Surface destroyed"],
];

/// Fallback: generates demo data when adb is unavailable.
async fn stream_demo(tx: mpsc::Sender<LogEntry>) {
    for i in 0usize..50_000 {
        if tx.send(make_entry(i)).is_err() {
            return;
        }
    }
    let mut i = 50_000usize;
    loop {
        if tx.send(make_entry(i)).is_err() {
            break;
        }
        i += 1;
        if i % 10 == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

fn mix(i: usize) -> usize {
    let mut h = i.wrapping_mul(0x9E3779B97F4A7C15);
    h ^= h >> 16;
    h.wrapping_mul(0xBF58476D1CE4E5B9)
}

fn make_entry(i: usize) -> LogEntry {
    let h = mix(i);
    let tag_idx = h % TAGS.len();
    let level = match h % 20 {
        0 => LogLevel::Error,
        1..=2 => LogLevel::Warn,
        3..=5 => LogLevel::Verbose,
        6..=11 => LogLevel::Info,
        _ => LogLevel::Debug,
    };
    let msgs = MESSAGES[tag_idx];
    LogEntry {
        platform: LogPlatform::Android,
        timestamp: format!(
            "04-{:02} {:02}:{:02}:{:02}.{:03}",
            (i / 86_400) % 28 + 1,
            (i / 3_600) % 24,
            (i / 60) % 60,
            i % 60,
            i % 1_000,
        ),
        pid: Some(1000 + (h % 200) as u32),
        tid: Some((2000 + h % 50) as u64),
        level,
        tag: TAGS[tag_idx].to_string(),
        process: None,
        subsystem: None,
        category: None,
        message: msgs[h / 10 % msgs.len()].to_string(),
    }
}
