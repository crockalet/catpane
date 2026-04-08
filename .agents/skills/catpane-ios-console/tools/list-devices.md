# `list_devices`

Use `list_devices` when you need to discover current capture-device identifiers before starting a capture, or when `get_status` was called without `includeDevices`.

## Arguments

```json
{}
```

## What to read from the response

- `deviceCount`
- `devices[].serial` — pass this to `start_capture.device` or as a `device` selector in other tools
- `devices[].friendlyName` and `devices[].description` — useful when choosing between similar simulators
- `devices[].platform` — look for `"iOS"` entries

## Operational notes

- `list_devices` does not create a capture.
- This shared tool can also return connected Android devices; ignore non-iOS entries for this skill.
- If no iOS simulator entries are found, `start_capture` will fail. Boot a simulator first.
- If you also need capture state, prefer `get_status` with `{"includeDevices": true}` so you can inspect both captures and simulators in one call.
