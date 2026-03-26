# `list_devices`

Use `list_devices` when you need current adb serials before starting a capture, or when `get_status` was called without `includeDevices`.

## Arguments

```json
{}
```

## What to read from the response

- `deviceCount`
- `devices[].serial` — pass this to `start_capture.device` or as a `device` selector in other tools
- `devices[].friendlyName` and `devices[].description` — useful when choosing between similar devices
- `devices[].isTcp` — useful when both USB and TCP devices are present

## Operational notes

- `list_devices` does not create a capture.
- If `deviceCount` is `0`, `start_capture` will fail until adb sees a device.
- If you also need capture state, prefer `get_status` with `{"includeDevices": true}` so you can inspect both captures and devices in one call.
