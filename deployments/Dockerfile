# syntax=docker/dockerfile:1.29.0@sha256:572cebbc2b15e6f84c077742952674889cb63cd5e9b80abdffcbe587b6f19183

# The server image: the neon binary on a minimal base. Build from the repository's root:
#   docker buildx build -f deployments/Dockerfile --platform linux/amd64,linux/arm64 .
#
# The target updater is the updater image instead: neon-updater beside the docker CLI and
# cosign.
#   docker buildx build -f deployments/Dockerfile --target updater --platform linux/amd64,linux/arm64 .
#
# The Rust build runs on the builder's own platform and cross-compiles for the image's, so an
# arm64 image builds on an amd64 machine, and the reverse, without emulating the compiler.
#
# The build argument CARGO_PROFILE names the Cargo profile of each binary: release, the default,
# for every published image, or e2e, Cargo.toml's quicker build of the same code, which make e2e
# passes unless E2E_CARGO_PROFILE names another.

# cargo-chef, cargo-about and cargo-auditable for the builder's platform, amd64 or arm64, as
# their projects release them, checked against the checksums those releases publish. Compiled
# from source they took minutes of a CI runner's build. Bumped by hand: a new version needs its
# new checksums.
FROM --platform=$BUILDPLATFORM scratch AS tools-amd64
ADD --unpack=true --checksum=sha256:70ef940ef90d04d122f0176fdb8d6c39069191b484a1eaa29b327370c2e1c3c0 \
    https://github.com/LukeMathWalker/cargo-chef/releases/download/v0.1.78/cargo-chef-x86_64-unknown-linux-gnu.tar.xz /
ADD --unpack=true --checksum=sha256:3ad1e94ea1e63d69d1961f4fb15ccbbe9575d906111f96d2cb4dce6df48f71b4 \
    https://github.com/EmbarkStudios/cargo-about/releases/download/0.9.3/cargo-about-0.9.3-x86_64-unknown-linux-musl.tar.gz /
ADD --unpack=true --checksum=sha256:6de0005fc0d414551da45d679bca258565d521ff8ee51c9da10ceed6b957a033 \
    https://github.com/rust-secure-code/cargo-auditable/releases/download/v0.7.8/cargo-auditable-x86_64-unknown-linux-musl.tar.xz /

FROM --platform=$BUILDPLATFORM scratch AS tools-arm64
ADD --unpack=true --checksum=sha256:a47e13fba89c2895f5a5c3d0844acd2a5fd416eceb3a6f9dfb26e28155099f4e \
    https://github.com/LukeMathWalker/cargo-chef/releases/download/v0.1.78/cargo-chef-aarch64-unknown-linux-gnu.tar.xz /
ADD --unpack=true --checksum=sha256:eb4d9ff6ebd2518152a8308729431a04b3836e71d2bd5ead7aea69f91a71162e \
    https://github.com/EmbarkStudios/cargo-about/releases/download/0.9.3/cargo-about-0.9.3-aarch64-unknown-linux-musl.tar.gz /
ADD --unpack=true --checksum=sha256:3a84e9242f1309e519e7eeb941e08c7857ca62ce663913c222a73481120461a5 \
    https://github.com/rust-secure-code/cargo-auditable/releases/download/v0.7.8/cargo-auditable-aarch64-unknown-linux-musl.tar.xz /

FROM tools-$BUILDARCH AS tools

# The Rust version rust-toolchain.toml pins, on Debian 13 like the runtime image below, so the
# binary links against the glibc it runs with. The `chef` stage fails when the two differ.
FROM --platform=$BUILDPLATFORM rust:1.100.0-slim-trixie@sha256:3b199fd6db7d709274df56565b8f4003f9e30d7bfcff9d53148c86053b7855e2 AS chef

