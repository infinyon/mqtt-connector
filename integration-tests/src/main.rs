use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Context, Result};
use async_std::{channel::bounded, stream::StreamExt, task};
use bollard::{
    container::{Config, CreateContainerOptions, RemoveContainerOptions, StartContainerOptions},
    image::CreateImageOptions,
    service::{HostConfig, PortBinding},
    Docker,
};
use futures_util::stream::TryStreamExt;
use log::{debug, info};
use serde::{Deserialize, Serialize};

use fluvio::{
    consumer::{ConsumerConfigExt, Record},
    metadata::topic::TopicSpec,
    Fluvio, Offset,
};
use fluvio_future::retry::{retry, ExponentialBackoff, RetryExt};

const MOSQUITTO_IMAGE: &str = "eclipse-mosquitto:1.6.1";
const MQTT_HOST_PORT: &str = "1883";

#[async_std::main]
async fn main() -> Result<()> {
    env_logger::init();

    info!("preparing environment");
    let docker = connect_docker()
        .await
        .context("unable to connect to docker engine")?;

    let fluvio = connect_fluvio()
        .await
        .context("unable to connect to fluvio cluster")?;

    run_mosquitto(&docker)
        .await
        .context("unable to run mosquitto container")?;

    info!("running mqtt-connector integration tests");
    let result1 = test_env_secret_json_output_type(&fluvio)
        .await
        .context("test_env_secret_json_output_type failed");

    let result2 = test_env_secret_binary_output_type(&fluvio)
        .await
        .context("test_env_secret_binary_output_type failed");

    let _ = remove_mosquitto(&docker).await;
    result1?;
    result2?;
    Ok(())
}

async fn test_env_secret_json_output_type(fluvio: &Fluvio) -> Result<()> {
    // given
    info!("running 'test_env_secret_json_output_type' test");
    let config_path = new_config_path("test_env_secret_json_output_type.yaml")?;
    debug!("{config_path:?}");
    let config: TestConfig = serde_yaml::from_reader(std::fs::File::open(&config_path)?)?;
    cdk_deploy_start(&config_path, Some(("MQTT_URL", "mqtt://127.0.0.1/"))).await?;
    let connector_name = &config.meta.name;
    let connector_status = cdk_deploy_status(connector_name)?;
    info!("connector: {connector_name}, status: {connector_status:?}");

    let count = 10;
    let records = generate_records(count)?;

    produce_to_mqtt(&config.mqtt.topic, records.clone()).await?;

    // when
    let received_records = read_from_fluvio(fluvio, &config.meta.topic, count).await?;
    cdk_deploy_shutdown(connector_name)?;
    remove_topic(fluvio, &config.meta.topic).await?;

    // then
    #[derive(Deserialize)]
    struct JsonRecord {
        mqtt_topic: String,
        payload: JsonPayload,
    }

    #[derive(Serialize, Deserialize, PartialEq, Eq)]
    struct JsonPayload {
        timestamp: u128,
    }

    let json_records = received_records
        .into_iter()
        .map(|r| serde_json::from_slice(r.value()))
        .collect::<std::result::Result<Vec<JsonRecord>, _>>()?;

    for json in json_records.iter() {
        if !json.mqtt_topic.eq(&config.mqtt.topic) {
            anyhow::bail!(
                "expected: {}, got: {}",
                &config.mqtt.topic,
                &json.mqtt_topic
            );
        }
    }

    let unpacked_received: Vec<String> = json_records
        .into_iter()
        .map(|r| serde_json::to_string(&r.payload))
        .collect::<Result<Vec<String>, _>>()?;

    if !records.eq(&unpacked_received) {
        anyhow::bail!("expected: {records:?}, got: {unpacked_received:?}");
    }

    info!("test 'test_env_secret_json_output_type' passed");
    Ok(())
}

