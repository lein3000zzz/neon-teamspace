# syntax=docker/dockerfile:1.29.0@sha256:572cebbc2b15e6f84c077742952674889cb63cd5e9b80abdffcbe587b6f19183

# The web image: the app and the docs as static files behind Caddy, which also passes the
# server's paths on to it. Build from the repository's root, as the app and the docs use
# scripts/ and api/ beside them:
#   docker buildx build -f web/Dockerfile --platform linux/amd64,linux/arm64 .
#
# The app and the docs are the same files on every platform, so they build on the builder's
# own platform, and only Caddy is the image's. Nothing is emulated.

# Bun as docs/package.json pins it, on Debian, as Playwright's Chromium needs glibc.
FROM --platform=$BUILDPLATFORM oven/bun:1.4.4-slim@sha256:1846adf22496bd711036aae8be147e57bdebbaea2e5264e3e0b5af1db187d9a4 AS bun

FROM bun AS app
WORKDIR /src/web
COPY web/package.json web/bun.lock web/bunfig.toml ./
RUN --mount=type=cache,target=/root/.bun/install/cache bun install --frozen-lockfile
# The license helpers the build uses to write THIRD_PARTY_NOTICES.txt. The API types are
# committed, so the build needs no spec.
COPY scripts /src/scripts
COPY web ./
RUN bun run build

FROM bun AS docs
WORKDIR /src/docs
ENV ASTRO_TELEMETRY_DISABLED=1
COPY docs/package.json docs/bun.lock docs/bunfig.toml ./
RUN --mount=type=cache,target=/root/.bun/install/cache bun install --frozen-lockfile
# The headless Chromium that draws the diagrams while the docs build, with the system
# libraries it needs. It stays in this stage.
RUN bun run chromium --with-deps
# The API reference is generated from the committed spec, and the license helpers write
# THIRD_PARTY_NOTICES.txt.
COPY api /src/api
COPY scripts /src/scripts
COPY docs ./
RUN bun run build

# Checks the Caddyfile with the docs' policy in place.
FROM --platform=$BUILDPLATFORM caddy:2.11.9-alpine@sha256:003a97808e781aa9f7e75cc2f43d34d461099031faf5e20faefb9f61126836f0 AS caddyfile
COPY web/Caddyfile /etc/caddy/Caddyfile
COPY --from=docs /src/docs/csp.caddy /etc/caddy/docs-csp.caddy
RUN caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile

FROM caddy:2.11.9-alpine@sha256:003a97808e781aa9f7e75cc2f43d34d461099031faf5e20faefb9f61126836f0

COPY --from=caddyfile /etc/caddy /etc/caddy
# The app, with THIRD_PARTY_NOTICES.txt and the fonts' license files beside it.
COPY --from=app /src/web/dist /srv/app
# The docs, with their own THIRD_PARTY_NOTICES.txt.
COPY --from=docs /src/docs/dist /srv/docs

# The base image's labels describe Caddy; these describe the image.
LABEL org.opencontainers.image.title="neon-teamspace-web" \
      org.opencontainers.image.description="Neon Teamspace's web app and docs, served by Caddy" \
      org.opencontainers.image.source="https://github.com/lein3000zzz/neon-teamspace" \
      org.opencontainers.image.licenses="PolyForm-Noncommercial-1.0.0" \
      org.opencontainers.image.version="" \
      org.opencontainers.image.url="" \
      org.opencontainers.image.documentation="" \
      org.opencontainers.image.vendor=""

# Not root. Docker lets any user in a container bind ports 80 and 443, and elsewhere the
# caddy binary carries the capability to bind them. The base image leaves /data/caddy and
# /config/caddy, where Caddy keeps its certificates and state, writable by every user.
USER 10001:10001

# 80 for HTTP, and with NEON_DOMAIN set, 443 for HTTPS over TCP and HTTP/3 over UDP.
EXPOSE 80 443 443/udp

HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
    CMD ["wget", "-q", "-O", "/dev/null", "http://127.0.0.1:8081/healthz"]
