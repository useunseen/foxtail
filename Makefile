# Makefile for AWS Mock Data Service (Rust)

BIN := target/debug/aws-mock-data-service
DB_PATH := mock_data.db

.PHONY: build build-release run gen gen-baseline gen-spike gen-idle-heavy serve setup setup-mock verify-cli-interoperability help

help:
	@echo "AWS Mock Data Service"
	@echo "  make build          - Build debug binary"
	@echo "  make build-release  - Build release binary"
	@echo "  make gen    - Discover resources and generate mock data"
	@echo "  make gen-baseline   - Regenerate data with Baseline scenario"
	@echo "  make gen-spike      - Regenerate data with Spike scenario"
	@echo "  make gen-idle-heavy - Regenerate data with IdleHeavy scenario"
	@echo "  make serve  - Start the API server"
	@echo "  make setup  - Build and generate data"
	@echo "  make verify-cli-interoperability - Run AWS CLI smoke checks against a local server"

build:
	cargo build

build-release:
	cargo build --release

gen:
	DATABASE_URL="sqlite:$(DB_PATH)" ./$(BIN) gen --prune

gen-baseline:
	DATABASE_URL="sqlite:$(DB_PATH)" ./$(BIN) gen --prune --scenario baseline

gen-spike:
	DATABASE_URL="sqlite:$(DB_PATH)" ./$(BIN) gen --prune --scenario spike

gen-idle-heavy:
	DATABASE_URL="sqlite:$(DB_PATH)" ./$(BIN) gen --prune --scenario idle-heavy

serve:
	DATABASE_URL="sqlite:$(DB_PATH)" ./$(BIN) serve --port 8080

setup: build gen

# Compatibility alias so users can run this target from service dir or repo root.
setup-mock: setup

verify-cli-interoperability:
	bash scripts/verify_cli_interop.sh
