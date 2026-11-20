# ---- build stage -----------------------------------------------------------
FROM rust:1-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libasound2-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
COPY tests ./tests

# NOTE: classic builder compatible (no BuildKit cache mounts). For fast
# iteration use the builder stage + a registry volume (see README dev notes).
RUN cargo build --release && \
    cp target/release/craved /usr/local/bin/craved

# ---- runtime stage ---------------------------------------------------------
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    libasound2 ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /usr/local/bin/craved /usr/local/bin/craved
COPY web ./web

# Pattern library + settings live here (mount a volume)
VOLUME /data
EXPOSE 8090

ENTRYPOINT ["craved"]
CMD ["--port", "8090", "--data-dir", "/data", "--web-dir", "/app/web"]
