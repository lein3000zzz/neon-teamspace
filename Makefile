# The entry point for local work: `make` lists every target. Works with GNU make 3.81
# or later and a POSIX shell.

CARGO  ?= cargo
BUN    ?= bun
SQLX   ?= sqlx
CLAUDE ?= claude
# Options for cargo-deny, before its command. check adds --offline.
DENY_FLAGS ?=
# The e2e stack's Compose project, and the tags of the images make e2e builds and runs it from:
# a run from another checkout can name its own, to keep apart from this one's.
E2E_PROJECT       ?= neon-e2e
E2E_SERVER_IMAGE  ?= neon-teamspace-server:local
E2E_WEB_IMAGE     ?= neon-teamspace-web:local
# The Cargo profile of the server image make e2e tests: e2e, the release profile's quicker
# build, unless set to release, as the run before a release does to test the published build.
E2E_CARGO_PROFILE ?= e2e
# Options for docker buildx bake as it builds those images, such as CI's layer cache.
E2E_BUILD_FLAGS   ?=
# The port of 127.0.0.1 the e2e stack's web origin is on: one free when make e2e starts it,
# unless set.
E2E_WEB_PORT      ?=
# The container make test-s3 runs SeaweedFS in: a run from another checkout can name its own.
S3_TEST_CONTAINER ?= neon-s3-test
# The image the seaweedfs profile of deployments/docker-compose.yml runs: keep the two in step.
SEAWEEDFS_IMAGE   := chrislusf/seaweedfs:4.55@sha256:7ca46aeae2f223b37e5d91593c17ab37738eec6df6ab69b48a0487f7643342b9

# The version of every crate, and so of a release: the cargo workspace's, in Cargo.toml.
VERSION       := $(shell sed -n '/^\[workspace\.package\]/,/^\[/s/^version = "\(.*\)"$$/\1/p' Cargo.toml)
# The files of the Compose archive a release attaches: what a stack of the published images
# needs beside them, at the paths the install docs give.
COMPOSE_FILES := LICENSE deployments/docker-compose.yml deployments/.env.example \
	deployments/server.env.example deployments/grafana
API_SPEC      := api/openapi.json
SETTINGS_PAGE := docs/src/content/docs/reference/settings.md
# A fixed project name, so db-down stops what db-up started whatever COMPOSE_PROJECT_NAME says.
DEV_COMPOSE   := docker compose -p neon-dev -f deployments/docker-compose.dev.yml
# The whole stack: Postgres, the server and Caddy, with the settings in deployments/.env and
# deployments/server.env. On its own, the file runs the published images.
STACK_COMPOSE := docker compose -f deployments/docker-compose.yml
# The same stack with both images built from this checkout, whatever NEON_VERSION says.
BUILD_COMPOSE := $(STACK_COMPOSE) -f deployments/docker-compose.build.yml
# Compose reads the whole file, so starting or stopping services that do not use
# POSTGRES_PASSWORD still needs one.
NO_DATABASE   := POSTGRES_PASSWORD=unused
# A throwaway stack for e2e, built from this checkout, beside any other: a project of its own,
# a free port on loopback, Mailpit for its mail, and neither deployments/.env nor
# deployments/server.env, so the tests see the defaults.
E2E_COMPOSE   := docker compose -p $(E2E_PROJECT) --env-file /dev/null -f deployments/docker-compose.yml \
	-f deployments/docker-compose.build.yml -f deployments/docker-compose.e2e.yml
# Builds the e2e stack's images as docker-compose.build.yml describes them, with bake rather
# than Compose, as bake takes cache options; the server in E2E_CARGO_PROFILE. Bake reads a
# Compose file's build context from the directory it runs in, not from the file's, so the root
# is named again.
E2E_BAKE      := docker buildx bake -f deployments/docker-compose.build.yml \
	--set server.context=. --set web.context=. --set server.args.CARGO_PROFILE=$(E2E_CARGO_PROFILE) \
	--set server.tags=$(E2E_SERVER_IMAGE) --set web.tags=$(E2E_WEB_IMAGE)
