# syntax=docker/dockerfile:1.29.0@sha256:572cebbc2b15e6f84c077742952674889cb63cd5e9b80abdffcbe587b6f19183

# The server image: the neon binary on a minimal base. Build from the repository's root:
#   docker buildx build -f deployments/Dockerfile --platform linux/amd64,linux/arm64 .
#
# The Rust build runs on the builder's own platform and cross-compiles for the image's, so an
# arm64 image builds on an amd64 machine, and the reverse, without emulating the compiler.

# The Rust version rust-toolchain.toml pins, on Debian 13 like the runtime image below, so the
# binary links against the glibc it runs with. The `chef` stage fails when the two differ.
FROM --platform=$BUILDPLATFORM rust:1.100.0-slim-trixie@sha256:6f14b2b4b278ec0d50aa1d69541c304ce0f95e0f6268487aa9924af1c2e9aebe AS chef

# make, for jemalloc, which every Linux build compiles from its C sources. The image has gcc.
RUN apt-get update \
    && apt-get install -y --no-install-recommends make \
    && rm -rf /var/lib/apt/lists/*
# cargo-chef builds the dependencies alone, from a recipe of the manifests, into a layer that
# stays cached until Cargo.lock or a manifest changes.
RUN cargo install cargo-chef --locked --version 0.1.78

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

COPY --from=planner /src/recipe.json recipe.json
RUN . /cross.env \
    && cargo chef cook --release --locked --target "$RUST_TARGET" -p neon-server --recipe-path recipe.json

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# Compiled into the binary: the migrations, and the query cache the SQL is checked against,
# as there is no database here.
COPY migrations ./migrations
COPY .sqlx ./.sqlx
ENV SQLX_OFFLINE=true
RUN . /cross.env \
    && cargo build --release --locked --target "$RUST_TARGET" -p neon-server --bin neon \
    && cp "target/$RUST_TARGET/release/neon" /neon

# glibc, libgcc and CA certificates, no shell, running as the unprivileged user nonroot.
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:cb87acc574abe719b6e5c547384defa2f7b1f24af9605d3f618cf852495d8be2

COPY --from=build /neon /usr/local/bin/neon

LABEL org.opencontainers.image.title="neon-workspace-server" \
      org.opencontainers.image.description="Neon Workspace's server, the neon binary" \
      org.opencontainers.image.source="https://github.com/lein3000zzz/neon-workspace" \
      org.opencontainers.image.licenses="PolyForm-Noncommercial-1.0.0"

# Heap profiling switched on in jemalloc, which reads this once as the process starts. It costs
# little until profiling.profiles lists heap; see the observability docs.
ENV _RJEM_MALLOC_CONF=prof:true,prof_active:true,lg_prof_sample:19

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
