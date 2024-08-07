mod config;
mod error;
mod event;
mod formatter;
mod source;

use config::MqttConfig;

use fluvio::{RecordKey, TopicProducerPool};
use fluvio_connector_common::{
    connector,
    tracing::{debug, trace},
    Result, Source,
};
use futures::StreamExt;
use source::MqttSource;

#[connector(source)]
async fn start(config: MqttConfig, producer: TopicProducerPool) -> Result<()> {
    debug!(?config);
    let source = MqttSource::new(&config)?;
    let mut stream = source.connect(None).await?;
    while let Some(item) = stream.next().await {
        trace!(?item);
        producer.send(RecordKey::NULL, item).await?;
    }
    Ok(())
}
