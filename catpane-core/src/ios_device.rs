use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;

use crate::command::OneShotCommand;

const DEVICE_LIST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IosDevice {
    pub udid: String,
    pub name: String,
    pub description: String,
}

pub fn idevicesyslog_binary() -> &'static str {
    static IDEVICESYSLOG_PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    IDEVICESYSLOG_PATH.get_or_init(|| {
        resolve_binary(
            "idevicesyslog",
            [
                "/opt/homebrew/bin/idevicesyslog",
                "/usr/local/bin/idevicesyslog",
            ],
        )
    })
}

pub fn idevicesyslog_available() -> bool {
    let binary = idevicesyslog_binary();
    binary != "idevicesyslog" || binary_in_path("idevicesyslog")
}

fn xcrun_command<I, S>(
    args: I,
    context: &'static str,
    timeout: std::time::Duration,
) -> OneShotCommand
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    OneShotCommand::new("xcrun", args, context, timeout)
}

fn resolve_binary<I, S>(name: &str, candidates: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<Path>,
{
    for candidate in candidates {
        let candidate = candidate.as_ref();
        if candidate.exists() {
            return candidate.to_string_lossy().into_owned();
        }
    }

    if let Some(path) = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.exists())
    }) {
        return path.to_string_lossy().into_owned();
    }

    name.to_string()
}

fn binary_in_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .any(|candidate| candidate.exists())
    })
}

#[derive(Debug, Deserialize)]
struct DevicectlList {
    result: DevicectlResult,
}

#[derive(Debug, Deserialize)]
struct DevicectlResult {
    devices: Vec<DevicectlDevice>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DevicectlDevice {
    #[serde(default)]
    connection_properties: DevicectlConnectionProperties,
    #[serde(default)]
    device_properties: DevicectlDeviceProperties,
    #[serde(default)]
    hardware_properties: DevicectlHardwareProperties,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DevicectlConnectionProperties {
    #[serde(default)]
    pairing_state: String,
    #[serde(default)]
    transport_type: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DevicectlDeviceProperties {
    #[serde(default)]
    name: String,
    #[serde(default)]
    os_version_number: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DevicectlHardwareProperties {
    #[serde(default)]
    marketing_name: String,
    #[serde(default)]
    platform: String,
    #[serde(default)]
    reality: String,
    #[serde(default)]
    udid: String,
}

fn parse_devices(stdout: &[u8]) -> Result<Vec<IosDevice>, String> {
    let parsed: DevicectlList = serde_json::from_slice(stdout)
        .map_err(|err| format!("Failed to parse `xcrun devicectl list devices` output: {err}"))?;

    let mut devices = parsed
        .result
        .devices
        .into_iter()
        .filter_map(|device| {
            if !device
                .hardware_properties
                .platform
                .eq_ignore_ascii_case("iOS")
                || !device
                    .hardware_properties
                    .reality
                    .eq_ignore_ascii_case("physical")
            {
                return None;
            }

            if !device
                .connection_properties
                .pairing_state
                .eq_ignore_ascii_case("paired")
                || !device
                    .connection_properties
                    .transport_type
                    .eq_ignore_ascii_case("wired")
            {
                return None;
            }

            let udid = normalize_optional_string(&device.hardware_properties.udid)?;
            let name = normalize_optional_string(&device.device_properties.name)
                .or_else(|| normalize_optional_string(&device.hardware_properties.marketing_name))
                .unwrap_or_else(|| udid.clone());

            let mut description_parts = Vec::new();
            if let Some(model) =
                normalize_optional_string(&device.hardware_properties.marketing_name)
                && model != name
            {
                description_parts.push(model);
            }
            if let Some(version) =
                normalize_optional_string(&device.device_properties.os_version_number)
            {
                description_parts.push(format!("iOS {version}"));
            }
            description_parts.push("USB".to_string());

            Some(IosDevice {
                udid,
                name,
                description: description_parts.join(" · "),
            })
        })
        .collect::<Vec<_>>();

    devices.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.description.cmp(&right.description))
            .then_with(|| left.udid.cmp(&right.udid))
    });
    Ok(devices)
}

fn devicectl_json_path() -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "catpane-devicectl-{}-{now}.json",
        std::process::id()
    ))
}

pub async fn list_connected_devices_strict() -> Result<Vec<IosDevice>, String> {
    if !idevicesyslog_available() {
        return Ok(Vec::new());
    }

    let json_path = devicectl_json_path();
    let json_path_string = json_path.to_string_lossy().into_owned();
    let command = xcrun_command(
        [
            "devicectl",
            "list",
            "devices",
            "--json-output",
            &json_path_string,
        ],
        "listing connected physical iOS devices",
        DEVICE_LIST_TIMEOUT,
    );
    let output = command.run().await?;
    let result = if output.status.success() {
        let bytes = fs::read(&json_path).map_err(|err| {
            format!(
                "Failed to read `xcrun devicectl list devices` JSON output from {}: {err}",
                json_path.display()
            )
        })?;
        parse_devices(&bytes)
    } else {
        Err(command.status_error(&output))
    };
    let _ = fs::remove_file(&json_path);
    result
}

pub async fn list_connected_devices() -> Vec<IosDevice> {
    list_connected_devices_strict().await.unwrap_or_default()
}

fn normalize_optional_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::parse_devices;

    #[test]
    fn parses_only_paired_wired_ios_devices() {
        let devices = parse_devices(
            br#"{
                "result": {
                    "devices": [
                        {
                            "connectionProperties": {
                                "pairingState": "paired",
                                "transportType": "wired"
                            },
                            "deviceProperties": {
                                "name": "Yaniu",
                                "osVersionNumber": "26.3.1"
                            },
                            "hardwareProperties": {
                                "marketingName": "iPhone 15 Pro Max",
                                "platform": "iOS",
                                "reality": "physical",
                                "udid": "00008130-000618A91E08001C"
                            }
                        },
                        {
                            "connectionProperties": {
                                "pairingState": "paired",
                                "transportType": "wireless"
                            },
                            "deviceProperties": {
                                "name": "Wireless iPhone"
                            },
                            "hardwareProperties": {
                                "marketingName": "iPhone 16 Pro",
                                "platform": "iOS",
                                "reality": "physical",
                                "udid": "wireless"
                            }
                        },
                        {
                            "connectionProperties": {
                                "pairingState": "paired",
                                "transportType": "wired"
                            },
                            "deviceProperties": {
                                "name": "iPhone Simulator"
                            },
                            "hardwareProperties": {
                                "marketingName": "iPhone 16",
                                "platform": "iOS",
                                "reality": "simulated",
                                "udid": "sim"
                            }
                        }
                    ]
                }
            }"#,
        )
        .unwrap();

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].udid, "00008130-000618A91E08001C");
        assert_eq!(devices[0].name, "Yaniu");
        assert_eq!(
            devices[0].description,
            "iPhone 15 Pro Max · iOS 26.3.1 · USB"
        );
    }

    #[test]
    fn reports_invalid_devicectl_json() {
        let err = parse_devices(br#"{"result": {"devices": "oops"}}"#).unwrap_err();
        assert!(err.contains("Failed to parse `xcrun devicectl list devices` output"));
    }
}
