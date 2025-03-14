use std::time::Duration;

use fluvio_connector_common::{connector, secret::SecretString};
use serde::Deserialize;

const DEFAULT_TIMEOUT_VALUE: Duration = Duration::from_secs(60);
const MAX_INCOMING_PACKET_SIZE: usize = 10 * 1024;
const MAX_OUTGOING_PACKET_SIZE: usize = 10 * 1024;

#[connector(config, name = "mqtt")]
#[derive(Debug)]
pub(crate) struct MqttConfig {
    #[serde(default = "default_timeout")]
    pub timeout: Duration,
    pub url: SecretString,
    pub topic: String,
    #[serde(default = "default_client_id")]
    pub client_id: String,
    #[serde(default)]
    pub payload_output_type: OutputType,
    #[serde(default = "defualt_max_incoming_packet_size")]
    pub max_incoming_packet_size: usize,
    #[serde(default = "defualt_max_outgoing_packet_size")]
    pub max_outgoing_packet_size: usize,
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

fn defualt_max_incoming_packet_size() -> usize {
    MAX_INCOMING_PACKET_SIZE
}

fn defualt_max_outgoing_packet_size() -> usize {
    MAX_OUTGOING_PACKET_SIZE
}