# make, for jemalloc, which every Linux build compiles from its C sources. The image has gcc.
RUN apt-get update \
    && apt-get install -y --no-install-recommends make \
    && rm -rf /var/lib/apt/lists/*
# cargo-chef builds the dependencies alone, from a recipe of the manifests, into a layer that
# stays cached until Cargo.lock or a manifest changes.
COPY --from=tools /cargo-chef-*/cargo-chef /usr/local/cargo/bin/

WORKDIR /src
COPY rust-toolchain.toml ./
# The toolchain is the image's. Naming it skips the components the file lists for development,
# rustfmt and clippy, which rustup would download otherwise.
ENV RUSTUP_TOOLCHAIN=$RUST_VERSION
RUN grep -qx "channel = \"$RUST_VERSION\"" rust-toolchain.toml \
    || { echo "rust-toolchain.toml does not pin Rust $RUST_VERSION; update the rust image in deployments/Dockerfile with it" >&2; exit 1; }

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo chef prepare --recipe-path recipe.json

# cargo-about in the recipe's skeleton of the workspace, which has empty sources: the license
# notices of the crates a binary links depend on the manifests and Cargo.lock alone, so they
# stay cached while only the code changes. They come from each crate's own license files, as
# about.toml at the repository's root sets out, the same for every platform, so they are made
# once.
FROM chef AS about
COPY --from=tools /cargo-about-*/cargo-about /usr/local/cargo/bin/
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --no-build --recipe-path recipe.json
COPY about.toml about.hbs ./

# The notices of the crates the neon binary links.
FROM about AS notices
RUN cargo about generate --locked --fail -m crates/server/Cargo.toml -c about.toml \
        -o /THIRD_PARTY_NOTICES.txt about.hbs
# jemalloc's own notice, which about.toml adds to its crate's: a newer crate turns that off.
RUN grep -q "Jason Evans" /THIRD_PARTY_NOTICES.txt \
    || { echo "jemalloc's notice is missing: update the tikv-jemalloc-sys checksums in about.toml" >&2; exit 1; }

FROM chef AS build
ARG TARGETARCH

# The Rust target and the C toolchain for the image's platform. jemalloc and ring compile C,
# and the binary links through gcc. Debian names the compilers by their triple, the native
# one too, so the same variables serve a native and a cross build; the cross compiler and
# libc are installed only when the platforms differ. /cross.env keeps the choice for the
# commands below.
RUN <<'EOF'
set -eu
case "$TARGETARCH" in
    amd64) rust=x86_64-unknown-linux-gnu gnu=x86_64-linux-gnu package=x86-64-linux-gnu ;;
    arm64) rust=aarch64-unknown-linux-gnu gnu=aarch64-linux-gnu package=aarch64-linux-gnu ;;
    *) echo "unsupported platform: linux/$TARGETARCH" >&2; exit 1 ;;
esac
if [ "$TARGETARCH" != "$(dpkg --print-architecture)" ]; then
    apt-get update
    apt-get install -y --no-install-recommends "gcc-$package" "libc6-dev-$TARGETARCH-cross"
    rm -rf /var/lib/apt/lists/*
fi
rustup target add "$rust"
upper=$(echo "$rust" | tr 'a-z-' 'A-Z_')
lower=$(echo "$rust" | tr '-' '_')
cat > /cross.env <<ENV
export RUST_TARGET=$rust
export CARGO_TARGET_${upper}_LINKER=$gnu-gcc
export CC_$lower=$gnu-gcc
export AR_$lower=$gnu-ar
ENV
EOF

# Declared after the toolchain, which both profiles share. Cargo puts each profile's output in a
# directory of its name, but dev's, which it names debug.
ARG CARGO_PROFILE=release
COPY --from=planner /src/recipe.json recipe.json
RUN . /cross.env \
    && cargo chef cook --profile "$CARGO_PROFILE" --locked --target "$RUST_TARGET" -p neon-server \
        --recipe-path recipe.json

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# Compiled into the binary: the migrations, and the query cache the SQL is checked against,
# as there is no database here.
COPY migrations ./migrations
COPY .sqlx ./.sqlx
ENV SQLX_OFFLINE=true
# cargo-auditable writes the crates the binary links, with their versions, into a section of
# the binary, where the image's SBOM scanner reads them: without it, the SBOM would list the
# base image's Debian packages alone. Only the binary's own link changes, so the dependencies
# cooked above are used as they are.
COPY --from=tools /cargo-auditable-*/cargo-auditable /usr/local/cargo/bin/
RUN . /cross.env \
    && cargo auditable build --profile "$CARGO_PROFILE" --locked --target "$RUST_TARGET" -p neon-server --bin neon \
    && cp "target/$RUST_TARGET/$CARGO_PROFILE/neon" /neon \
    && mkdir /data