# The instance name the e2e stack is given, and its tests expect.
E2E_INSTANCE  := Neon E2E
# The first admin of the e2e stack, whom the tests sign in as. make e2e makes the password.
E2E_ADMIN     := admin@e2e.example
# The neon binary, built from the .sqlx/ query cache, so generating files needs no database.
NEON          := SQLX_OFFLINE=true $(CARGO) run --quiet -p neon-server --
# Where check builds, apart from the builds checked against a database.
OFFLINE_TARGET_DIR := $(CURDIR)/target/offline
# The apps' Tauri project: the bundled server picker in APP_DIR, and its Rust side, a Cargo
# workspace of its own, which builds into its own target directory, where Tauri's CLI looks
# for it, whatever CARGO_TARGET_DIR says for the server's.
APP_DIR       := apps/shell
APP_MANIFEST  := $(APP_DIR)/src-tauri/Cargo.toml
APP_CARGO     := CARGO_TARGET_DIR=$(CURDIR)/$(APP_DIR)/src-tauri/target $(CARGO)
# The port make app-test previews the picker on: a run from another checkout can name its own.
PICKER_PORT   ?= 1421

# Astro reports usage over the network unless told not to.
export ASTRO_TELEMETRY_DISABLED := 1

# Fails, showing the difference, when a command's output is not the committed file.
# $(call check-generated,FILE,COMMAND,TARGET THAT REWRITES FILE)
check-generated = set -e; tmp="$$(mktemp)"; trap 'rm -f "$$tmp"' EXIT; $(2) > "$$tmp"; \
	diff -u $(1) "$$tmp" || { echo "$(1) is stale: run make $(3)"; exit 1; }; \
	echo "$(1) is current"

.DEFAULT_GOAL := help

.PHONY: help
help: ## Show this help
	@awk 'BEGIN {FS = ":.*## "; print "Usage: make <target>"} /^## / {printf "\n%s\n", substr($$0, 4)} /^[a-zA-Z0-9_-]+:.*## / {printf "  \033[36m%-15s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

## Build and run

.PHONY: build
build: ## Compile the workspace, checking SQL against DATABASE_URL or, unset, the .sqlx/ cache
	$(CARGO) build --workspace

.PHONY: run
run: ## Run the server on :8080 with the variables from .env (after make db-up and make migrate)
	@test -f .env || { echo "no .env: copy .env.example to .env"; exit 1; }
	@set -a; . ./.env; set +a; $(CARGO) run -p neon-server -- serve

## Docker stack (deployments/)

.PHONY: up
up: ## Build both images from this checkout and run the stack on http://127.0.0.1:3000 (Ctrl+C stops it)
	@test -f deployments/.env || { echo "no deployments/.env: copy deployments/.env.example to deployments/.env and set POSTGRES_PASSWORD"; exit 1; }
	NEON_DOMAIN= NEON_HTTP_PORT=127.0.0.1:3000 $(BUILD_COMPOSE) up --build

.PHONY: down
down: ## Stop what make up and make obs-up started; the data stays in volumes
	$(NO_DATABASE) $(STACK_COMPOSE) --profile mail --profile observability down

.PHONY: obs-up
obs-up: ## Start Grafana, Prometheus, Tempo, Loki and Pyroscope on http://127.0.0.1:3001, for make run
	$(NO_DATABASE) $(STACK_COMPOSE) --profile observability up --detach --wait otel-lgtm
	@echo "Grafana is on http://127.0.0.1:3001 and scrapes the admin listener of make run."
	@echo "For traces and profiles too, add these to .env and restart make run:"
	@echo "  NEON_OBSERVABILITY__TRACING__ENABLED=true"
	@echo "  NEON_OBSERVABILITY__PROFILING__MODE=push"
	@echo "  NEON_OBSERVABILITY__PROFILING__PUSH__SERVER_ADDRESS=http://127.0.0.1:4040"

## Database

.PHONY: db-up
db-up: ## Start the development Postgres on 127.0.0.1:55432 and Mailpit on http://127.0.0.1:58025 (Docker)
	$(DEV_COMPOSE) up -d --wait

.PHONY: db-down
db-down: ## Stop the development Postgres and Mailpit; the database stays in a volume
	$(DEV_COMPOSE) down

