use std::time::{Duration, Instant};

use crate::formatter::{self, Formatter};
use crate::{config::MqttConfig, error::MqttConnectorError, event::MqttEvent};
use anyhow::{Context, Result};
use async_std::channel::{self, Receiver, Sender};
use async_std::task::spawn;
use async_trait::async_trait;
use fluvio::Offset;
use fluvio_connector_common::tracing::info;
use fluvio_connector_common::{
    tracing::{error, warn},
    Source,
};
use futures::{stream::LocalBoxStream, StreamExt};
use rumqttc::{AsyncClient, EventLoop, MqttOptions, QoS, Transport};
use rustls::ClientConfig;
use url::Url;

const CHANNEL_BUFFER_SIZE: usize = 10000;
const MQTT_CLIENT_BUFFER_SIZE: usize = 10;
const MIN_LOG_WARN_TIME: Duration = Duration::from_secs(5 * 60);

pub(crate) struct MqttSource {
    formatter: Box<dyn Formatter + Sync + Send>,
    options: MqttOptions,
    topic: String,
    qos: QoS,
}

impl MqttSource {
    pub(crate) fn new(config: &MqttConfig) -> Result<Self> {
        let mut url =
            Url::parse(&config.url).context("unable to parse mqtt broker endpoint url")?;

        if !url.query_pairs().any(|(key, _)| key == "client_id") {
            url.query_pairs_mut()
                .append_pair("client_id", &config.client_id);
        }

        {
            let mut url_without_password = url.clone();
            let _ = url_without_password.set_password(None);
            info!(
                timeout=?config.timeout,
                mqtt_url=%url_without_password,
                %config.topic,
                %config.client_id
            );
        }
        let mut options = MqttOptions::try_from(url.clone())?;
        options.set_keep_alive(config.timeout);
        if url.scheme() == "mqtts" || url.scheme() == "ssl" {
            info!("using tls");
            let mut root_cert_store = rustls::RootCertStore::empty();
            for cert in
                rustls_native_certs::load_native_certs().context("could not load platform certs")?
            {
                root_cert_store
                    .add(&rustls::Certificate(cert.0))
                    .context("Failed to parse DER")?;
            }
            let client_config = ClientConfig::builder()
                .with_safe_defaults()
                .with_root_certificates(root_cert_store)
                .with_no_client_auth();

            options.set_transport(Transport::tls_with_config(client_config.into()));
        }
        let formatter = formatter::from_output_type(&config.payload_output_type);
        let topic = config.topic.clone();
        let qos = QoS::AtMostOnce;
        Ok(Self {
            formatter,
            options,
            topic,
            qos,
        })
    }
}

#[async_trait]
impl<'a> Source<'a, String> for MqttSource {
    async fn connect(self, _offset: Option<Offset>) -> Result<LocalBoxStream<'a, String>> {
        let (client, event_loop) = AsyncClient::new(self.options, MQTT_CLIENT_BUFFER_SIZE);
        client.subscribe(self.topic, self.qos).await?;
        let (sender, receiver) = channel::bounded(CHANNEL_BUFFER_SIZE);
        spawn(mqtt_loop(
            sender,
            receiver.clone(),
            event_loop,
            self.formatter,
        ));
        Ok(receiver.boxed_local())
    }
}

async fn mqtt_loop(
    tx: Sender<String>,
    rx: Receiver<String>,
    mut event_loop: EventLoop,
    formatter: Box<dyn Formatter + Sync + Send>,
) -> Result<(), MqttConnectorError> {
    let mut last_warn = Instant::now();
    let mut num_dropped_messages = 0u64;
    loop {
        // eventloop.poll() docs state "Don't block while iterating"
        let notification = match event_loop.poll().await {
            Ok(notification) => notification,
            Err(e) => {
                error!("Mqtt error {}. Finishing mqtt loop", e);
                tx.close();
                return Err(MqttConnectorError::MqttConnection(e));
            }
        };

        if let Ok(mqtt_event) = MqttEvent::try_from(notification) {
            if tx.is_full() {
                num_dropped_messages += 1;
                let elapsed = last_warn.elapsed();
                if elapsed > MIN_LOG_WARN_TIME {
                    warn!("Queue backed up. Dropped {num_dropped_messages} mqtt messages in last {elapsed:?}");
                    last_warn = Instant::now();
                    num_dropped_messages = 0;
                }

                _ = rx.try_recv()
            }
            let formatted = match formatter.to_string(&mqtt_event) {
                Ok(s) => s,
                Err(_) => {
                    num_dropped_messages += 1;
                    let elapsed = last_warn.elapsed();
                    if elapsed > MIN_LOG_WARN_TIME {
                        warn!("Failed to format message. Dropped {num_dropped_messages} failed to parse messages in last {elapsed:?}");
                        last_warn = Instant::now();
                        num_dropped_messages = 0;
                    };
                    continue;
                }
            };
            match tx.try_send(formatted) {
                Ok(_) => {}
                Err(e) => match e {
                    async_std::channel::TrySendError::Full(_) => {
                        unreachable!(); // there is only one sender and here we remove a record if
                                        // full before sending
                    }
                    async_std::channel::TrySendError::Closed(_) => {
                        error!("Channel closed. Finishing mqtt loop");
                        return Err(MqttConnectorError::ChannelClosed);
                    }
                },
            }
        }
    }
}
