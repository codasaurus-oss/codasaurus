# syntax=docker/dockerfile:1

# ── Stage 1: Build the SPA from codasaurus-frontend ─────────────
FROM node:20-alpine AS frontend
ARG DASHBOARD_REF=main
RUN apk add --no-cache git
WORKDIR /app
RUN git clone --depth 1 --branch "${DASHBOARD_REF}" \
      https://github.com/codasaurus-oss/codasaurus-frontend.git svelte-dashboard
WORKDIR /app/svelte-dashboard
RUN npm ci --ignore-scripts
RUN npm run build

# ── Stage 2: Build Rust binary ──────────────────────────────────
FROM rust:1.90-slim-bookworm AS backend
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
ENV CODASAURUS_SKIP_FRONTEND_BUILD=1
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/app/target \
    mkdir -p src && echo "fn main() {}" > src/main.rs \
    && echo "" > src/lib.rs \
    && cargo build --release --locked 2>&1 | tail -1 \
    && rm -rf src

COPY . .
COPY --from=frontend /app/svelte-dashboard/dist/ svelte-dashboard/dist/
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/app/target \
    cargo build --release --locked \
    && cp target/release/codasaurus /app/codasaurus \
    && strip /app/codasaurus

# ── Stage 3: Runtime ────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -r codasaurus --gid 65532 \
    && useradd -r -g codasaurus --uid 65532 --home-dir /data --create-home codasaurus \
    && mkdir -p /data /tmp/codasaurus \
    && chown -R 65532:65532 /data /tmp/codasaurus
WORKDIR /app
COPY --from=frontend /app/svelte-dashboard/dist/ /app/svelte-dashboard/dist/
COPY --from=backend /app/codasaurus /usr/local/bin/codasaurus
USER 65532:65532
ENV PORT=3000 \
    CODASAURUS_DATA_DIR=/data \
    HOME=/data
EXPOSE 3000
# Render sets PORT; health CLI reads $PORT when --port omitted.
HEALTHCHECK --interval=30s --timeout=8s --start-period=90s --retries=5 \
  CMD ["codasaurus", "health", "--ready"]
ENTRYPOINT ["codasaurus"]
CMD ["serve", "--host", "0.0.0.0"]