.PHONY: migrate
migrate: ## Apply pending migrations to the database DATABASE_URL names (.env)
	$(SQLX) migrate run

## Rust

.PHONY: fmt
fmt: ## Format the Rust code, the apps' too
	$(CARGO) fmt --all
	$(CARGO) fmt --manifest-path $(APP_MANIFEST) --all

.PHONY: fmt-check
fmt-check: ## Fail if any Rust file needs formatting
	$(CARGO) fmt --all --check

.PHONY: lint
lint: ## Run clippy on every target, warnings as errors
	$(CARGO) clippy --workspace --all-targets -- -D warnings

.PHONY: doc-check
doc-check: ## Build the Rust docs, private items included, warnings as errors
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc --workspace --no-deps --document-private-items

.PHONY: test
test: ## Run the Rust tests (needs the dev database: make db-up and make migrate)
	$(CARGO) test --workspace

.PHONY: test-s3
# SeaweedFS on a free port of loopback, with a key and secret for the run alone, run as the
# seaweedfs profile of deployments/docker-compose.yml runs it; the trap removes its container
# however the run ends, and prints its log first when the run failed. The tests build with the
# whole workspace's features, as make test does, so the build make test left is used again
# instead of a second one for neon-server alone.
# The S3 port answers its health check before the filer and a volume server can take a write,
# so the run waits until a signed write of a probe object goes through, and gives up after a
# minute.
test-s3: ## Test the S3 blob store against SeaweedFS in a throwaway container (Docker)
	@set -e; \
	trap 'status=$$?; if [ $$status -ne 0 ]; then echo "SeaweedFS log:"; docker logs --tail 200 $(S3_TEST_CONTAINER) 2>&1; fi; docker container rm --force $(S3_TEST_CONTAINER) >/dev/null 2>&1; exit $$status' EXIT; \
	docker run -d --name $(S3_TEST_CONTAINER) -p 127.0.0.1::8333 \
		-e AWS_ACCESS_KEY_ID=neon-test -e AWS_SECRET_ACCESS_KEY=neon-test-secret \
		$(SEAWEEDFS_IMAGE) server -s3 -ip=127.0.0.1 -ip.bind=127.0.0.1 -s3.ip.bind=0.0.0.0 \
		-s3.port.iceberg=0 -s3.port.lance=0 >/dev/null; \
	address="$$(docker port $(S3_TEST_CONTAINER) 8333/tcp | head -n 1)"; \
	signed="--aws-sigv4 aws:amz:us-east-1:s3 --user neon-test:neon-test-secret"; \
	ready=no; \
	for _ in $$(seq 1 60); do \
		curl -fsS $$signed -X PUT "http://$$address/neon-test" >/dev/null 2>&1 || true; \
		if curl -fsS $$signed -X PUT --data probe "http://$$address/neon-test/ready-probe" >/dev/null 2>&1; then \
			ready=yes; break; \
		fi; \
		sleep 1; \
	done; \
	if [ "$$ready" != yes ]; then echo "SeaweedFS took no write within a minute"; exit 1; fi; \
	curl -fsS $$signed -X DELETE "http://$$address/neon-test/ready-probe" >/dev/null; \
	NEON_TEST_S3_ENDPOINT="http://$$address" NEON_TEST_S3_BUCKET=neon-test \
		NEON_TEST_S3_ACCESS_KEY_ID=neon-test NEON_TEST_S3_SECRET_ACCESS_KEY=neon-test-secret \
		$(CARGO) test --workspace --test blobs

## Generated files

.PHONY: sqlx-prepare
sqlx-prepare: ## Rewrite the .sqlx/ query cache after a query or migration changes (needs the dev database)
	$(CARGO) sqlx prepare --workspace

.PHONY: sqlx-check
sqlx-check: ## Fail if the .sqlx/ query cache is stale (needs the dev database)
	$(CARGO) sqlx prepare --workspace --check

.PHONY: api-spec
api-spec: ## Rewrite api/openapi.json from the server's routes
	$(NEON) openapi --out $(API_SPEC)
	@echo "updated $(API_SPEC); run make web-api to update the web app's types"

