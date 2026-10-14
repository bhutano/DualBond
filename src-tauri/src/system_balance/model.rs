use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::{SystemTime, UNIX_EPOCH};

pub const CONFIG_SCHEMA: u32 = 1;
pub const CONTROL_PORT: u16 = 17_777;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LinkPreference {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub metered: bool,
}

fn default_true() -> bool {
    true
}

impl Default for LinkPreference {
    fn default() -> Self {
        Self {
            enabled: true,
            metered: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceConfig {
    pub schema: u32,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub allow_metered_fallback: bool,
    #[serde(default)]
    pub links: BTreeMap<String, LinkPreference>,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            schema: CONFIG_SCHEMA,
            enabled: false,
            allow_metered_fallback: false,
            links: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SystemLinkStatus {
    pub key: String,
    pub name: String,
    pub kind: String,
    pub if_index: u32,
    pub local_ip: IpAddr,
    pub enabled: bool,
    pub metered: bool,
    pub state: String,
    pub mbps: f64,
    pub avg_mbps: f64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub tcp_flows: u32,
    pub udp_flows: u32,
    pub errors: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SystemBalanceStatus {
    pub service_available: bool,
    pub enabled: bool,
    pub state: String,
    pub adapter_name: String,
    pub ipv4_only: bool,
    pub total_mbps: f64,
    pub total_avg_mbps: f64,
    pub tcp_flows: u32,
    pub udp_flows: u32,
    pub links: Vec<SystemLinkStatus>,
    pub last_error: Option<String>,
    pub updated_at: i64,
}

impl Default for SystemBalanceStatus {
    fn default() -> Self {
        Self::unavailable("Servizio DualBond non raggiungibile")
    }
}

impl SystemBalanceStatus {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            service_available: false,
            enabled: false,
            state: "unavailable".into(),
            adapter_name: "DualBond".into(),
            ipv4_only: true,
            total_mbps: 0.0,
            total_avg_mbps: 0.0,
            tcp_flows: 0,
            udp_flows: 0,
            links: Vec::new(),
            last_error: Some(message.into()),
            updated_at: unix_ms(),
        }
    }
}

pub fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "camelCase")]
pub enum ControlCommand {
    Status {
        token: String,
    },
    SetEnabled {
        token: String,
        enabled: bool,
    },
    Refresh {
        token: String,
    },
    SetLinkEnabled {
        token: String,
        key: String,
        enabled: bool,
    },
    SetLinkMetered {
        token: String,
        key: String,
        metered: bool,
    },
}

impl ControlCommand {
    pub fn token(&self) -> &str {
        match self {
            Self::Status { token }
            | Self::SetEnabled { token, .. }
            | Self::Refresh { token }
            | Self::SetLinkEnabled { token, .. }
            | Self::SetLinkMetered { token, .. } => token,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlResponse {
    pub ok: bool,
    pub status: SystemBalanceStatus,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_to_safe_disabled_mode() {
        let config = ServiceConfig::default();
        assert!(!config.enabled);
        assert!(!config.allow_metered_fallback);
        assert_eq!(config.schema, CONFIG_SCHEMA);
    }

    #[test]
    fn older_json_without_new_fields_migrates_through_defaults() {
        let config: ServiceConfig = serde_json::from_str(r#"{"schema":1,"enabled":true}"#).unwrap();
        assert!(config.enabled);
        assert!(config.links.is_empty());
        assert!(!config.allow_metered_fallback);
    }

    #[test]
    fn command_token_is_uniform_across_variants() {
        let command = ControlCommand::Refresh {
            token: "secret".into(),
        };
        assert_eq!(command.token(), "secret");
    }
}