# ----------------------------------------------------------------------------------------------
# The updater image, the target updater: neon-updater beside the docker CLI, its Compose plugin
# and cosign, which it runs. Its stages build only for that target.

# The crates neon-updater links.
FROM about AS updater-notices
RUN cargo about generate --locked --fail -m crates/updater/Cargo.toml -c about.toml \
        -o /THIRD_PARTY_NOTICES.txt about.hbs

# neon-updater, linked statically against musl for the docker CLI image's Alpine. None of its
# crates compiles C, so rust-lld, which comes with the toolchain, links it for either platform:
# no C toolchain for the other platform, and no emulation.
FROM chef AS updater-build
ARG TARGETARCH
RUN <<'EOF'
set -eu
case "$TARGETARCH" in
    amd64) rust=x86_64-unknown-linux-musl ;;
    arm64) rust=aarch64-unknown-linux-musl ;;
    *) echo "unsupported platform: linux/$TARGETARCH" >&2; exit 1 ;;
esac
rustup target add "$rust"
upper=$(echo "$rust" | tr 'a-z-' 'A-Z_')
cat > /musl.env <<ENV
export RUST_TARGET=$rust
export CARGO_TARGET_${upper}_LINKER=rust-lld
ENV
EOF
ARG CARGO_PROFILE=release
COPY --from=planner /src/recipe.json recipe.json
RUN . /musl.env \
    && cargo chef cook --profile "$CARGO_PROFILE" --locked --target "$RUST_TARGET" -p neon-updater \
        --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# As for the server: the crates it links, written into the binary for the SBOM's scanner.
COPY --from=tools /cargo-auditable-*/cargo-auditable /usr/local/cargo/bin/
RUN . /musl.env \
    && cargo auditable build --profile "$CARGO_PROFILE" --locked --target "$RUST_TARGET" \
        -p neon-updater --bin neon-updater \
    && cp "target/$RUST_TARGET/$CARGO_PROFILE/neon-updater" /neon-updater \
    && mkdir /home-neon-updater

# The two official images the updater image is made of, for the image's platform.
FROM docker:29.10.1-cli-alpine3.24@sha256:4e60c31615e83523afbb59773b25d055c0da9edcd5f80a4c6a6eebad49de1a25 AS docker-cli
FROM ghcr.io/sigstore/cosign/cosign:v3.1.6@sha256:7d08ec40d6f6fd9ced81a498c367ee6d4dd90d73218f58a791f78661bdbef769 AS cosign