.PHONY: api-check
api-check: ## Fail if api/openapi.json is stale
	@$(call check-generated,$(API_SPEC),$(NEON) openapi,api-spec)

.PHONY: web-api
web-api: ## Rewrite the web app's API types from api/openapi.json
	cd web && $(BUN) run gen:api

.PHONY: docs-gen
docs-gen: ## Rewrite the docs' settings reference page from the settings registry
	@set -e; tmp="$$(mktemp)"; trap 'rm -f "$$tmp"' EXIT; \
	$(NEON) settings docs > "$$tmp"; cat "$$tmp" > $(SETTINGS_PAGE); \
	echo "updated $(SETTINGS_PAGE)"

## Web app (web/)

.PHONY: web-install
web-install: ## Install the web app's packages
	cd web && $(BUN) install --frozen-lockfile

.PHONY: web-dev
web-dev: ## Run the web app on :5173, passing /api and the server's other paths to make run
	cd web && $(BUN) run dev

.PHONY: web-check
web-check: ## Type-check, lint, format-check and test the web app and scripts/; check its API types
	cd web && $(BUN) run check

.PHONY: web-build
web-build: ## Build the web app into web/dist, with its third-party notices
	cd web && $(BUN) run build

## Docs (docs/)

.PHONY: docs-install
docs-install: ## Install the docs' packages and the headless Chromium that draws their diagrams
	cd docs && $(BUN) install --frozen-lockfile
	cd docs && $(BUN) run chromium

.PHONY: docs-dev
docs-dev: ## Run the docs on :4321/docs/, reloading as pages change
	cd docs && $(BUN) run dev

.PHONY: docs-build
docs-build: ## Build the docs into docs/dist, with their Content-Security-Policy
	cd docs && $(BUN) run build

.PHONY: docs-check
docs-check: ## Type-check the docs and test their scripts; fail if the settings page is stale
	cd docs && $(BUN) run check
	@$(call check-generated,$(SETTINGS_PAGE),$(NEON) settings docs,docs-gen)

## End-to-end tests (e2e/)

.PHONY: e2e-install
e2e-install: ## Install the end-to-end tests' packages and Playwright's headless Chromium, with its system libraries on CI
	cd e2e && $(BUN) install --frozen-lockfile
	cd e2e && $(BUN) run chromium $(if $(CI),--with-deps)

.PHONY: e2e-check
e2e-check: ## Type-check the end-to-end tests and check their formatting
	cd e2e && $(BUN) run check

.PHONY: e2e-images
e2e-images: ## Build both images for make e2e, the server in Cargo's quicker e2e profile unless E2E_CARGO_PROFILE says otherwise
	$(E2E_BAKE) --load $(E2E_BUILD_FLAGS) server web

.PHONY: e2e
# Under make -j the images build while e2e-install runs. The server's public URL names the web
# origin's port, so the port is picked before the stack starts, from those free then. The trap
# removes the stack, its volumes too, however the run ends, after printing the containers'
# logs when it failed, and keeps the run's exit status.
e2e: e2e-install e2e-images ## Build both images, test a throwaway stack of them in Chromium, then remove it
	@set -e; \
	port="$(E2E_WEB_PORT)"; if [ -z "$$port" ]; then port="$$($(BUN) e2e/free-port.ts)"; fi; \
	export POSTGRES_PASSWORD="$$(openssl rand -hex 16)" E2E_INSTANCE_NAME="$(E2E_INSTANCE)" \
		E2E_ADMIN_EMAIL="$(E2E_ADMIN)" E2E_ADMIN_PASSWORD="$$(openssl rand -hex 16)" \
		E2E_SECRET_KEYS="e2e:$$(openssl rand -base64 32)" E2E_WEB_PORT="$$port" \
		E2E_SERVER_IMAGE="$(E2E_SERVER_IMAGE)" E2E_WEB_IMAGE="$(E2E_WEB_IMAGE)"; \
	trap 'status=$$?; if [ $$status -ne 0 ]; then $(E2E_COMPOSE) logs --no-color --tail=100; fi; \
		$(E2E_COMPOSE) down --volumes --remove-orphans >/dev/null 2>&1; exit $$status' EXIT; \
	$(E2E_COMPOSE) up --detach --no-build --wait; \
	receiver="$$($(E2E_COMPOSE) port receiver 8080)"; \
	mail="$$($(E2E_COMPOSE) port mailpit 8025)"; \
	echo "testing http://127.0.0.1:$$port; the report goes to e2e/playwright-report/"; \
	(cd e2e && E2E_BASE_URL="http://127.0.0.1:$$port" E2E_RECEIVER_URL="http://$$receiver" \
		E2E_MAIL_URL="http://$$mail" $(BUN) run test)

