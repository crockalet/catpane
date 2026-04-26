use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::command::OneShotCommand;
use crate::network_condition::{
    NetworkConditionPreset, ios_network_throttling_enabled, ios_network_throttling_gate_message,
};

const SIMCTL_LIST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const SIMCTL_BOOT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const OPEN_SIMULATOR_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const IOS_NETWORK_CONTROL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

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

fn bundled_app_dir_for_executable(exe: &Path) -> Option<PathBuf> {
    let macos_dir = exe.parent()?;
    if macos_dir.file_name()? != "MacOS" {
        return None;
    }

    let contents_dir = macos_dir.parent()?;
    if contents_dir.file_name()? != "Contents" {
        return None;
    }

    let app_dir = contents_dir.parent()?;
    if app_dir.extension()? != std::ffi::OsStr::new("app") {
        return None;
    }

    Some(app_dir.to_path_buf())
}

fn network_controller_binary() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("CATPANE_NETWORK_CTL") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    let executable = std::env::current_exe().ok()?;
    let app_dir = bundled_app_dir_for_executable(&executable)?;
    let contents_dir = app_dir.join("Contents");
    let candidates = [
        contents_dir
            .join("MacOS")
            .join("CatPaneThrottlingController"),
        contents_dir.join("MacOS").join("catpane"),
        contents_dir.join("MacOS").join("catpane-network-ctl"),
        contents_dir
            .join("Helpers")
            .join("CatPaneThrottlingController"),
        contents_dir.join("Helpers").join("catpane-network-ctl"),
    ];
    candidates.into_iter().find(|candidate| candidate.exists())
}

fn network_controller_command<I, S>(
    args: I,
    context: &'static str,
) -> Result<OneShotCommand, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let Some(binary) = network_controller_binary() else {
        return Err(
            "iOS Simulator network throttling requires the bundled catpane-network-ctl helper, but no helper binary was found in the current app bundle. Rebuild CatPane with the native macOS network scaffold."
                .to_string(),
        );
    };
    Ok(OneShotCommand::new(
        binary.to_string_lossy().into_owned(),
        args,
        context,
        IOS_NETWORK_CONTROL_TIMEOUT,
    ))
}

fn render_controller_message(output: &std::process::Output, fallback: &str) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !stdout.is_empty() {
        stdout
    } else if !stderr.is_empty() {
        stderr
    } else {
        fallback.to_string()
    }
}

async fn run_network_controller<I, S>(
    args: I,
    context: &'static str,
    success_fallback: String,
) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let command = network_controller_command(args, context)?;
    let output = command.run().await?;

    if output.status.success() {
        Ok(render_controller_message(&output, &success_fallback))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(command.status_error(&output))
        } else {
            Err(stderr)
        }
    }
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

const SIMCTL_LOCATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Set the GPS location of a booted iOS Simulator.
pub async fn set_simulator_location(udid: &str, lat: f64, lon: f64) -> Result<String, String> {
    let coord = format!("{},{}", lat, lon);
    let cmd = xcrun_command(
        ["simctl", "location", udid, "set", &coord],
        "set simulator location",
        SIMCTL_LOCATION_TIMEOUT,
    );
    let output = cmd.run().await?;
    cmd.ensure_success(output)?;
    Ok(format!("Location set to {}, {} on {}", lat, lon, udid))
}

/// Clear any spoofed GPS location on a booted iOS Simulator.
pub async fn clear_simulator_location(udid: &str) -> Result<String, String> {
    let cmd = xcrun_command(
        ["simctl", "location", udid, "clear"],
        "clear simulator location",
        SIMCTL_LOCATION_TIMEOUT,
    );
    let output = cmd.run().await?;
    cmd.ensure_success(output)?;
    Ok(format!("Location cleared on {}", udid))
}

pub async fn set_simulator_network_condition(
    udid: &str,
    preset: NetworkConditionPreset,
) -> Result<String, String> {
    if !ios_network_throttling_enabled() {
        return Err(ios_network_throttling_gate_message());
    }
    run_network_controller(
        [
            "apply".to_string(),
            "--udid".to_string(),
            udid.to_string(),
            "--preset".to_string(),
            preset.slug().to_string(),
        ],
        "configuring iOS Simulator network throttling",
        format!("Applied {} network condition on {}", preset.label(), udid),
    )
    .await
}

pub async fn clear_simulator_network_condition(udid: &str) -> Result<String, String> {
    if !ios_network_throttling_enabled() {
        return Err(ios_network_throttling_gate_message());
    }
    run_network_controller(
        ["clear".to_string(), "--udid".to_string(), udid.to_string()],
        "clearing iOS Simulator network throttling",
        format!("Cleared network throttling on {}", udid),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{bundled_app_dir_for_executable, parse_simulators};
    use std::path::Path;

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

    #[test]
    fn resolves_app_bundle_from_executable() {
        let app_dir = bundled_app_dir_for_executable(Path::new(
            "/Applications/CatPane.app/Contents/MacOS/catpane",
        ))
        .unwrap();
        assert_eq!(app_dir, Path::new("/Applications/CatPane.app"));
    }
}
