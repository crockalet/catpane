---
name: catpane-logcat
description: Use this when you need to inspect Android adb/logcat output through CatPane's MCP server, including starting captures, checking status, and querying buffered logs with filters and cursors.
---

# CatPane logcat MCP

Use this skill when you need Android logcat from CatPane's MCP runtime. It is for live debugging, reproductions, and incremental monitoring; it is not a generic `adb logcat` tutorial.

## Use it when

- you need to discover whether a useful capture already exists
- you need to start a scoped capture for one device, app, or PID
- you need focused log queries instead of dumping raw logcat
- you need to poll for new logs during an investigation

## Tool surface

- `get_status` — inspect captures and optionally include connected adb devices
- `list_devices` — list currently connected adb devices
- `start_capture` — start buffering logcat for a device, optionally scoped by `package` or `pid`
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
   - If you only need device serials, `list_devices` is the lighter call.
2. **Start capture only when needed.**
   - Call `start_capture` with `device` when multiple devices may be connected.
   - Use either `package` or `pid`, not both.
   - Use `restart: true` only when replacing an existing capture on the same device.
3. **Query with focused filters.**
   - Start with `limit`, `minLevel`, `tagQuery`, and `text`.
   - Prefer targeted queries over large unfiltered pulls.
4. **Page with cursors.**
   - `get_logs` is cursor-based.
   - Reuse the same `order`, pass `page.nextCursor` as the next `cursor`, and stop when `page.hasMore` is false.
5. **Use `since` for incremental polling.**
   - Keep the last processed threadtime timestamp.
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
- `tagQuery` uses CatPane syntax:
  - `tag:ActivityManager` — exact include
  - `tag-:chatty` — exact exclude
  - `tag~:^(MyApp|Auth)` — regex include
- `text` is a case-insensitive substring filter over both tag and message.
- `since` must use logcat threadtime format: `MM-DD HH:MM:SS.mmm`

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
  "device": "emulator-5554",
  "package": "com.example.app"
}
```

`get_logs`

```json
{
  "device": "emulator-5554",
  "order": "desc",
  "limit": 100,
  "minLevel": "error",
  "tagQuery": "tag~:^(MyApp|Auth) tag-:OkHttp",
  "text": "timeout"
}
```