async fn test_env_secret_binary_output_type(fluvio: &Fluvio) -> Result<()> {
    // given
    info!("running 'test_env_secret_binary_output_type' test");
    let config_path = new_config_path("test_env_secret_binary_output_type.yaml")?;
    debug!("{config_path:?}");
    let config: TestConfig = serde_yaml::from_reader(std::fs::File::open(&config_path)?)?;
    cdk_deploy_start(&config_path, Some(("MQTT_URL", "mqtt://127.0.0.1/"))).await?;
    let connector_name = &config.meta.name;
    let connector_status = cdk_deploy_status(connector_name)?;
    info!("connector: {connector_name}, status: {connector_status:?}");

    let count = 10;
    let records = generate_records(count)?;

    produce_to_mqtt(&config.mqtt.topic, records.clone()).await?;

    // when
    let received_records = read_from_fluvio(fluvio, &config.meta.topic, count).await?;
    cdk_deploy_shutdown(connector_name)?;
    remove_topic(fluvio, &config.meta.topic).await?;

    // then
    #[derive(Deserialize)]
    struct JsonRecord {
        mqtt_topic: String,
        payload: Vec<u8>,
    }

    #[derive(Serialize, Deserialize, PartialEq, Eq)]
    struct BinaryPayload {
        timestamp: u128,
    }

    let json_records = received_records
        .into_iter()
        .map(|r| serde_json::from_slice(r.value()))
        .collect::<std::result::Result<Vec<JsonRecord>, _>>()?;

    for json in json_records.iter() {
        if !json.mqtt_topic.eq(&config.mqtt.topic) {
            anyhow::bail!(
                "expected: {}, got: {}",
                &config.mqtt.topic,
                &json.mqtt_topic
            );
        }
    }

    let unpacked_received: Vec<String> = json_records
        .into_iter()
        .map(|r| String::from_utf8(r.payload))
        .collect::<Result<Vec<String>, _>>()?;

    if !records.eq(&unpacked_received) {
        anyhow::bail!("expected: {records:?}, got: {unpacked_received:?}");
    }

    info!("test 'test_env_secret_json_output_type' passed");
    Ok(())
}

async fn connect_docker() -> Result<Docker> {
    info!("checking docker engine availability");

    let docker = Docker::connect_with_local_defaults()?;
    let version = docker.version().await?;
    info!(
        "connected to docker version: {:?}, api_version: {:?}",
        &version.version, &version.api_version
    );
    Ok(docker)
}

async fn run_mosquitto(docker: &Docker) -> Result<()> {
    info!("starting mosquitto container");

    let config = Config {
        image: Some(MOSQUITTO_IMAGE),
        exposed_ports: Some(HashMap::from([(MQTT_HOST_PORT, Default::default())])),
        host_config: Some(HostConfig {
            port_bindings: Some(HashMap::from([(
                "1883".to_owned(),
                Some(vec![PortBinding {
                    host_ip: Some("0.0.0.0".to_owned()),
                    host_port: Some(MQTT_HOST_PORT.to_owned()),
                }]),
            )])),
            ..Default::default()
        }),
        ..Default::default()
    };
    let _ = &docker
        .create_image(
            Some(CreateImageOptions {
                from_image: MOSQUITTO_IMAGE,
                ..Default::default()
            }),
            None,
            None,
        )
        .try_collect::<Vec<_>>()
        .await?;

    let _ = &docker
        .create_container(
            Some(CreateContainerOptions {
                name: "mqtt",
                platform: None,
            }),
            config,
        )
        .await?;

    let _ = &docker
        .start_container("mqtt", None::<StartContainerOptions<String>>)
        .await?;

    info!("mosquitto container created, waiting for readiness");
    retry(ExponentialBackoff::from_millis(100).take(4), || {
        produce_to_mqtt("test_connection", vec!["ping"])
    })
    .await?;

    info!("mosquitto container started with {MOSQUITTO_IMAGE} image");

    Ok(())
}

async fn remove_mosquitto(docker: &Docker) -> Result<()> {
    let _ = &docker
        .remove_container(
            "mqtt",
            Some(RemoveContainerOptions {
                v: true,
                force: true,
                ..Default::default()
            }),
        )
        .await?;
    info!("mosquitto container removed");
    Ok(())
}

async fn connect_fluvio() -> Result<Fluvio> {
    info!("checking fluvio cluster availability");
    let fluvio = fluvio::Fluvio::connect().await?;
    info!("connected to fluvio version: {}", fluvio.platform_version());
    Ok(fluvio)
}

async fn remove_topic(fluvio: &Fluvio, topic: &str) -> Result<()> {
    fluvio.admin().await.delete::<TopicSpec>(topic).await?;
    Ok(())
}

