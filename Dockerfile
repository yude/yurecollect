# Multi-stage build for yurecollect

# Build stage
FROM rust:1.88-trixie AS builder
WORKDIR /app

# Copy sources
COPY . /app

# Build release binary
RUN cargo build --release

# Runtime stage
FROM debian:trixie-slim
# Install minimal runtime deps (optional)
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*

# Non-root user
RUN useradd -u 10001 -r -s /usr/sbin/nologin appuser

# Copy binary
COPY --from=builder /app/target/release/yurecollect /usr/local/bin/yurecollect

EXPOSE 3000
ENV RUST_LOG=info
# Optionally set WS_URL via environment or pass as arg
ENV WS_URL="wss://unstable.kusaremkn.com/yure/"

USER appuser
ENTRYPOINT ["/usr/local/bin/yurecollect"]
# Default: no args, override with `docker run ... yurecollect <ws-url>` if desired
