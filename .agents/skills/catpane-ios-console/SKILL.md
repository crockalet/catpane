---
name: catpane-ios-console
description: Use this when you need to inspect booted iOS simulator console logs through CatPane's MCP server, including starting captures, checking status, and querying buffered logs with filters and cursors.
---

# CatPane iOS simulator console MCP

Use this skill when you need iOS simulator console logs from CatPane's MCP runtime. It is for live debugging, reproductions, and incremental monitoring of iOS simulator output.

## Use it when

- you need to discover whether a useful capture already exists
- you need to start a capture for a booted iOS simulator
- you need focused log queries instead of dumping raw console output
- you need to poll for new logs during an investigation

## Tool surface

- `get_status` — inspect captures and optionally include all currently available capture devices
- `list_devices` — list all currently available capture devices; focus on iOS simulator entries
- `start_capture` — start buffering console logs for a simulator
- `get_logs` — read buffered logs with filters and cursor pagination
- `clear_logs` — reset the buffered window for a capture without stopping it
- `stop_capture` — stop and remove a capture

Supporting docs:

- [tools/status.md](tools/status.md) — `get_status`
- [tools/list-devices.md](tools/list-devices.md) — `list_devices`
- [tools/capture.md](tools/capture.md) — `start_capture`, `clear_logs`, `stop_capture`
- [tools/get-logs.md](tools/get-logs.md) — `get_logs`

## Recommended workflow

1. **Check runtime state first.**
   - Call `get_status` with `{"includeDevices": true}`.
   - If a suitable capture already exists and is `running`, reuse it.
   - If you only need device identifiers, `list_devices` is the lighter call.
   - Both calls can include Android devices too, so filter to `platform: "iOS"` for this skill.
2. **Start capture only when needed.**
   - Call `start_capture` with `device` when multiple simulators are booted.
   - Use `restart: true` only when replacing an existing capture on the same simulator.
3. **Query with focused filters.**
   - Start with `limit`, `minLevel`, and `text`.
   - Use `process`, `subsystem`, and `category` to narrow results to specific system components.
   - Prefer targeted queries over large unfiltered pulls.
4. **Page with cursors.**
   - `get_logs` is cursor-based.
   - Reuse the same `order`, pass `page.nextCursor` as the next `cursor`, and stop when `page.hasMore` is false.
5. **Use `since` for incremental polling.**
   - Keep the last processed timestamp.
   - Pass it back as `since` on the next call.
   - `since` is inclusive, so expect one boundary overlap and dedupe by `seq` if needed.
6. **Clear logs for a fresh observation window.**
   - Use `clear_logs` before reproducing an issue when you want a clean buffer.
   - Use `stop_capture` only when the capture is no longer needed.

## Query rules

- Always prefer `captureId` or `device` once more than one capture exists. Unqualified calls only auto-resolve when exactly one capture is registered.
- `get_logs` reads the in-memory ring buffer for a capture. Older entries can age out when the buffer reaches capacity.
- `cursor` is exclusive:
  - `order: "desc"` returns older entries with `seq < cursor`
  - `order: "asc"` returns newer entries with `seq > cursor`
- Use `page.hasMore` to decide whether to continue paging. `page.nextCursor` is still the correct next anchor.
- `minLevel` is a threshold, not an exact match. Example: `warn` returns `warn`, `error`, and `fatal`.
- `text` is a case-insensitive substring filter over process name, subsystem, category, and message.
- `process` is a case-insensitive substring filter on the originating process name.
- `subsystem` is a case-insensitive substring filter on the logging subsystem.
- `category` is a case-insensitive substring filter on the logging category.
- `since` must use the format: `MM-DD HH:MM:SS.mmm`

## Quick start

`get_status`

```json
{
  "includeDevices": true
}
```

`start_capture`

```json
{
  "device": "iPhone 16 Pro"
}
```

`get_logs`

```json
{
  "device": "iPhone 16 Pro",
  "order": "desc",
  "limit": 100,
  "minLevel": "error",
  "process": "MyApp",
  "subsystem": "com.example.app",
  "text": "timeout"
}
```
