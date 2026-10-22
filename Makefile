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
SLC_VERSION := 0.23.1

# Installed cargo subcommand binaries live under the same registry volume
# (its own subdirectory, since only .../registry is mounted, not the whole
# $CARGO_HOME) so the install survives across --rm containers instead of
# repeating on every build.
CARGO_TOOLS_BIN := /usr/local/cargo/registry/tools/bin

ADAPTER_SRCS := $(shell find crates/adbc-vs-adapter/src -type f 2>/dev/null)
SCAN_SRCS := $(shell find crates/adbc-vs-scan/src -type f 2>/dev/null)

# adbc-vs-scan's build script and the driver declarations it generates the
# compiled-in tables from. They sit outside src/ and are not manifests, so
# without this a version-range or capability-list edit would leave the .so
# untouched and every tier that consumes it certifying the previous binary.
SCAN_BUILD_INPUTS := crates/adbc-vs-scan/build.rs $(shell find crates/adbc-vs-scan/drivers -type f 2>/dev/null)
WORKSPACE_MANIFESTS := \
	Cargo.toml \
	Cargo.lock \
	crates/adbc-vs-adapter/Cargo.toml \
	crates/adbc-vs-scan/Cargo.toml

CARGO_RUN := docker run --rm \
	-u $(HOST_UID):$(HOST_GID) \
	-e HOME=/tmp \
	-e CARGO_HOME=/usr/local/cargo \
	-e PATH=$(CARGO_TOOLS_BIN):/usr/local/cargo/bin:/usr/local/rustup/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
	-v $(CURDIR):/workspace \
	-v $(CARGO_REGISTRY_VOLUME):/usr/local/cargo/registry \
	-w /workspace \
	$(CARGO_IMAGE)

$(SO_FILE): $(ADAPTER_SRCS) $(SCAN_SRCS) $(SCAN_BUILD_INPUTS) $(WORKSPACE_MANIFESTS)
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

.PHONY: test-it-postgres
test-it-postgres: stack-up-postgres
	cargo test -p adbc-vs-scan --features postgres-it --test postgres_it_test

.PHONY: test-it-trino
test-it-trino: stack-up-trino
	cargo test -p adbc-vs-scan --features trino-it --test trino_it_test

.PHONY: test-e2e
test-e2e: cross-udf-build stack-up-exasol stack-up-mysql
	cargo test -p adbc-vs-adapter --features exasol-e2e --test e2e_mysql_test -- --test-threads=1

.PHONY: test-e2e-postgres
test-e2e-postgres: cross-udf-build stack-up-exasol stack-up-postgres stack-up-mysql
	cargo test -p adbc-vs-adapter --features exasol-e2e --test e2e_postgres_test -- --test-threads=1

.PHONY: test-e2e-trino
test-e2e-trino: cross-udf-build stack-up-exasol stack-up-trino stack-up-mysql
	cargo test -p adbc-vs-adapter --features exasol-e2e --test e2e_trino_test -- --test-threads=1

# Reaches the public driver registry, and is the only target that does. Run
# by hand when a declared version range is changed or suspected stale; no CI
# job invokes it, because a CDN outage is not a defect in the commit.
.PHONY: test-registry-live
test-registry-live:
	cargo test -p adbc-vs-adapter --features registry-live --test provisioning_test -- registry_live

# One target per source, each depending on Exasol plus only its own source.
# `bench` runs them one after another through sub-makes rather than as its
# own prerequisites, so even `make -j` never times two sources at once on
# the same cores, then combines the newest per-source CSVs into one table.
BENCH_DIR ?= dist/bench

.PHONY: bench
bench:
	$(MAKE) bench-mysql
	$(MAKE) bench-postgres
	$(MAKE) bench-trino
	$(MAKE) bench-summary

.PHONY: bench-mysql
bench-mysql: cross-udf-build stack-up-exasol stack-up-mysql
	cargo test -p adbc-vs-adapter --features bench --test bench_mysql_test -- --test-threads=1 --nocapture

