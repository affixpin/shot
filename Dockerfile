# ── Build ─────────────────────────────────────────────────────
FROM rust:1.88-slim AS builder

RUN apt-get update && \
    apt-get install -y --no-install-recommends pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

RUN cargo build --release --workspace && \
    strip target/release/shot target/release/armaments

# ── Runtime ───────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl bash && \
    apt-get clean && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/shot /usr/local/bin/shot
COPY --from=builder /app/target/release/armaments /usr/local/bin/armaments

RUN groupadd -g 1000 agent && useradd -u 1000 -g agent -m -s /bin/bash agent

WORKDIR /home/agent
USER agent

ENTRYPOINT ["shot"]