async fn cdk_deploy_start(config_path: &Path, env: Option<(&str, &str)>) -> Result<()> {
    info!("deploying connector with config from {config_path:?}");
    let mut command = Command::new("cdk");
    command
        .arg("deploy")
        .arg("start")
        .arg("--config")
        .arg(config_path);
    if let Some((env_name, env_value)) = env {
        command.env(env_name, env_value);
    }
    let output = command.output()?;
    if !output.status.success() {
        anyhow::bail!(
            "`cdk deploy start` failed with:\n {}",
            String::from_utf8_lossy(output.stderr.as_slice())
        )
    }
    task::sleep(Duration::from_secs(10)).await; // time for connector to start
    Ok(())
}

fn cdk_deploy_shutdown(connector_name: &str) -> Result<()> {
    info!("shutting down connector {connector_name}");
    let output = Command::new("cdk")
        .arg("deploy")
        .arg("shutdown")
        .arg("--name")
        .arg(connector_name)
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "`cdk deploy shutdown` failed with:\n {}",
            String::from_utf8_lossy(output.stderr.as_slice())
        )
    }
    Ok(())
}

fn cdk_deploy_status(connector_name: &str) -> Result<Option<String>> {
    let output = Command::new("cdk").arg("deploy").arg("list").output()?;
    if !output.status.success() {
        anyhow::bail!(
            "`cdk deploy list` failed with:\n {}",
            String::from_utf8_lossy(output.stderr.as_slice())
        )
    }
    for line in String::from_utf8_lossy(output.stdout.as_slice())
        .lines()
        .skip(1)
    {
        let mut column_iter = line.split_whitespace();
        match column_iter.next() {
            Some(name) if name.eq(connector_name) => {
                return Ok(column_iter.next().map(|s| s.to_owned()))
            }
            _ => {}
        }
    }
    Ok(None)
}

async fn produce_to_mqtt<V: Into<Vec<u8>> + std::fmt::Debug + Send + Sync + 'static>(
    mqtt_topic: &str,
    records: Vec<V>,
) -> Result<()> {
    let mut mqttoptions = rumqttc::MqttOptions::new("rumqtt-async-client", "127.0.0.1", 1883);
    mqttoptions.set_keep_alive(Duration::from_secs(5));
    let (client, mut eventloop) = rumqttc::AsyncClient::new(mqttoptions, 10);
    let topic = mqtt_topic.to_owned();
    let (s, r) = bounded(1);
    task::spawn(async move {
        let len = records.len();
        debug!("{records:?}");
        for record in records {
            if let Err(err) = client
                .publish(&topic, rumqttc::QoS::ExactlyOnce, false, record.into())
                .await
            {
                debug!("unable to publish record: {}", err);
            };
            task::sleep(Duration::from_millis(100)).await;
        }
        info!("sent to mqtt {len} records");
        if let Err(err) = s.send(()).await {
            debug!("unable to sent to channel: {}", err);
        }
    });
    while r.is_empty() {
        eventloop.poll().await?;
    }
    let started = Instant::now();
    while started.elapsed().as_secs() < 5 {
        eventloop.poll().await?;
    }
    info!("mqtt event loop finished");
    Ok(())
}

async fn read_from_fluvio(fluvio: &Fluvio, topic: &str, count: usize) -> Result<Vec<Record>> {
    let consumer_config = ConsumerConfigExt::builder()
        .topic(topic)
        .offset_start(Offset::beginning())
        .build()?;
    let stream = fluvio
        .consumer_with_config(consumer_config)
        .await?
        .take(count);
    let stream_collected = stream
        .collect::<Vec<_>>()
        .timeout(Duration::from_secs(30))
        .await
        .context("unable to read {count} records from Fluvio in 30 seconds")?;
    let result: Result<Vec<Record>, _> = stream_collected.into_iter().collect();
    Ok(result?)
}

#[derive(Debug, Deserialize)]
struct MetaConfig {
    name: String,
    topic: String,
}

#[derive(Debug, Deserialize)]
struct MqttConfig {
    topic: String,
}

#[derive(Debug, Deserialize)]
struct TestConfig {
    meta: MetaConfig,
    mqtt: MqttConfig,
}

fn generate_records(count: usize) -> Result<Vec<String>> {
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_millis();
    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        result.push(format!("{{\"timestamp\":{}}}", timestamp + (i as u128)));
    }
    Ok(result)
}

fn new_config_path(name: &str) -> Result<PathBuf> {
    let package_dir = std::env::var("CARGO_MANIFEST_DIR")?;
    let mut path = PathBuf::new();
    path.push(package_dir);
    path.push("configs");
    path.push(name);
    Ok(path)
}
