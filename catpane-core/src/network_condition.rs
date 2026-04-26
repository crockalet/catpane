use serde::{Deserialize, Serialize};

pub const IOS_NETWORK_THROTTLING_ENV: &str = "CATPANE_ENABLE_IOS_NETWORK_THROTTLING";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NetworkConditionPreset {
    #[serde(rename = "unthrottled")]
    Unthrottled,
    #[serde(rename = "edge")]
    Edge,
    #[serde(rename = "3g")]
    ThreeG,
    #[serde(rename = "offline")]
    Offline,
}

impl NetworkConditionPreset {
    pub const ALL: [Self; 4] = [Self::Unthrottled, Self::Edge, Self::ThreeG, Self::Offline];

    pub fn slug(self) -> &'static str {
        match self {
            Self::Unthrottled => "unthrottled",
            Self::Edge => "edge",
            Self::ThreeG => "3g",
            Self::Offline => "offline",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unthrottled => "Unthrottled",
            Self::Edge => "Edge",
            Self::ThreeG => "3G",
            Self::Offline => "Offline",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "unthrottled" | "full" | "none" => Ok(Self::Unthrottled),
            "edge" => Ok(Self::Edge),
            "3g" | "three-g" | "three_g" | "umts" => Ok(Self::ThreeG),
            "offline" | "airplane" | "airplane-mode" | "airplane_mode" => Ok(Self::Offline),
            _ => Err(format!(
                "Unsupported network condition '{value}'. Expected one of: {}",
                Self::ALL
                    .iter()
                    .map(|preset| preset.slug())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    pub fn android_profile(self) -> AndroidEmulatorNetworkProfile {
        match self {
            Self::Unthrottled => AndroidEmulatorNetworkProfile {
                speed: "full",
                delay: "none",
                wifi_enabled: true,
                data_enabled: true,
            },
            Self::Edge => AndroidEmulatorNetworkProfile {
                speed: "edge",
                delay: "edge",
                wifi_enabled: true,
                data_enabled: true,
            },
            Self::ThreeG => AndroidEmulatorNetworkProfile {
                speed: "umts",
                delay: "umts",
                wifi_enabled: true,
                data_enabled: true,
            },
            Self::Offline => AndroidEmulatorNetworkProfile {
                speed: "full",
                delay: "none",
                wifi_enabled: false,
                data_enabled: false,
            },
        }
    }
}

pub fn ios_network_throttling_enabled() -> bool {
    matches!(
        std::env::var(IOS_NETWORK_THROTTLING_ENV)
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

pub fn ios_network_throttling_gate_message() -> String {
    format!(
        "iOS Simulator network throttling is disabled by default until CatPane ships a properly signed Network Extension build. Set {IOS_NETWORK_THROTTLING_ENV}=1 to re-enable it for local testing."
    )
}

impl std::fmt::Display for NetworkConditionPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.slug())
    }
}

impl std::str::FromStr for NetworkConditionPreset {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AndroidEmulatorNetworkProfile {
    pub speed: &'static str,
    pub delay: &'static str,
    pub wifi_enabled: bool,
    pub data_enabled: bool,
}

/// User-tunable shaping parameters for the Android helper VPN.
///
/// All fields are optional in serde so callers can omit values they don't want
/// to constrain (e.g. only set `delay_ms` and leave bandwidth uncapped).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct CustomNetworkParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<u32>,
    /// Packet loss as a percentage in the range `0.0..=100.0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loss_pct: Option<f32>,
    /// Downlink (device-bound) bandwidth cap in kilobits per second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downlink_kbps: Option<u32>,
    /// Uplink (device-originated) bandwidth cap in kilobits per second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uplink_kbps: Option<u32>,
}

impl CustomNetworkParams {
    pub const MAX_DELAY_MS: u32 = 60_000;
    pub const MAX_JITTER_MS: u32 = 60_000;
    pub const MAX_BANDWIDTH_KBPS: u32 = 10_000_000;

    /// Returns true if every field is `None` (i.e. effectively unthrottled).
    pub fn is_empty(&self) -> bool {
        self.delay_ms.is_none()
            && self.jitter_ms.is_none()
            && self.loss_pct.is_none()
            && self.downlink_kbps.is_none()
            && self.uplink_kbps.is_none()
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(delay) = self.delay_ms
            && delay > Self::MAX_DELAY_MS
        {
            return Err(format!(
                "delay_ms must be <= {} (got {delay})",
                Self::MAX_DELAY_MS
            ));
        }
        if let Some(jitter) = self.jitter_ms
            && jitter > Self::MAX_JITTER_MS
        {
            return Err(format!(
                "jitter_ms must be <= {} (got {jitter})",
                Self::MAX_JITTER_MS
            ));
        }
        if let Some(loss) = self.loss_pct
            && !(0.0..=100.0).contains(&loss)
        {
            return Err(format!("loss_pct must be in 0.0..=100.0 (got {loss})"));
        }
        if let Some(loss) = self.loss_pct
            && !loss.is_finite()
        {
            return Err("loss_pct must be a finite number".to_string());
        }
        if let Some(down) = self.downlink_kbps
            && down > Self::MAX_BANDWIDTH_KBPS
        {
            return Err(format!(
                "downlink_kbps must be <= {} (got {down})",
                Self::MAX_BANDWIDTH_KBPS
            ));
        }
        if let Some(up) = self.uplink_kbps
            && up > Self::MAX_BANDWIDTH_KBPS
        {
            return Err(format!(
                "uplink_kbps must be <= {} (got {up})",
                Self::MAX_BANDWIDTH_KBPS
            ));
        }
        Ok(())
    }
}

/// A unified network-condition specification used by the UI, MCP layer, and
/// device dispatchers. Either a named preset or a fully custom shaping profile.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NetworkConditionSpec {
    Preset {
        preset: NetworkConditionPreset,
    },
    Custom {
        #[serde(flatten)]
        params: CustomNetworkParams,
    },
}

impl NetworkConditionSpec {
    pub fn preset(preset: NetworkConditionPreset) -> Self {
        Self::Preset { preset }
    }