.PHONY: bench-postgres
bench-postgres: cross-udf-build stack-up-exasol stack-up-postgres
	cargo test -p adbc-vs-adapter --features bench --test bench_postgres_test -- --test-threads=1 --nocapture

.PHONY: bench-trino
bench-trino: cross-udf-build stack-up-exasol stack-up-trino
	cargo test -p adbc-vs-adapter --features bench --test bench_trino_test -- --test-threads=1 --nocapture

.PHONY: bench-summary
bench-summary:
	python3 scripts/bench_summary.py $(BENCH_DIR)

# --- release packaging --------------------------------------------------------
#
# Both targets back the `release` CI job. release-tarball is a real-file
# target keyed on the .so, like cross-udf-build itself, so re-running it
# without a new .so is a no-op. third-party-licenses is always re-run: it
# reflects whatever Cargo.lock currently resolves to. release-tarball packs
# its output, so run third-party-licenses first.

DIST_DIR := dist
RELEASE_TARBALL := $(DIST_DIR)/adbc-vs.tar.gz

$(RELEASE_TARBALL): $(SO_FILE)
	mkdir -p $(DIST_DIR)/pkg/udf
	cp $(SO_FILE) $(DIST_DIR)/pkg/udf/
	chmod 755 $(DIST_DIR)/pkg/udf/libadbc_vs_adapter.so
	cp $(DIST_DIR)/THIRD-PARTY-LICENSES.md $(DIST_DIR)/pkg/
	tar -czf $(RELEASE_TARBALL) -C $(DIST_DIR)/pkg udf THIRD-PARTY-LICENSES.md

.PHONY: release-tarball
release-tarball: $(RELEASE_TARBALL)

.PHONY: third-party-licenses
third-party-licenses:
	mkdir -p $(DIST_DIR)
	cargo about generate --manifest-path crates/adbc-vs-adapter/Cargo.toml \
		-c about.toml about.hbs -o $(DIST_DIR)/THIRD-PARTY-LICENSES.md

# --- stack lifecycle -----------------------------------------------------------
#
# One stack-up target per source, plus one for Exasol: a fourth (or fifth)
# container makes every E2E job pay for sources it never queries. Every
# target below blocks until its own service accepts connections; none ever
# relies on a fixed sleep. `stack-up` remains the one "start everything"
# convenience entry point, depending on every other stack-up-* target.

.PHONY: stack-up-mysql
stack-up-mysql:
	$(COMPOSE) up -d --wait mysql

.PHONY: stack-up-postgres
stack-up-postgres:
	$(COMPOSE) up -d --wait postgres

EXASOL_DSN ?= exasol://sys:exasol@localhost:8563?validateservercertificate=0
EXASOL_WAIT_TIMEOUT ?= 300
# Seconds between attempts in every bounded readiness loop, named for the same
# reason each deadline is: a bare literal beside a named deadline reads as if
# only one of the two were tunable.
POLL_INTERVAL ?= 5

.PHONY: stack-up-exasol
stack-up-exasol:
	$(COMPOSE) up -d exasol
	SECONDS=0; \
	until exapump sql --dsn "$(EXASOL_DSN)" "SELECT 1" >/dev/null 2>&1; do \
		if [ "$$SECONDS" -ge "$(EXASOL_WAIT_TIMEOUT)" ]; then \
			echo "exasol did not accept connections within $(EXASOL_WAIT_TIMEOUT)s" >&2; \
			exit 1; \
		fi; \
		sleep $(POLL_INTERVAL); \
	done

