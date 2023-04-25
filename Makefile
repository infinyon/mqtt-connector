integration_tests:
	cdk build
	RUST_LOG=warn,integration_tests=info cargo run --manifest-path ./integration-tests/Cargo.toml

cloud_e2e_test:
	bats ./tests/cloud-consumes-data-from-mqtt.bats

test_fluvio_install:
	sleep 10
	fluvio version
	fluvio topic list
	fluvio topic create foobar
	sleep 3
	echo foo | fluvio produce foobar
	fluvio consume foobar -B -d
