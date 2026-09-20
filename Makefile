.PHONY: build test lint lint-spark test-client fmt-check audit
build:
	cargo build --locked --release --features appliance
test:
	python3 tests/standalone_contract.py
	cargo test --locked --workspace --all-targets --features appliance
test-client:
	cargo test --locked --no-default-features --all-targets
lint:
	cargo fmt --all -- --check
	cargo clippy --locked --workspace --all-targets --features appliance -- -D warnings
	cargo clippy --locked --no-default-features --all-targets -- -D warnings
lint-spark: lint
fmt-check:
	cargo fmt --all -- --check
audit:
	cargo deny --all-features check
