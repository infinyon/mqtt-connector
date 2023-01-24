use rumqttc::{Event, Packet};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MqttEvent {
    pub mqtt_topic: String,
    pub payload: Vec<u8>,
}
impl TryFrom<Event> for MqttEvent {
    type Error = &'static str;
    fn try_from(event: Event) -> Result<Self, Self::Error> {
        match event {
            Event::Incoming(Packet::Publish(p)) => Ok(Self {
                mqtt_topic: p.topic,
                payload: p.payload.to_vec(),
            }),
            _ => Err("unsupported event"),
        }
    }
}
