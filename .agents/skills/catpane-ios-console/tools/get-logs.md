# `get_logs`

Use `get_logs` to query buffered iOS simulator console log entries from an existing capture. It is optimized for targeted retrieval, not bulk dumping.

## Core arguments

- `captureId` or `device` — select the capture to query
- `order` — `desc` (newest first, default) or `asc` (forward paging)
- `limit` — default `100`, maximum `1000`
- `cursor` — exclusive sequence anchor for pagination
- `minLevel` — minimum level threshold
- `text` — case-insensitive substring across process, subsystem, category, and message
- `process` — case-insensitive substring filter on originating process name
- `subsystem` — case-insensitive substring filter on logging subsystem
- `category` — case-insensitive substring filter on logging category
- `since` — inclusive timestamp lower bound in `MM-DD HH:MM:SS.mmm`

## Log level filtering

`minLevel` is a threshold, not an exact match.

Accepted values:

- full names: `verbose`, `debug`, `info`, `warn`, `error`, `fatal`
- aliases: `V`, `D`, `I`, `W`, `E`, `F`

Examples:

- `minLevel: "warn"` returns `warn`, `error`, and `fatal`
- `minLevel: "E"` returns `error` and `fatal`

## iOS-specific filters

Use `process`, `subsystem`, and `category` to focus on specific system components:

- `process: "MyApp"` — filter to logs from a specific process
- `subsystem: "com.example.networking"` — filter to a specific logging subsystem
- `category: "HTTP"` — filter to a specific logging category

These filters combine with `minLevel`, `text`, and `since` using AND semantics.

## Example: focused error query

```json
{
  "device": "iPhone 16 Pro",
  "order": "desc",
  "limit": 100,
  "minLevel": "error",
  "process": "MyApp",
  "text": "timeout"
}
```

## Example: filter by subsystem and category

```json
{
  "device": "iPhone 16 Pro",
  "order": "desc",
  "limit": 50,
  "subsystem": "com.example.app",
  "category": "networking"
}
```

## Example: page older results with the returned cursor

```json
{
  "captureId": "capture-1",
  "order": "desc",
  "cursor": 8421,
  "limit": 200
}
```

## Example: incremental polling with `since`

```json
{
  "captureId": "capture-1",
  "order": "asc",
  "since": "03-10 06:30:47.000",
  "minLevel": "info",
  "process": "MyApp"
}
```

## Pagination behavior

- `get_logs` uses cursor-based pagination.
- With no `cursor`, `order: "desc"` starts at the newest buffered entries and `order: "asc"` starts at the oldest buffered entries.
- `cursor` is exclusive.
- For `order: "desc"`, pass `page.nextCursor` back to fetch older entries.
- For `order: "asc"`, pass `page.nextCursor` back to fetch newer entries after the last seen sequence.
- Keep the same filters and the same `order` while paging one result set.
- Use `page.hasMore` to decide whether to continue paging.
- `page.nextCursor` remains the correct next anchor even when `page.hasMore` is false.

## Incremental polling behavior

- `since` is inclusive: entries at exactly that timestamp are returned again.
- Keep the last processed timestamp and dedupe the boundary entry by `seq` if you need strict once-only processing.
- Use `since` when you want time-based polling across repeated calls, especially after `clear_logs` or when you do not want to keep cursor state indefinitely.

## Notes

- If more than one capture exists, do not rely on an unqualified call; pass `captureId` or `device`.
- `entries[]` are buffered logs only. If `capture.buffer.dropped` grows or the buffer is near capacity, older logs may already be gone.
- Response pagination fields live under `page`: `returned`, `firstSeq`, `lastSeq`, `nextCursor`, `hasMore`.
