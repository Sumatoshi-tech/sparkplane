.PHONY: build test lint lint-spark test-client fmt-check audit update web-build web-check web-test web-test-browser
.DEFAULT_GOAL := build
web/node_modules/.sparkplane-deps: web/package.json web/package-lock.json
	cd web && npm ci --no-fund
	touch web/node_modules/.sparkplane-deps
web-build: web/node_modules/.sparkplane-deps
	cd web && npm run build
web-check: web/node_modules/.sparkplane-deps
	cd web && npm run check
web-test: web/node_modules/.sparkplane-deps
	cd web && npm test
web-test-browser: web-build
	cd web && npm run test:browser
	cargo test --locked --features appliance --lib browser_authentication_redirects_over_https -- --ignored
build: web-build
	cargo build --locked --release --features appliance
test: web-build web-test
	python3 tests/standalone_contract.py
	python3 tests/spark_update.py
	cargo test --locked --workspace --all-targets --features appliance
test-client:
	cargo test --locked --no-default-features --all-targets
lint: web-build web-check
	cargo fmt --all -- --check
	cargo clippy --locked --workspace --all-targets --features appliance -- -D warnings
	cargo clippy --locked --no-default-features --all-targets -- -D warnings
lint-spark: lint
fmt-check:
	cargo fmt --all -- --check
audit:
	cargo deny --all-features check
HOST ?= dgx-spark
update:
	@python3 scripts/update-spark.py "$(HOST)"