# The license notices of the docker CLI, its Compose and Buildx plugins and cosign, and of the Go
# modules each links: Apache-2.0 asks for a copy of the license, and MIT and BSD for their
# notice in every copy, and neither official image carries them. As for Caddy's in the web
# image, each module comes from the Go module proxy, which go checks against the checksum
# database, then against the hash the program records, where it records one: Buildx, built
# from its vendor directory, records none. Neither does the docker CLI, built from its vendor
# directory without module information at all: its modules are the ones vendor/modules.txt
# lists in its source, at the version its build stamped into it. Every license, copying, notice
# and patents file at a module's root goes in, and a module with none fails the build, unless
# the list below names it: a module that ships no license file but names its license in
# another of its files, which goes in with that license's text as another module ships it. An
# entry of the list that no module needs fails the build too. The programs are read, not run,
# so this stage runs on the builder's platform for either image.
FROM --platform=$BUILDPLATFORM golang:1.27.4-alpine3.24@sha256:70bd7ddea8b6cbe5d6943cde279d6b67feb10cc924fb9100d1d8757c1575b280 AS go-notices
ENV GOTOOLCHAIN=local GOPROXY=https://proxy.golang.org GOSUMDB=sum.golang.org GOFLAGS=-modcacherw
COPY --from=docker-cli /usr/local/bin/docker /programs/docker/docker
COPY --from=docker-cli /usr/local/libexec/docker/cli-plugins/docker-compose /programs/docker/docker-compose
COPY --from=docker-cli /usr/local/libexec/docker/cli-plugins/docker-buildx /programs/docker/docker-buildx
COPY --from=cosign /ko-app/cosign /programs/cosign/cosign
RUN --mount=type=cache,target=/go/pkg/mod,sharing=locked <<'EOF'
set -eu
# The modules without a license file: each one's path and version, the file that names its
# license, that license, and the module whose LICENSE file holds its text.
cat > /unlicensed <<'LIST'
github.com/alibabacloud-go/cr-20160607 v1.0.1 README.md Apache-2.0 github.com/sigstore/cosign/v3
LIST
: > /unlicensed.used
# Each program's modules, one per line: its path, its version, and the hash of its files that
# the program records, or - where it records none.
for group in /programs/*; do
    for program in "$group"/*; do
        name="${program##*/}"
        go version -m "$program" > /info
        if grep -q "^$(printf '\t')=>" /info; then
            echo "$name: go version -m lists a replaced module, which this stage does not handle" >&2
            exit 1
        fi
        awk -F'\t' '$2 == "dep" { print $3, $4, ($5 == "" ? "-" : $5) }' /info > "/modules.$name"
        main="$(awk -F'\t' '$2 == "mod" { print $3, $4 }' /info)"
        if [ -z "$main" ]; then
            version="$(sed -n 's/.*github\.com\/docker\/cli\/cli\/version\.Version=\([0-9][^ "\\]*\).*/\1/p' /info)"
            [ -n "$version" ] || { echo "$name: go version -m names no module" >&2; exit 1; }
            main="github.com/docker/cli v$version+incompatible"
            set -- $main
            dir="$(go mod download -json "$1@$2" | awk -F'"' '/^\t"Dir"/ { print $4 }')"
            [ -f "$dir/vendor/modules.txt" ] || { echo "$1@$2 has no vendor/modules.txt" >&2; exit 1; }
            if grep -q '^# .*=>' "$dir/vendor/modules.txt"; then
                echo "$1@$2 vendors a replaced module, which this stage does not handle" >&2
                exit 1
            fi
            awk '$1 == "#" { print $2, $3, "-" }' "$dir/vendor/modules.txt" >> "/modules.$name"
        fi
        echo "$main -" >> "/modules.$name"
        printf '%s\t%s\t%s\n' "$name" "$main" "$(head -n 1 /info | sed 's/^[^:]*: //')" >> "/programs.${group##*/}"
    done
done
# Each module once, with the hash a program records for it, if one does.
sort -u /modules.* | awk '
    { key = $1 " " $2 }
    $3 != "-" && sums[key] != "" && sums[key] != $3 { print "two programs record two hashes for " key > "/dev/stderr"; exit 1 }
    $3 != "-" { sums[key] = $3 }
    !(key in seen) { seen[key] = 1; order[n++] = key }
    END { for (i = 0; i < n; i++) print order[i], (sums[order[i]] == "" ? "-" : sums[order[i]]) }' > /modules
