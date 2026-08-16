.PHONY: help build test clippy fmt fmt-check validate lint clean install-hooks

help:
	@echo "agents-of-empires Makefile"
	@echo ""
	@echo "Usage:"
	@echo "  make build          - Build the release binaries"
	@echo "  make test           - Run the workspace test suite"
	@echo "  make clippy         - Run Clippy across every target"
	@echo "  make fmt            - Format the workspace"
	@echo "  make fmt-check      - Check formatting without changing files"
	@echo "  make validate       - Validate the bundled example arena"
	@echo "  make lint           - Run the complete local CI gate"
	@echo "  make clean          - Remove Cargo build artifacts"
	@echo "  make install-hooks  - Install the pre-push lint hook"

build:
	cargo build --release --workspace

test:
	cargo test --workspace

clippy:
	cargo clippy --workspace --all-targets

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

validate:
	cargo run --quiet --bin agents-of-empires -- arena validate examples/hello-service-arena

lint: fmt-check clippy test validate

clean:
	cargo clean

install-hooks:
	@mkdir -p .git/hooks
	@printf '#!/usr/bin/env bash\nset -e\nexec make lint\n' > .git/hooks/pre-push
	@chmod +x .git/hooks/pre-push
	@echo "Installed pre-push hook -> make lint"
