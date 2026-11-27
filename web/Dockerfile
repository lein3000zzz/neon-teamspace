# syntax=docker/dockerfile:1.29.0@sha256:572cebbc2b15e6f84c077742952674889cb63cd5e9b80abdffcbe587b6f19183

# The web image: the app and the docs as static files behind Caddy, which also passes the
# server's paths on to it. Build from the repository's root, as the app and the docs use
# scripts/ and api/ beside them:
#   docker buildx build -f web/Dockerfile --platform linux/amd64,linux/arm64 .
#
# The app and the docs are the same files on every platform, so they build on the builder's
# own platform, and only Caddy is the image's. Nothing is emulated.

# Bun as docs/package.json pins it, on Debian, as Playwright's Chromium needs glibc.
FROM --platform=$BUILDPLATFORM oven/bun:1.4.5-slim@sha256:b780593f3e10918cbdaec3d5a08d52224b9198069a65eb47812085c2ec007643 AS bun

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
# The Go modules the caddy binary links, each with its version and the hash of its files, as
# the binary records them; every platform's binary links the same ones.
RUN caddy build-info > /caddy-build-info.txt

# The license notices of Caddy and of the Go modules its binary links: Apache-2.0 asks for a
# copy of the license, and MIT and BSD for their notice in every copy, and the official image
# carries none. Each module comes from the Go module proxy, which go checks against the
# checksum database, and then against the hash the binary records. Every license, copying,
# notice and patents file at a module's root goes in, and a module with none fails the build.
FROM --platform=$BUILDPLATFORM golang:1.27.4-alpine3.24@sha256:70bd7ddea8b6cbe5d6943cde279d6b67feb10cc924fb9100d1d8757c1575b280 AS caddy-notices
ENV GOTOOLCHAIN=local GOPROXY=https://proxy.golang.org GOSUMDB=sum.golang.org GOFLAGS=-modcacherw
COPY --from=caddyfile /caddy-build-info.txt /
RUN <<'EOF'
set -eu
info=/caddy-build-info.txt
if grep -q '^=>' "$info"; then
    echo "caddy build-info lists a replaced module, which this stage does not handle" >&2
    exit 1
fi
awk -F'\t' '$1 == "dep" { print $2, $3, $4 }' "$info" > /modules
caddy="$(awk '$1 == "github.com/caddyserver/caddy/v2" { print $2 }' /modules)"
[ -n "$caddy" ] || { echo "caddy build-info lists no github.com/caddyserver/caddy/v2" >&2; exit 1; }
toolchain="$(awk -F'\t' '$1 == "go" { print $2 }' "$info")"
# One JSON object per module, each field on a line of its own.
go mod download -json $(awk '{ print $1 "@" $2 }' /modules) > /downloads.json
awk -F'"' '/^\t"Path"/ { path = $4 } /^\t"Version"/ { version = $4 } /^\t"Sum"/ { sum = $4 }
    /^\t"Dir"/ { dir = $4 } /^}/ { print path, version, sum, dir }' /downloads.json > /downloads
notices=/THIRD_PARTY_NOTICES.txt
cat > "$notices" <<NOTICE
Third-party notices

The web image's Caddy, /usr/bin/caddy, is Caddy $caddy built with $toolchain, and links the
Go modules below. Each section is one module, at the version the binary was built with, and
the license files it ships. The last is Go's own, for the standard library and runtime the
binary is compiled with.
NOTICE
rule="$(printf '=%.0s' $(seq 80))"
thin="$(printf -- '-%.0s' $(seq 80))"
missing=""
while read -r path version sum; do
    found="$(awk -v p="$path" -v v="$version" '$1 == p && $2 == v' /downloads)"
    [ -n "$found" ] || { echo "go mod download gave nothing for $path $version" >&2; exit 1; }
    set -- $found
    if [ "$3" != "$sum" ]; then
        echo "$path $version: the download's hash $3 is not the binary's, $sum" >&2
        exit 1
    fi
    files="$(find "$4" -maxdepth 1 -type f \( -iname 'licen[cs]e*' -o -iname 'copying*' \
        -o -iname 'notice*' -o -iname 'patents*' \) | sort)"
    if [ -z "$files" ]; then
        missing="$missing $path@$version"
        continue
    fi
    printf '\n%s\n%s %s\n%s\n' "$rule" "$path" "$version" "$thin" >> "$notices"
    for file in $files; do
        printf '\n%s:\n\n' "${file##*/}" >> "$notices"
        cat "$file" >> "$notices"
    done
done < /modules
if [ -n "$missing" ]; then
    echo "no license file in:$missing" >&2
    exit 1
fi
printf '\n%s\nGo, its standard library and runtime\n%s\n' "$rule" "$thin" >> "$notices"
for file in LICENSE PATENTS; do
    printf '\n%s:\n\n' "$file" >> "$notices"
    cat "/usr/local/go/$file" >> "$notices"
done
EOF

FROM caddy:2.11.9-alpine@sha256:003a97808e781aa9f7e75cc2f43d34d461099031faf5e20faefb9f61126836f0

COPY --from=caddyfile /etc/caddy /etc/caddy
# The app, with THIRD_PARTY_NOTICES.txt and the fonts' license files beside it.
COPY --from=app /src/web/dist /srv/app
# The docs, with their own THIRD_PARTY_NOTICES.txt.
COPY --from=docs /src/docs/dist /srv/docs
# Caddy's notices, with those of the Go modules it links.
COPY --from=caddy-notices /THIRD_PARTY_NOTICES.txt /usr/share/doc/caddy/THIRD_PARTY_NOTICES.txt

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
