# ── Build ─────────────────────────────────────────────────────
FROM rust:1.88-slim AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY packages/shotclaw/Cargo.toml packages/shotclaw/Cargo.lock ./

# Cache dependencies with dummy source
RUN mkdir -p src && echo "fn main() {}" > src/main.rs && echo "pub fn dummy() {}" > src/lib.rs
RUN cargo build --release 2>/dev/null || cargo build --release
RUN rm -rf src

# Build actual binary
COPY packages/shotclaw/src/ src/
COPY packages/shotclaw/defaults/ defaults/
RUN touch src/main.rs src/lib.rs && \
    rm -rf target/release/.fingerprint/shotclaw-* && \
    cargo build --release && \
    strip target/release/shotclaw

# ── Runtime ───────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl jq bash git && \
    apt-get clean && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/shotclaw /usr/local/bin/shotclaw

RUN groupadd -g 1000 agent && useradd -u 1000 -g agent -m -s /bin/bash agent

WORKDIR /home/agent
USER agent

ENTRYPOINT ["shotclaw"]
