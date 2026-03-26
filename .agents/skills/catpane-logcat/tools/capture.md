# Capture lifecycle tools

## `start_capture`

Use `start_capture` to begin buffering logcat for later `get_logs` calls.

### Key arguments

- `device` — strongly recommended when more than one adb device could be connected
- `package` — resolve a package name to a PID before capture starts
- `pid` — explicit PID filter
- `capacity` — ring-buffer size for this capture
- `restart` — replace an existing capture for the same device

### Example: capture a device

```json
{
  "device": "emulator-5554"
}
```

### Example: capture one package

```json
{
  "device": "emulator-5554",
  "package": "com.example.app",
  "capacity": 20000
}
```

### Example: capture one PID and replace an existing capture

```json
{
  "device": "emulator-5554",
  "pid": 12345,
  "restart": true
}
```

### Notes

- If `device` is omitted, auto-selection only works when exactly one adb device is connected.
- Use either `pid` or `package`, not both.
- `package` is resolved to a PID at start time; it is not a live package subscription.
- If a capture is already running for the same device and `restart` is not `true`, the tool can fail with a conflict.
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

- This clears the buffered entries only.
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
  "device": "emulator-5554"
}
```

### Notes

- After `stop_capture`, later `get_logs` or `clear_logs` calls for that capture will fail until a new capture is started.
- Use `stop_capture` to free the runtime state; use `clear_logs` to keep streaming but reset the buffer.
