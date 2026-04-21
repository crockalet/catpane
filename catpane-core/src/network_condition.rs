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

#[cfg(test)]
mod tests {
    use super::NetworkConditionPreset;

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
