# syntax=docker/dockerfile:1.29.0@sha256:572cebbc2b15e6f84c077742952674889cb63cd5e9b80abdffcbe587b6f19183

# The server image: the neon binary on a minimal base. Build from the repository's root:
#   docker buildx build -f deployments/Dockerfile --platform linux/amd64,linux/arm64 .
#
# The Rust build runs on the builder's own platform and cross-compiles for the image's, so an
# arm64 image builds on an amd64 machine, and the reverse, without emulating the compiler.
#
# The build argument CARGO_PROFILE names the Cargo profile of the binary: release, the default,
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
FROM --platform=$BUILDPLATFORM rust:1.100.0-slim-trixie@sha256:6f14b2b4b278ec0d50aa1d69541c304ce0f95e0f6268487aa9924af1c2e9aebe AS chef

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

# The license notices of the crates the binary links, from each crate's own license files, as
# about.toml at the repository's root sets out. The same for every platform, so it runs once.
# They depend on the manifests and Cargo.lock alone, so they are made in the recipe's skeleton
# of the workspace, which has empty sources, and stay cached while only the code changes.
FROM chef AS notices
COPY --from=tools /cargo-about-*/cargo-about /usr/local/cargo/bin/
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --no-build --recipe-path recipe.json
COPY about.toml about.hbs ./
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
