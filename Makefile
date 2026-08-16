.PHONY: help version-bump release build demo demo-check test clippy fmt fmt-check validate lint clean install-hooks release-preflight

define get_next_version
$(shell \
	TODAY=$$(date +%Y%m%d); \
	LATEST=$$(git tag -l "v$$TODAY.*" 2>/dev/null | sort -V | tail -1); \
	if [ -z "$$LATEST" ]; then \
		echo "$$TODAY.0.0"; \
	else \
		PATCH=$$(echo "$$LATEST" | sed 's/.*\.0\.\([0-9]*\)/\1/'); \
		echo "$$TODAY.0.$$((PATCH + 1))"; \
	fi \
)
endef

VERSION := $(get_next_version)

help:
	@echo "agents-of-empires Makefile"
	@echo ""
	@echo "Usage:"
	@echo "  make release                       - Version, merge, tag, and push a release"
	@echo "  make release VERSION=20260816.0.0  - Release a specific version"
	@echo "  make build          - Build the release binaries"
	@echo "  make demo           - Run a free oracle race and generate its report"
	@echo "  make demo-check     - Check the demo launcher without running guests"
	@echo "  make test           - Run the workspace test suite"
	@echo "  make clippy         - Run Clippy across every target"
	@echo "  make fmt            - Format the workspace"
	@echo "  make fmt-check      - Check formatting without changing files"
	@echo "  make validate       - Validate the bundled example arena"
	@echo "  make lint           - Run the complete local CI gate"
	@echo "  make clean          - Remove Cargo build artifacts"
	@echo "  make install-hooks  - Install the pre-push lint hook"
	@echo ""
	@echo "Next version will be: $(VERSION)"

release-preflight:
	@test "$$(git branch --show-current)" = "main" || { echo "release must start on main" >&2; exit 1; }
	@test -z "$$(git status --porcelain)" || { echo "release requires a clean worktree" >&2; exit 1; }
	@git rev-parse --verify "refs/tags/v$(VERSION)" >/dev/null 2>&1 && { echo "tag v$(VERSION) already exists" >&2; exit 1; } || true

version-bump: release-preflight
	@echo "Creating release/v$(VERSION)..."
	@git switch -c "release/v$(VERSION)"
	@sed -i 's/^version = .*/version = "$(VERSION)"/' Cargo.toml
	@cargo check --workspace --quiet
	@git add Cargo.toml Cargo.lock
	@git commit -m "chore: bump version to $(VERSION)"

release: version-bump
	@echo "Merging release/v$(VERSION) into main..."
	@git switch main
	@git merge --no-ff "release/v$(VERSION)" -m "Merge branch 'release/v$(VERSION)'"
	@git tag -a "v$(VERSION)" -m "Release v$(VERSION)"
	@git push origin main "v$(VERSION)"
	@echo "Released v$(VERSION); GitHub Actions will attach the platform binaries."

build:
	cargo build --release --workspace

demo:
	@./scripts/run-demo.sh

demo-check:
	bash -n scripts/run-demo.sh

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

lint: fmt-check clippy test validate demo-check

clean:
	cargo clean

install-hooks:
	@mkdir -p .git/hooks
	@printf '#!/usr/bin/env bash\nset -e\nexec make lint\n' > .git/hooks/pre-push
	@chmod +x .git/hooks/pre-push
	@echo "Installed pre-push hook -> make lint"