## Block fixtures (tools/blocknote-fixtures/)

.PHONY: blocks-install
blocks-install: ## Install the packages of the tool that writes and checks the block fixtures with BlockNote
	cd tools/blocknote-fixtures && $(BUN) install --frozen-lockfile

.PHONY: blocks-fixtures
blocks-fixtures: ## Rewrite the block fixtures: BlockNote's for the corpus, then neon-blocks' for BlockNote's
	cd tools/blocknote-fixtures && $(BUN) run generate
	BLOCKS_WRITE_FIXTURES=1 $(CARGO) test -p neon-blocks --test blocknote

.PHONY: blocks-check
blocks-check: ## Test neon-blocks against BlockNote's fixtures, and check BlockNote loads neon-blocks' updates
	$(CARGO) test -p neon-blocks
	cd tools/blocknote-fixtures && $(BUN) run check

## Apps (apps/shell/)

.PHONY: app-install
app-install: ## Install the apps' packages: the server picker's, its tests' and Tauri's CLI
	cd $(APP_DIR) && $(BUN) install --frozen-lockfile

.PHONY: app-browsers
app-browsers: ## Install the headless Chromium and WebKit make app-test runs, with their system libraries on CI
	cd $(APP_DIR) && $(BUN) x playwright install $(if $(CI),--with-deps) --only-shell chromium
	cd $(APP_DIR) && $(BUN) x playwright install $(if $(CI),--with-deps) webkit

.PHONY: app-picker
app-picker: ## Build the bundled server picker into apps/shell/dist, which the app's Rust build embeds
	cd $(APP_DIR) && $(BUN) run build

.PHONY: app-dev
app-dev: ## Run the desktop app, the picker reloading as it changes
	cd $(APP_DIR) && $(BUN) run tauri dev

.PHONY: app-check
app-check: app-picker ## Type-check, lint and format-check the picker; format-check, lint, doc-check and test the app's Rust side
	cd $(APP_DIR) && $(BUN) run check
	$(CARGO) fmt --manifest-path $(APP_MANIFEST) --all --check
	$(APP_CARGO) clippy --manifest-path $(APP_MANIFEST) --all-targets -- -D warnings
	RUSTDOCFLAGS="-D warnings" $(APP_CARGO) doc --manifest-path $(APP_MANIFEST) --no-deps --document-private-items
	$(APP_CARGO) test --manifest-path $(APP_MANIFEST)

.PHONY: app-test
app-test: app-picker ## Test the picker on its own, under the app's policy, in Chromium and WebKit at desktop and phone sizes
	cd $(APP_DIR) && PICKER_PORT=$(PICKER_PORT) $(BUN) run test

.PHONY: app-build
# CI=true makes the disk image's script skip the AppleScript that lays out its Finder window,
# which needs permission to control Finder; the image is the same otherwise.
app-build: ## Build the desktop app and its installers for this machine, into apps/shell/src-tauri/target/release/bundle
	cd $(APP_DIR) && CI=true $(BUN) run tauri build

.PHONY: app-webview-check
# The binary make app-build leaves, as CI's Linux build does too. Under xvfb-run -a where there
# is no display.
app-webview-check: ## On Linux, run the built app against a hostile server in WebKitGTK: no page of a server may call the picker's commands
	cd $(APP_DIR) && $(BUN) scripts/webview-check.ts src-tauri/target/release/neon-teamspace

