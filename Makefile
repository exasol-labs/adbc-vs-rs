# adbc-vs build, test, and policy surface.
#
# Every check a developer or CI runs goes through a target here — CI jobs
# call `make <target>` and never re-spell a check as inline shell.

SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c

COMPOSE := docker compose

# --- cross-udf-build --------------------------------------------------------
#
# The SLC-compatible shared object MUST be built inside rust:1.94-trixie,
# never on the host, so its glibc fingerprint matches the language-container
# the SLC loads it into. This is a real-file target: an unchanged workspace
# is a no-op, and touching a crate source invalidates the .so.

CARGO_IMAGE := rust:1.94-trixie
CARGO_REGISTRY_VOLUME := adbc-vs-cargo-registry
SO_FILE := target/release/libadbc_vs_adapter.so
HOST_UID := $(shell id -u)
HOST_GID := $(shell id -g)

# cargo-exasol-udf embeds the SDK fingerprint it validates against, so its
# version MUST track exasol-udf-sdk's own version exactly (see
# crates/adbc-vs-adapter/Cargo.toml and crates/adbc-vs-adapter/tests/common's
# SLC_VERSION) or a correct .so reports a false mismatch.
SLC_VERSION := 0.23.0

# Installed cargo subcommand binaries live under the same registry volume
# (its own subdirectory, since only .../registry is mounted, not the whole
# $CARGO_HOME) so the install survives across --rm containers instead of
# repeating on every build.
CARGO_TOOLS_BIN := /usr/local/cargo/registry/tools/bin

ADAPTER_SRCS := $(shell find crates/adbc-vs-adapter/src -type f 2>/dev/null)
SCAN_SRCS := $(shell find crates/adbc-vs-scan/src -type f 2>/dev/null)
WORKSPACE_MANIFESTS := \
	Cargo.toml \
	Cargo.lock \
	crates/adbc-vs-adapter/Cargo.toml \
	crates/adbc-vs-scan/Cargo.toml \
	.cargo/config.toml

CARGO_RUN := docker run --rm \
	-u $(HOST_UID):$(HOST_GID) \
	-e HOME=/tmp \
	-e CARGO_HOME=/usr/local/cargo \
	-e PATH=$(CARGO_TOOLS_BIN):/usr/local/cargo/bin:/usr/local/rustup/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
	-v $(CURDIR):/workspace \
	-v $(CARGO_REGISTRY_VOLUME):/usr/local/cargo/registry \
	-w /workspace \
	$(CARGO_IMAGE)

$(SO_FILE): $(ADAPTER_SRCS) $(SCAN_SRCS) $(WORKSPACE_MANIFESTS)
	docker volume create $(CARGO_REGISTRY_VOLUME) >/dev/null
	docker run --rm -v $(CARGO_REGISTRY_VOLUME):/usr/local/cargo/registry $(CARGO_IMAGE) \
		chown -R $(HOST_UID):$(HOST_GID) /usr/local/cargo/registry
	$(CARGO_RUN) cargo build --release -p adbc-vs-adapter
	$(CARGO_RUN) bash -c ' \
		test -x $(CARGO_TOOLS_BIN)/cargo-exasol-udf || \
		cargo install cargo-exasol-udf --version $(SLC_VERSION) --root $(dir $(CARGO_TOOLS_BIN))'
	$(CARGO_RUN) cargo exasol-udf validate $(SO_FILE)

# Not .PHONY: this is the developer-facing alias for the real $(SO_FILE)
# target above, so `make cross-udf-build` inherits its file-mtime tracking.
cross-udf-build: $(SO_FILE)

# --- host checks -------------------------------------------------------------

.PHONY: fmt-check
fmt-check:
	cargo fmt --all -- --check

.PHONY: lint
lint:
	cargo clippy --workspace --all-targets --all-features -- -D warnings

.PHONY: test
test:
	cargo test --workspace

.PHONY: deny
deny:
	cargo deny check licenses advisories bans sources
	cargo test -p adbc-vs-adapter --features policy-check --test policy_test

# --- container-backed test tiers ---------------------------------------------

.PHONY: test-artifact
test-artifact: cross-udf-build
	cargo test -p adbc-vs-adapter --features built-artifact --test artifact_test

.PHONY: test-it
test-it: stack-up-mysql
	cargo test -p adbc-vs-scan --features mysql-it --test mysql_it_test

.PHONY: test-e2e
test-e2e: cross-udf-build stack-up
	cargo test -p adbc-vs-adapter --features exasol-e2e --test e2e_mysql_test -- --test-threads=1

# --- stack lifecycle -----------------------------------------------------------
#
# stack-up-mysql and stack-up block until their services accept connections;
# they never rely on a fixed sleep.

.PHONY: stack-up-mysql
stack-up-mysql:
	$(COMPOSE) up -d --wait mysql

EXASOL_DSN ?= exasol://sys:exasol@localhost:8563?validateservercertificate=0
EXASOL_WAIT_TIMEOUT ?= 300

.PHONY: stack-up
stack-up: stack-up-mysql
	$(COMPOSE) up -d exasol
	SECONDS=0; \
	until exapump sql --dsn "$(EXASOL_DSN)" "SELECT 1" >/dev/null 2>&1; do \
		if [ "$$SECONDS" -ge "$(EXASOL_WAIT_TIMEOUT)" ]; then \
			echo "exasol did not accept connections within $(EXASOL_WAIT_TIMEOUT)s" >&2; \
			exit 1; \
		fi; \
		sleep 5; \
	done

.PHONY: stack-down
stack-down:
	$(COMPOSE) down
