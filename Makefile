integration_tests:
	cdk build
	RUST_LOG=warn,integration_tests=info cargo run --manifest-path ./integration-tests/Cargo.toml

