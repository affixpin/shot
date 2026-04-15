FROM rust:1.88-alpine AS builder
RUN apk add --no-cache musl-dev gcc
WORKDIR /app
COPY . .
RUN cargo build --release --workspace

FROM alpine:3.20
RUN apk add --no-cache ca-certificates curl
COPY --from=builder /app/target/release/shot /app/target/release/armaments /usr/local/bin/
RUN adduser -D agent
USER agent
WORKDIR /home/agent
ENTRYPOINT ["shot"]