    pub fn custom(params: CustomNetworkParams) -> Self {
        Self::Custom { params }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Preset { .. } => Ok(()),
            Self::Custom { params } => params.validate(),
        }
    }

    /// True when the spec corresponds to "no throttling at all".
    pub fn is_unthrottled(&self) -> bool {
        match self {
            Self::Preset { preset } => *preset == NetworkConditionPreset::Unthrottled,
            Self::Custom { params } => params.is_empty(),
        }
    }

    /// Short human-readable label for status surfaces (UI, MCP responses).
    pub fn label(&self) -> String {
        match self {
            Self::Preset { preset } => preset.label().to_string(),
            Self::Custom { .. } => "Custom".to_string(),
        }
    }
}

impl From<NetworkConditionPreset> for NetworkConditionSpec {
    fn from(preset: NetworkConditionPreset) -> Self {
        Self::Preset { preset }
    }
}

#[cfg(test)]
mod tests {
    use super::{CustomNetworkParams, NetworkConditionPreset, NetworkConditionSpec};

    #[test]
    fn parses_supported_slugs_and_aliases() {
        assert_eq!(
            NetworkConditionPreset::parse("unthrottled").unwrap(),
            NetworkConditionPreset::Unthrottled
        );
        assert_eq!(
            NetworkConditionPreset::parse("3g").unwrap(),
            NetworkConditionPreset::ThreeG
        );
        assert_eq!(
            NetworkConditionPreset::parse("airplane-mode").unwrap(),
            NetworkConditionPreset::Offline
        );
    }

    #[test]
    fn rejects_unknown_presets() {
        let err = NetworkConditionPreset::parse("satellite").unwrap_err();
        assert!(err.contains("Unsupported network condition"));
        assert!(err.contains("unthrottled"));
        assert!(err.contains("offline"));
    }

    #[test]
    fn custom_params_validates_bounds() {
        let mut params = CustomNetworkParams {
            delay_ms: Some(100),
            jitter_ms: Some(20),
            loss_pct: Some(5.0),
            downlink_kbps: Some(1_000),
            uplink_kbps: Some(500),
        };
        assert!(params.validate().is_ok());

        params.loss_pct = Some(150.0);
        assert!(params.validate().unwrap_err().contains("loss_pct"));

        params.loss_pct = Some(f32::NAN);
        assert!(params.validate().is_err());

        params.loss_pct = Some(0.0);
        params.delay_ms = Some(CustomNetworkParams::MAX_DELAY_MS + 1);
        assert!(params.validate().unwrap_err().contains("delay_ms"));
    }

    #[test]
    fn custom_params_is_empty_when_all_none() {
        assert!(CustomNetworkParams::default().is_empty());
        let p = CustomNetworkParams {
            delay_ms: Some(1),
            ..Default::default()
        };
        assert!(!p.is_empty());
    }

    #[test]
    fn spec_serde_round_trip() {
        let preset = NetworkConditionSpec::preset(NetworkConditionPreset::ThreeG);
        let json = serde_json::to_string(&preset).unwrap();
        assert!(json.contains("\"kind\":\"preset\""));
        assert!(json.contains("\"preset\":\"3g\""));
        let back: NetworkConditionSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, preset);

        let custom = NetworkConditionSpec::custom(CustomNetworkParams {
            delay_ms: Some(250),
            loss_pct: Some(2.5),
            ..Default::default()
        });
        let json = serde_json::to_string(&custom).unwrap();
        assert!(json.contains("\"kind\":\"custom\""));
        assert!(json.contains("\"delay_ms\":250"));
        // Unset fields should be omitted to keep the wire format compact.
        assert!(!json.contains("downlink_kbps"));
        let back: NetworkConditionSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, custom);
    }

    #[test]
    fn spec_is_unthrottled() {
        assert!(NetworkConditionSpec::preset(NetworkConditionPreset::Unthrottled).is_unthrottled());
        assert!(!NetworkConditionSpec::preset(NetworkConditionPreset::Edge).is_unthrottled());
        assert!(NetworkConditionSpec::custom(CustomNetworkParams::default()).is_unthrottled());
        assert!(
            !NetworkConditionSpec::custom(CustomNetworkParams {
                delay_ms: Some(10),
                ..Default::default()
            })
            .is_unthrottled()
        );
    }

    #[test]
    fn maps_android_profiles() {
        let edge = NetworkConditionPreset::Edge.android_profile();
        assert_eq!(edge.speed, "edge");
        assert_eq!(edge.delay, "edge");
        assert!(edge.wifi_enabled);
        assert!(edge.data_enabled);

        let offline = NetworkConditionPreset::Offline.android_profile();
        assert!(!offline.wifi_enabled);
        assert!(!offline.data_enabled);
    }
}