# One JSON object per module, each field on a line of its own.
go mod download -json $(awk '{ print $1 "@" $2 }' /modules) > /downloads.json
awk -F'"' '/^\t"Path"/ { path = $4 } /^\t"Version"/ { version = $4 } /^\t"Sum"/ { sum = $4 }
    /^\t"Dir"/ { dir = $4 } /^}/ { print path, version, sum, dir }' /downloads.json > /downloads
rule="$(printf '=%.0s' $(seq 80))"
thin="$(printf -- '-%.0s' $(seq 80))"
missing=""
for group in /programs/*; do
    group="${group##*/}"
    mkdir -p "/notices/$group"
    notices="/notices/$group/THIRD_PARTY_NOTICES.txt"
    {
        echo "Third-party notices"
        echo
        echo "These programs of the updater image, from the official image named after the first, are"
        echo "built from the Go modules below:"
        echo
        awk -F'\t' '{ printf "  %s: %s, built with %s\n", $1, $2, $3 }' "/programs.$group"
        echo
        echo "Each section is one module, at the version the programs were built with, and the"
        echo "license files it ships. The last is Go's own, for the standard library and runtime the"
        echo "programs are compiled with."
    } > "$notices"
    sort -u $(awk -F'\t' '{ print "/modules." $1 }' "/programs.$group") | awk '{ print $1, $2 }' | uniq > /group
    while read -r path version; do
        sum="$(awk -v p="$path" -v v="$version" '$1 == p && $2 == v { print $3 }' /modules)"
        found="$(awk -v p="$path" -v v="$version" '$1 == p && $2 == v' /downloads)"
        [ -n "$found" ] || { echo "go mod download gave nothing for $path $version" >&2; exit 1; }
        set -- $found
        if [ "$sum" != - ] && [ "$3" != "$sum" ]; then
            echo "$path $version: the download's hash $3 is not the program's, $sum" >&2
            exit 1
        fi
        files="$(find "$4" -maxdepth 1 -type f \( -iname 'licen[cs]e*' -o -iname 'copying*' \
            -o -iname 'notice*' -o -iname 'patents*' \) | sort)"
        dir="$4"
        if [ -z "$files" ]; then
            entry="$(awk -v p="$path" -v v="$version" '$1 == p && $2 == v { print $3, $4, $5 }' /unlicensed)"
            if [ -z "$entry" ]; then
                missing="$missing $path@$version"
                continue
            fi
            set -- $entry
            text="$(awk -v p="$3" '$1 == p { print $4; exit }' /downloads)/LICENSE"
            if ! grep -qF "$2" "$dir/$1" || [ ! -f "$text" ]; then
                echo "$path $version: its $1 does not name $2, or $3 has no LICENSE" >&2
                exit 1
            fi
            printf '\n%s\n%s %s\n%s\n' "$rule" "$path" "$version" "$thin" >> "$notices"
            printf '\nNo license file: its %s names its license, %s.\n\n%s:\n\n' "$1" "$2" "$1" >> "$notices"
            cat "$dir/$1" >> "$notices"
            printf '\n%s, as %s ships it in LICENSE:\n\n' "$2" "$3" >> "$notices"
            cat "$text" >> "$notices"
            echo "$path $version" >> /unlicensed.used
            continue
        fi
        printf '\n%s\n%s %s\n%s\n' "$rule" "$path" "$version" "$thin" >> "$notices"
        for file in $files; do
            printf '\n%s:\n\n' "${file##*/}" >> "$notices"
            cat "$file" >> "$notices"
        done
    done < /group
    printf '\n%s\nGo, its standard library and runtime\n%s\n' "$rule" "$thin" >> "$notices"
    for file in LICENSE PATENTS; do
        printf '\n%s:\n\n' "$file" >> "$notices"
        cat "/usr/local/go/$file" >> "$notices"
    done
done
if [ -n "$missing" ]; then
    echo "no license file in:$missing" >&2
    exit 1