.PHONY: app-android
app-android: ## Build an Android APK signed with the debug key, for testing (needs JAVA_HOME, ANDROID_HOME and NDK_HOME)
	cd $(APP_DIR) && CI=true $(BUN) run tauri android build --apk --debug

.PHONY: app-ios
app-ios: ## Build the iOS app with Xcode, signed for APPLE_DEVELOPMENT_TEAM and exported for App Store Connect
	cd $(APP_DIR) && CI=true $(BUN) run tauri ios build --export-method app-store-connect

## Claude Code plugin (.claude-plugin/ and integrations/claude-code/)

.PHONY: plugin-check
# The test checks the files' fields and paths, and that the skills name only the MCP server's
# tools and resources, read from its code; make web-check runs it too, with the rest of
# scripts/. Claude Code's own validation runs as well where Claude Code is installed.
plugin-check: ## Check the Claude Code plugin and its marketplace, and that its skills name only the MCP server's tools
	$(BUN) test scripts/claude-plugin
	@if command -v $(CLAUDE) >/dev/null 2>&1; then \
		$(CLAUDE) plugin validate --strict . && $(CLAUDE) plugin validate --strict integrations/claude-code; \
	else echo "$(CLAUDE) is not installed: skipped claude plugin validate"; fi

## Releases

.PHONY: version-check
# The release workflow passes the tag it runs for as TAG, which the recipe reads from the
# environment rather than pasted into the command.
version-check: ## Fail unless every stated version is Cargo.toml's: neon --version, the plugin, the packages, Compose, the docs, and TAG if set
	@set -e; neon="$$($(NEON) --version)"; \
	$(BUN) scripts/version.ts --neon "$$neon" $(if $(TAG),--tag "$$TAG")

.PHONY: compose-archive
# From the commit checked out, not the working tree, so the archive holds what the tag holds.
compose-archive: ## Write dist/neon-teamspace-compose-VERSION.tar.gz, the Compose file and its settings a release attaches
	@mkdir -p dist
	git archive --format=tar.gz --prefix=neon-teamspace-$(VERSION)/ \
		-o dist/neon-teamspace-compose-$(VERSION).tar.gz HEAD -- $(COMPOSE_FILES)
	@echo "wrote dist/neon-teamspace-compose-$(VERSION).tar.gz"

## Checks

.PHONY: licenses
# Without --offline, cargo-deny first updates its copy of the RustSec advisory database.
licenses: ## Run cargo-deny's license, advisory, ban and source checks; check the JS packages' licenses
	$(CARGO) deny $(DENY_FLAGS) check
	$(CARGO) deny $(DENY_FLAGS) --manifest-path $(APP_MANIFEST) check
	$(BUN) scripts/licenses.ts web
	$(BUN) scripts/licenses.ts docs
	$(BUN) scripts/licenses.ts e2e
	$(BUN) scripts/licenses.ts tools/blocknote-fixtures
	$(BUN) scripts/licenses.ts $(APP_DIR)

.PHONY: check
# Offline. The build checks SQL against the committed .sqlx/ cache rather than a database, in
# a target directory of its own: neon-store's build script rebuilds it whenever SQLX_OFFLINE or
# DATABASE_URL changes, so that each build reads the right source, and in target/ each switch
# between this build and one checked against a database would rebuild neon-store and every
# crate above it. cargo-deny reads the advisory database it last fetched, and fails when that
# copy is more than 90 days old.
check: export SQLX_OFFLINE := true
check: export CARGO_TARGET_DIR := $(OFFLINE_TARGET_DIR)
check: DENY_FLAGS := --offline
check: fmt-check lint doc-check build api-check web-check docs-check e2e-check blocks-check plugin-check app-check version-check licenses ## Every check that needs no Docker or network (all but sqlx-check, test, e2e and app-test)
	@echo "all checks passed"

.PHONY: clean
clean: ## Remove build output: target/, dist/, web/dist, docs/dist, docs/.astro, docs/csp.caddy and the apps'
	$(CARGO) clean
	rm -rf "$(OFFLINE_TARGET_DIR)" dist web/dist docs/dist docs/.astro docs/csp.caddy
	rm -rf $(APP_DIR)/dist $(APP_DIR)/src-tauri/target
