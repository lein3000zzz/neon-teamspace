# The entry point for local work: `make` lists every target. Works with GNU make 3.81
# or later and a POSIX shell.

CARGO ?= cargo
BUN   ?= bun
SQLX  ?= sqlx

API_SPEC      := api/openapi.json
SETTINGS_PAGE := docs/src/content/docs/reference/settings.md
# A fixed project name, so db-down stops what db-up started whatever COMPOSE_PROJECT_NAME says.
DEV_COMPOSE   := docker compose -p neon-dev -f deployments/docker-compose.dev.yml
# The neon binary, built from the .sqlx/ query cache, so generating files needs no database.
NEON          := SQLX_OFFLINE=true $(CARGO) run --quiet -p neon-server --

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
	@awk 'BEGIN {FS = ":.*## "; print "Usage: make <target>"} /^## / {printf "\n%s\n", substr($$0, 4)} /^[a-zA-Z_-]+:.*## / {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

## Build and run

.PHONY: build
build: ## Compile the workspace, checking SQL against DATABASE_URL or, unset, the .sqlx/ cache
	$(CARGO) build --workspace

.PHONY: run
run: ## Run the server on :8080 with the variables from .env (after make db-up and make migrate)
	@test -f .env || { echo "no .env: copy .env.example to .env"; exit 1; }
	@set -a; . ./.env; set +a; $(CARGO) run -p neon-server -- serve

## Database

.PHONY: db-up
db-up: ## Start the development Postgres on 127.0.0.1:55432 (Docker)
	$(DEV_COMPOSE) up -d --wait

.PHONY: db-down
db-down: ## Stop the development Postgres; its data stays in a volume
	$(DEV_COMPOSE) down

.PHONY: migrate
migrate: ## Apply pending migrations to the database DATABASE_URL names (.env)
	$(SQLX) migrate run

## Rust

.PHONY: fmt
fmt: ## Format the Rust code
	$(CARGO) fmt --all

.PHONY: fmt-check
fmt-check: ## Fail if any Rust file needs formatting
	$(CARGO) fmt --all --check

.PHONY: lint
lint: ## Run clippy on every target, warnings as errors
	$(CARGO) clippy --workspace --all-targets -- -D warnings

.PHONY: test
test: ## Run the Rust tests (needs the dev database: make db-up and make migrate)
	$(CARGO) test --workspace

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

## Checks

.PHONY: licenses
# cargo-deny's advisories check is left out: it fetches the advisory database.
licenses: ## Check licenses against deny.toml: crates (with cargo-deny's bans and sources), web, docs
	$(CARGO) deny check bans licenses sources
	$(BUN) scripts/licenses.ts web
	$(BUN) scripts/licenses.ts docs

.PHONY: check
# Offline: the build checks SQL against the committed .sqlx/ cache, not a database.
check: export SQLX_OFFLINE := true
check: fmt-check lint build api-check web-check docs-check licenses ## Every check that needs no Docker or network (all but sqlx-check and test)
	@echo "all checks passed"

.PHONY: clean
clean: ## Remove build output: target/, web/dist, docs/dist, docs/.astro and docs/csp.caddy
	$(CARGO) clean
	rm -rf web/dist docs/dist docs/.astro docs/csp.caddy
