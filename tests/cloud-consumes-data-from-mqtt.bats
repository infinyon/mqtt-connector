#!/usr/bin/env bats

load './bats-helpers/bats-support/load'
load './bats-helpers/bats-assert/load'

setup() {
    FILE=$(mktemp)
    cp ./tests/cloud-consumes-data-from-mqtt.yaml $FILE
    UUID=$(uuidgen | awk '{print tolower($0)}')
    TOPIC=${UUID}-topic
    CONNECTOR=${UUID}-cloud-consumes-data-from-mqtt

    sed -i.BAK "s/CONNECTOR/${CONNECTOR}/g" $FILE
    sed -i.BAK "s/TOPIC/${TOPIC}/g" $FILE
    cat $FILE

    fluvio cloud login --email ${FLUVIO_CLOUD_TEST_USERNAME} --password ${FLUVIO_CLOUD_TEST_PASSWORD} --remote 'https://dev.infinyon.cloud'
    fluvio topic create $TOPIC
    fluvio cloud connector create --config $FILE

    CONNECTOR_PID=$!
}

teardown() {
    fluvio cloud connector delete $CONNECTOR
    kill $CONNECTOR_PID
}

@test "cloud-consumes-data-from-mqtt" {
    echo "Starting consumer on topic $TOPIC"
    echo "Using connector $CONNECTOR"
    sleep 15

    # We are using the same Topic for MQTT as for Connector
    mosquitto_pub -h test.mosquitto.org -t $TOPIC -m '{"device": {"device_id":"$UUID", "name":"device17"}}'
    sleep 35

    echo $(fluvio consume $TOPIC -B -d | jq .mqtt_topic)
    assert_success
}
