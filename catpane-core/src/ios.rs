use std::collections::HashMap;

use serde::Deserialize;

use crate::command::OneShotCommand;

const SIMCTL_LIST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const SIMCTL_BOOT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const OPEN_SIMULATOR_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IosSimulator {
    pub udid: String,
    pub name: String,
    pub runtime: String,
    pub state: String,
}

#[derive(Debug, Deserialize)]
struct SimctlDevices {
    devices: HashMap<String, Vec<SimctlDevice>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SimctlDevice {
    udid: String,
    name: String,
    state: String,
    is_available: bool,
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

fn open_command<I, S>(
    args: I,
    context: &'static str,
    timeout: std::time::Duration,
) -> OneShotCommand
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    OneShotCommand::new("open", args, context, timeout)
}

fn parse_simulators(stdout: &[u8]) -> Result<Vec<IosSimulator>, String> {
    let parsed: SimctlDevices = serde_json::from_slice(stdout).map_err(|err| {
        format!("Failed to parse `xcrun simctl list devices --json` output: {err}")
    })?;
    let mut simulators = Vec::new();
    for (runtime, devices) in parsed.devices {
        for device in devices {
            if !device.is_available {
                continue;
            }
            simulators.push(IosSimulator {
                udid: device.udid,
                name: device.name,
                runtime: runtime.clone(),
                state: device.state,
            });
        }
    }

    simulators.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.runtime.cmp(&right.runtime))
            .then_with(|| left.udid.cmp(&right.udid))
    });
    Ok(simulators)
}

pub async fn list_available_simulators_strict() -> Result<Vec<IosSimulator>, String> {
    let command = xcrun_command(
        ["simctl", "list", "devices", "--json"],
        "listing available iOS simulators",
        SIMCTL_LIST_TIMEOUT,
    );
    let output = command.ensure_success(command.run().await?)?;
    parse_simulators(&output.stdout)
}

pub async fn list_available_simulators() -> Vec<IosSimulator> {
    list_available_simulators_strict().await.unwrap_or_default()
}

pub async fn list_booted_simulators_strict() -> Result<Vec<IosSimulator>, String> {
    Ok(list_available_simulators_strict()
        .await?
        .into_iter()
        .filter(|simulator| simulator.state == "Booted")
        .collect())
}

pub async fn list_booted_simulators() -> Vec<IosSimulator> {
    list_booted_simulators_strict().await.unwrap_or_default()
}

pub async fn boot_simulator(udid: &str) -> Result<String, String> {
    let boot_command = xcrun_command(
        ["simctl", "bootstatus", udid, "-b"],
        "waiting for an iOS simulator to boot",
        SIMCTL_BOOT_TIMEOUT,
    );
    let output = boot_command.run().await?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    let combined = if stdout.is_empty() {
        stderr.clone()
    } else if stderr.is_empty() {
        stdout.clone()
    } else {
        format!("{stdout}\n{stderr}")
    };

    if !output.status.success() {
        Err(boot_command.status_error(&output))
    } else {
        let simulators = list_available_simulators_strict().await?;
        let simulator = simulators
            .iter()
            .find(|simulator| simulator.udid == udid)
            .ok_or_else(|| format!("Simulator {udid} was not found after booting"))?;

        if simulator.state != "Booted" {
            let details = if combined.is_empty() {
                String::new()
            } else {
                format!("\n\nsimctl output:\n{combined}")
            };
            return Err(format!(
                "Simulator {} did not end in the Booted state (current state: {}).{}",
                simulator.name, simulator.state, details
            ));
        }

        let open_command = open_command(
            ["-a", "Simulator", "--args", "-CurrentDeviceUDID", udid],
            "opening Simulator.app for a booted iOS simulator",
            OPEN_SIMULATOR_TIMEOUT,
        );
        open_command.ensure_success(open_command.run().await?)?;

        Ok(format!(
            "Booted {} and opened Simulator.app",
            simulator.name
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::parse_simulators;

    #[test]
    fn parses_available_simulators_from_simctl_json() {
        let simulators = parse_simulators(
            br#"{
                "devices": {
                    "com.apple.CoreSimulator.SimRuntime.iOS-18-0": [
                        {
                            "udid": "booted-1",
                            "name": "iPhone 16",
                            "state": "Booted",
                            "isAvailable": true
                        },
                        {
                            "udid": "ignored-1",
                            "name": "Unavailable iPhone",
                            "state": "Shutdown",
                            "isAvailable": false
                        }
                    ]
                }
            }"#,
        )
        .unwrap();

        assert_eq!(simulators.len(), 1);
        assert_eq!(simulators[0].udid, "booted-1");
        assert_eq!(
            simulators[0].runtime,
            "com.apple.CoreSimulator.SimRuntime.iOS-18-0"
        );
    }

    #[test]
    fn reports_invalid_simctl_json() {
        let err = parse_simulators(b"{not-json").unwrap_err();
        assert!(err.contains("Failed to parse `xcrun simctl list devices --json` output"));
    }
}
