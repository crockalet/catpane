use std::collections::HashMap;

use serde::Deserialize;
use tokio::process::Command;

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

pub async fn list_available_simulators() -> Vec<IosSimulator> {
    let output = match Command::new("xcrun")
        .args(["simctl", "list", "devices", "--json"])
        .output()
        .await
    {
        Ok(output) => output,
        Err(_) => return Vec::new(),
    };

    let parsed: SimctlDevices = match serde_json::from_slice(&output.stdout) {
        Ok(parsed) => parsed,
        Err(_) => return Vec::new(),
    };

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
    simulators
}

pub async fn list_booted_simulators() -> Vec<IosSimulator> {
    list_available_simulators()
        .await
        .into_iter()
        .filter(|simulator| simulator.state == "Booted")
        .collect()
}

pub async fn boot_simulator(udid: &str) -> Result<String, String> {
    let output = Command::new("xcrun")
        .args(["simctl", "bootstatus", udid, "-b"])
        .output()
        .await
        .map_err(|err| format!("Failed to run simctl bootstatus: {err}"))?;

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
        if stderr.is_empty() {
            Err(format!("Failed to boot simulator {udid}"))
        } else {
            Err(stderr)
        }
    } else {
        let simulators = list_available_simulators().await;
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

        let open_result = Command::new("open")
            .args(["-a", "Simulator", "--args", "-CurrentDeviceUDID", udid])
            .output()
            .await
            .map_err(|err| format!("Simulator booted, but opening Simulator.app failed: {err}"))?;

        if !open_result.status.success() {
            let open_stderr = String::from_utf8_lossy(&open_result.stderr)
                .trim()
                .to_string();
            return Err(if open_stderr.is_empty() {
                format!("Simulator booted, but Simulator.app could not be opened for {udid}")
            } else {
                format!("Simulator booted, but opening Simulator.app failed: {open_stderr}")
            });
        }

        Ok(format!(
            "Booted {} and opened Simulator.app",
            simulator.name
        ))
    }
}
