# `get_status`

Use `get_status` as the default first call. It shows what captures exist, whether they are running, and how full each buffer is.

## Common arguments

### All captures

```json
{}
```

### All captures plus currently available devices

```json
{
  "includeDevices": true
}
```

### One capture by ID

```json
{
  "captureId": "capture-1"
}
```

### One capture by device

```json
{
  "device": "iPhone 16 Pro"
}
```

## What to inspect

- `captureCount` and `runningCaptureCount`
- `captures[].running`
- `captures[].device` and `captures[].captureId`
- `captures[].buffer.len`, `captures[].buffer.capacity`, and `captures[].buffer.dropped`
- `captures[].parsedEntries` and `captures[].parseErrors`
- `devices[]` when `includeDevices` is `true`
- `devices[].platform` — use `"iOS"` entries for this skill

## Operational notes

- With no selector, `captures` includes every registered capture.
- With `captureId` or `device`, `captures` is narrowed to that capture, but the top-level counts still describe the whole runtime.
- `includeDevices` returns both connected Android devices and any iOS capture targets CatPane can currently use.
- Use this before `start_capture` to avoid duplicate captures.
- Use this before `get_logs` to learn the right `captureId` or `device` selector.
