# syntax=docker/dockerfile:1
FROM rust:1-alpine AS builder
RUN apk add --no-cache musl-dev pkgconfig
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs && \
    cargo build --release --target x86_64-unknown-linux-musl; \
    rm -rf src target/x86_64-unknown-linux-musl/release/.fingerprint/robot-* target/x86_64-unknown-linux-musl/release/deps/robot-* 2>/dev/null; true
COPY src ./src
RUN cargo build --release --target x86_64-unknown-linux-musl && \
    cp target/x86_64-unknown-linux-musl/release/robot /tmp/robot

FROM alpine:3.22
RUN apk add --no-cache ca-certificates
COPY --from=builder /tmp/robot /usr/local/bin/robot
ENTRYPOINT ["/usr/local/bin/robot"]
