use std::time::Duration;

use fluvio_connector_common::connector;
use serde::Deserialize;

const DEFAULT_TIMEOUT_VALUE: Duration = Duration::from_secs(60);

#[connector(config, name = "mqtt")]
#[derive(Debug)]
pub(crate) struct MqttConfig {
    #[serde(default = "default_timeout")]
    pub timeout: Duration,
    pub url: String,
    pub topic: String,
    #[serde(default = "default_client_id")]
    pub client_id: String,
    #[serde(default)]
    pub payload_output_type: OutputType,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OutputType {
    #[default]
    Binary,
    Json,
}

fn default_timeout() -> Duration {
    DEFAULT_TIMEOUT_VALUE
}

fn default_client_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
