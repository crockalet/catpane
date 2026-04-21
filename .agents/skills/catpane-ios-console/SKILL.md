---
name: catpane-ios-console
description: Use this when you need to inspect iOS simulator or physical-device logs through CatPane's MCP server, including starting scoped captures, checking status, pinning relevant lines, and querying buffered logs.
---

# CatPane iOS console MCP

Use this skill when you need iOS logs from CatPane's MCP runtime. It is for live debugging, reproductions, and incremental monitoring of simulator or physical-device output.

## Use it when

- you need to discover whether a useful capture already exists
- you need to start a scoped capture for an iOS simulator or physical device
- you need focused log queries instead of dumping raw console output
- you need to poll for new high-signal logs during an investigation

## Tool surface

- `get_status` — inspect captures and optionally include all currently available capture devices
- `list_devices` — list all currently available capture devices; focus on iOS entries
- `start_capture` — start buffering console logs for an iOS capture target
- `get_logs` — read buffered logs with filters and cursor pagination
- `clear_logs` — reset the buffered window for a capture without stopping it
- `stop_capture` — stop and remove a capture
- `get_crashes` — detect structured crash reports from buffered logs
- `create_watch` — pin high-signal matches so they survive main-buffer overflow
- `list_watches` — inspect active pinned watches
- `get_watch_matches` — poll retained watch matches incrementally

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
2. **Start capture only when needed, and scope it early.**
   - Call `start_capture` with `device` when more than one capture target is available.
   - For iOS, prefer `process`, `text`, and simulator `predicate` scope so irrelevant lines never enter the main buffer.
   - Use `quiet: true` on physical iOS captures when you need a lighter default stream.
   - Use `restart: true` only when replacing an existing capture on the same target.
3. **Clear logs before a fresh reproduction.**
   - Call `clear_logs` to reset the main buffer and any retained watch matches.
4. **Create a pinned watch for the important signal.**
   - Call `create_watch` for the app process, subsystem, or key error text before reproducing.
   - Poll `get_watch_matches` during the session; retained matches survive main-buffer overflow.
5. **Check crashes early.**
   - Call `get_crashes` after reproduction to surface structured crash reports without paging raw logs first.
6. **Query with focused filters.**
   - Keep `get_logs` pulls small.
   - Start with `limit`, `minLevel`, and `text`.
   - Use `process`, `subsystem`, and `category` to narrow results to specific system components.
   - Prefer targeted queries over large unfiltered pulls.
7. **Page with cursors.**
   - `get_logs` is cursor-based.
   - Reuse the same `order`, pass `page.nextCursor` as the next `cursor`, and stop when `page.hasMore` is false.
8. **Use `since` for incremental polling.**
   - Keep the last processed timestamp.
   - Pass it back as `since` on the next call.
   - `since` is inclusive, so expect one boundary overlap and dedupe by `seq` if needed.
9. **Stop capture only when you are done.**
   - Use `stop_capture` when the capture is no longer needed.

## Query rules

- Always prefer `captureId` or `device` once more than one capture exists. Unqualified calls only auto-resolve when exactly one capture is registered.
- `get_logs` reads the main in-memory ring buffer for a capture. Older entries can age out when the buffer reaches capacity.
- `create_watch` + `get_watch_matches` give you a second retained path for relevant lines; use that for long or noisy iOS sessions.
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
  "device": "My iPhone",
  "process": "MyApp",
  "text": "timeout",
  "quiet": true
}
```

`create_watch`

```json
{
  "device": "My iPhone",
  "name": "app-errors",
  "pattern": "timeout",
  "minLevel": "error",
  "tag": "MyApp"
}
```

`get_watch_matches`

```json
{
  "device": "My iPhone",
  "watchId": "w1",
  "sinceSeq": 0,
  "limit": 100
}
```

`get_logs`

```json
{
  "device": "My iPhone",
  "order": "desc",
  "limit": 50,
  "minLevel": "error",
  "process": "MyApp",
  "subsystem": "com.example.app"
}
```