fi
unused="$(awk '{ print $1, $2 }' /unlicensed | grep -vxF -f /unlicensed.used || true)"
if [ -n "$unused" ]; then
    echo "the list of modules without a license file names one no program links, or one that has a license file now: $unused" >&2
    exit 1
fi
EOF

# The docker CLI with its Compose plugin, on Alpine, as Docker publishes it: the updater runs
# it and cosign with their arguments as they are, and no shell reads them. Nothing runs here
# but copies, so no step needs the other platform emulated.
FROM docker-cli AS updater

COPY --from=cosign /ko-app/cosign /usr/local/bin/cosign
COPY --from=updater-build /neon-updater /usr/local/bin/neon-updater
COPY --from=updater-notices /THIRD_PARTY_NOTICES.txt /usr/share/doc/neon-updater/THIRD_PARTY_NOTICES.txt
COPY --from=go-notices /notices/docker/THIRD_PARTY_NOTICES.txt /usr/share/doc/docker/THIRD_PARTY_NOTICES.txt
COPY --from=go-notices /notices/cosign/THIRD_PARTY_NOTICES.txt /usr/share/doc/cosign/THIRD_PARTY_NOTICES.txt
# The home of the unprivileged user neon-updater serve runs as, for the docker CLI's own files.
COPY --from=updater-build --chown=65532:65532 /home-neon-updater /home/neon-updater

LABEL org.opencontainers.image.title="neon-teamspace-updater" \
      org.opencontainers.image.description="Neon Teamspace's updater, which updates a Compose stack to a release" \
      org.opencontainers.image.source="https://github.com/lein3000zzz/neon-teamspace" \
      org.opencontainers.image.licenses="PolyForm-Noncommercial-1.0.0"

# Root to start, as the socket's group is the host's to choose: neon-updater serve looks at the
# socket, then starts itself again as the unprivileged user 65532 in the socket's group, where
# that group may use it. An update's own container runs as root, to write the env file.
USER root

# The updater's HTTP API, for the server on the stack's network. Never publish it.
EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/neon-updater"]
CMD ["serve"]

# glibc, libgcc and CA certificates, no shell, running as the unprivileged user nonroot.
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:29ee70762214fe8190cea04ef1c0a84a8365879e047ce8f911ae5a5ab2355fa2

COPY --from=build /neon /usr/local/bin/neon
COPY --from=notices /THIRD_PARTY_NOTICES.txt /usr/share/doc/neon/THIRD_PARTY_NOTICES.txt
# The data directory, attachments on local disk among it, owned by nonroot. A volume mounted
# there takes its owner from it the first time.
COPY --from=build --chown=65532:65532 /data /var/lib/neon

LABEL org.opencontainers.image.title="neon-teamspace-server" \
      org.opencontainers.image.description="Neon Teamspace's server, the neon binary" \
      org.opencontainers.image.source="https://github.com/lein3000zzz/neon-teamspace" \
      org.opencontainers.image.licenses="PolyForm-Noncommercial-1.0.0"

# Heap profiling switched on in jemalloc, which reads this once as the process starts. It costs
# little until profiling.profiles lists heap; see the observability docs.
ENV _RJEM_MALLOC_CONF=prof:true,prof_active:true,lg_prof_sample:19
ENV NEON_SERVER__DATA_DIR=/var/lib/neon

# 8080 is the HTTP API, for the web image's Caddy to reach. 9090 is the admin listener, with
# metrics, health checks and profiles. It stays on 127.0.0.1 unless
# NEON_OBSERVABILITY__ADMIN__ADDR says otherwise, as on a private network for a scraper; never
# publish it.
EXPOSE 8080 9090

# `neon health` asks the admin listener's /readyz, which also works on loopback.
HEALTHCHECK --interval=10s --timeout=5s --start-period=30s --retries=3 \
    CMD ["/usr/local/bin/neon", "health"]

ENTRYPOINT ["/usr/local/bin/neon"]
CMD ["serve"]