# TRINO_PASSWORD is the plaintext behind docker/trino/password.db's bcrypt
# hash for user adbc_vs. `export` makes it visible to `docker compose exec`'s
# own `-e TRINO_PASSWORD`, which copies the value from this recipe's shell
# rather than from the trino service's container environment.
TRINO_PASSWORD ?= adbc_vs_password
export TRINO_PASSWORD
# TRINO_HOSTILE_PASSWORD is the plaintext behind the same file's hash for user
# adbc_vs_hostile. It carries `@ / : ? # & %` and a space in one value, so the
# tiers that authenticate as that principal prove a credential reaching no URI
# needs no encoding -- every one of those characters would have delimited,
# ended, or escaped something had it reached one. Declared here for the same
# reason TRINO_PASSWORD is: the plaintext behind a committed hash has one
# stated home rather than living only inside a test file. docker-compose.yml
# interpolates this variable rather than repeating the literal, so overriding
# it here reaches the container. The three Rust constants that spell the same
# plaintext -- crates/adbc-vs-scan/tests/trino_it_test.rs,
# crates/adbc-vs-scan/src/connection_tests.rs, and
# crates/adbc-vs-adapter/tests/common/trino.rs -- must be kept in step with it
# by hand, because the bcrypt hash in docker/trino/password.db is fixed and
# authenticates only this value.
TRINO_HOSTILE_PASSWORD ?= p@ss/w:o?r#d&x% y
export TRINO_HOSTILE_PASSWORD
TRINO_WAIT_TIMEOUT ?= 300
TRINO_TLS_DIR := docker/trino/generated
TRINO_KEYSTORE := $(TRINO_TLS_DIR)/keystore.pem

# Not the image's own HEALTHCHECK: that probes the server's info endpoint,
# and what it answers once PASSWORD authentication is on is not established
# here. This poll is both the readiness signal and the fixture seed, since
# the `memory` catalog holds no data across a restart -- the same
# `--file /etc/trino/init.sql` invocation is what "ready" means. The
# negative probe after it proves the container never answers a credential-
# free query: one that did would be silently unauthenticated.
# A real-file target rather than a step inside stack-up-trino: docker-compose.yml
# bind-mounts this path into the trino service, and Docker creates a *directory*
# at a bind-mount source that does not exist. Any `docker compose` invocation
# touching the trino service before the file existed would therefore make the
# `cat` below fail with "Is a directory", recoverable only by hand. As its own
# target the file is a declared prerequisite, so the build graph enforces the
# ordering that was previously left to convention.
$(TRINO_KEYSTORE):
	mkdir -p $(TRINO_TLS_DIR)
	openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj "/CN=localhost" \
		-keyout $(TRINO_TLS_DIR)/key.pem -out $(TRINO_TLS_DIR)/cert.pem
	cat $(TRINO_TLS_DIR)/key.pem $(TRINO_TLS_DIR)/cert.pem > $(TRINO_KEYSTORE)
	rm -f $(TRINO_TLS_DIR)/key.pem $(TRINO_TLS_DIR)/cert.pem

.PHONY: stack-up-trino
stack-up-trino: $(TRINO_KEYSTORE)
	$(COMPOSE) up -d trino
	SECONDS=0; \
	until $(COMPOSE) exec -T -e TRINO_PASSWORD trino \
		trino --server https://localhost:8443 --user adbc_vs --password --insecure \
			--file /etc/trino/init.sql >/dev/null 2>&1; do \
		if [ "$$SECONDS" -ge "$(TRINO_WAIT_TIMEOUT)" ]; then \
			echo "trino did not accept an authenticated connection within $(TRINO_WAIT_TIMEOUT)s" >&2; \
			exit 1; \
		fi; \
		sleep $(POLL_INTERVAL); \
	done
	if probe=$$($(COMPOSE) exec -T trino \
		trino --server https://localhost:8443 --insecure --execute "SELECT 1" 2>&1); then \
		echo "trino answered a credential-free SELECT 1 over https://localhost:8443" >&2; \
		exit 1; \
	elif ! printf '%s' "$$probe" | grep -qiE "401|[Aa]uthentication"; then \
		echo "the credential-free probe failed for a reason other than an authentication refusal, so it proves nothing about whether the container requires one:" >&2; \
		printf '%s\n' "$$probe" >&2; \
		exit 1; \
	fi

.PHONY: stack-up
stack-up: stack-up-mysql stack-up-postgres stack-up-trino stack-up-exasol

.PHONY: stack-down
stack-down:
	$(COMPOSE) down
