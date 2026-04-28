# Capture lifecycle tools

## `start_capture`

Use `start_capture` to begin buffering iOS logs for later `get_logs`, `get_crashes`, and `get_watch_matches` calls.

### Key arguments

- `device` — strongly recommended when more than one capture target is available
- `capacity` — ring-buffer size for this capture
- `process` — iOS source-side process scope
- `text` — iOS source-side message scope
- `predicate` — additional simulator NSPredicate scope
- `clean` — prefer the cleaner iOS source-side stream; pass `false` for raw capture
- `quiet` — physical-iOS quiet mode via `idevicesyslog --quiet`
- `restart` — replace an existing capture for the same device or simulator

### Example: capture a scoped iOS stream

```json
{
  "device": "My iPhone",
  "process": "MyApp",
  "text": "timeout",
  "quiet": true
}
```

### Example: capture with custom capacity

```json
{
  "device": "iPhone 16 Pro",
  "capacity": 20000
}
```

### Example: capture the raw iOS stream

```json
{
  "device": "iPhone 16 Pro",
  "clean": false
}
```

### Example: replace an existing capture

```json
{
  "device": "iPhone 16 Pro",
  "restart": true
}
```

### Notes

- If `device` is omitted, auto-selection only works when exactly one capture target is available.
- `package` and `pid` are not supported for iOS captures.
- `predicate` is only supported for simulator captures.
- `clean` defaults to `true` for iOS captures; use `clean: false` when you need the raw device stream.
- Prefer source scoping over large unfiltered captures; source scoping protects the main buffer from irrelevant lines.
- If a capture is already running for the same target and `restart` is not `true`, the tool can fail with a conflict.
- `capacity` is the per-capture ring-buffer size; older logs fall out when the buffer fills.

## `clear_logs`

Use `clear_logs` when you want a clean window for a reproduction but want to keep the capture running.

### Example

```json
{
  "captureId": "capture-1"
}
```

### Notes

- This clears the main buffered entries and any retained watch matches.
- The capture keeps running and new logs continue to arrive.
- Prefer this over `stop_capture` when you only want to reset the observation window.

## `stop_capture`

Use `stop_capture` when you are done with a capture and want to remove it from runtime state.

### Example

```json
{
  "captureId": "capture-1"
}
```

### Alternative selector

```json
{
  "device": "iPhone 16 Pro"
}
```

### Notes

- After `stop_capture`, later `get_logs` or `clear_logs` calls for that capture will fail until a new capture is started.
- Use `stop_capture` to free the runtime state; use `clear_logs` to keep streaming but reset the buffer.
