#!/usr/bin/env bats

load './bats-helpers/bats-support/load'
load './bats-helpers/bats-assert/load'

setup() {
    FILE=$(mktemp)
    cp ./tests/cloud-consumes-data-from-mqtt.yaml $FILE
    UUID=$(uuidgen | awk '{print tolower($0)}')
    TOPIC=${UUID}-topic
    CONNECTOR=${UUID}-consume

    sed -i.BAK "s/CONNECTOR/${CONNECTOR}/g" $FILE
    sed -i.BAK "s/TOPIC/${TOPIC}/g" $FILE
    cat $FILE

    fluvio cloud login --email ${CLOUD_USER_EMAIL} --password ${CLOUD_USER_PASSWORD}
    fluvio cloud cluster sync
    fluvio topic create $TOPIC
    fluvio cloud connector create --config $FILE
}

teardown() {
    fluvio cloud connector delete $CONNECTOR
}

@test "cloud-consumes-data-from-mqtt" {
    echo "Starting consumer on topic $TOPIC"
    echo "Using connector $CONNECTOR"
    sleep 45

    echo "Pre-check Connectors Statuses"
    fluvio cloud connector list

    echo "Initializing periodic status check"
    for i in {0..6}
    do
        if fluvio cloud connector list | sed 1d | grep "$CONNECTOR" | grep "Running" ; then
            echo "Connector $CONNECTOR is already Running!"
            break
        else
            echo "Attempt $i, not Running yet. Retrying after sleep"
            sleep 30
        fi
    done

    echo "Check connector logs"
    fluvio cloud connector logs $CONNECTOR || true

    echo "Check connector is status before testing"
    fluvio cloud connector list

    echo "Executing tests after cooldown"
    sleep 15

    # We are using the same Topic for MQTT as for Connector
    mosquitto_pub -h test.mosquitto.org -t $TOPIC -m '{"device": {"device_id":"$UUID", "name":"device17"}}'
    sleep 60

    fluvio consume $TOPIC -B -d | jq .mqtt_topic
    assert_success
}
