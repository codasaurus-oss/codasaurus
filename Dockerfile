# syntax=docker/dockerfile:1

# ── Stage 1: Build the SPA from the pinned codasaurus-frontend submodule ─
# Keep this major in sync with the dashboard jobs in .github/workflows/.
FROM node:26-alpine AS frontend
ARG SKIP_FRONTEND=0
WORKDIR /app
COPY codasaurus-frontend/package.json codasaurus-frontend/package-lock.json ./codasaurus-frontend/
RUN --mount=type=cache,target=/root/.npm \
    if [ "$SKIP_FRONTEND" != "1" ]; then \
      cd codasaurus-frontend && npm ci --ignore-scripts; \
    fi
COPY codasaurus-frontend/ ./codasaurus-frontend/
RUN if [ "$SKIP_FRONTEND" = "1" ]; then \
      mkdir -p svelte-dashboard/dist && \
      printf '<!doctype html><title>Codasaurus</title>' > svelte-dashboard/dist/index.html; \
    else \
      cd codasaurus-frontend && npm run build && \
      mkdir -p /app/svelte-dashboard && \
      cp -r dist /app/svelte-dashboard/dist; \
    fi

# ── Stage 2: Build Rust binary ──────────────────────────────────
# Pin to rust-toolchain.toml: a mismatch makes rustup install a second
# toolchain and recompile every dependency.
FROM rust:1.90-slim-bookworm AS backend
# utoipa-swagger-ui's build script downloads its UI bundle with curl.
RUN apt-get update \
    && apt-get install -y --no-install-recommends curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
ENV CODASAURUS_SKIP_FRONTEND_BUILD=1
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    mkdir -p src && echo "fn main() {}" > src/main.rs \
    && echo "" > src/lib.rs \
    && cargo build --release --locked \
    && rm -rf src

COPY . .
COPY --from=frontend /app/svelte-dashboard/dist/ svelte-dashboard/dist/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked \
    && cp target/release/codasaurus /app/codasaurus

# ── Stage 3: Runtime ────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -r codasaurus --gid 65532 \
    && useradd --no-log-init -r -g codasaurus --uid 65532 --home-dir /data --create-home codasaurus \
    && mkdir -p /data /tmp/codasaurus \
    && chown -R 65532:65532 /data /tmp/codasaurus
WORKDIR /app
COPY --link --from=frontend /app/svelte-dashboard/dist/ /app/svelte-dashboard/dist/
COPY --link --from=backend /app/codasaurus /usr/local/bin/codasaurus
USER 65532:65532
ENV PORT=3000 \
    CODASAURUS_DATA_DIR=/data \
    HOME=/data
EXPOSE 3000
# Render sets PORT; health CLI reads $PORT when --port omitted.
# 8s must stay above the CLI's own 5s HTTP timeout.
HEALTHCHECK --interval=30s --timeout=8s --start-period=90s --retries=5 \
  CMD ["codasaurus", "health", "--ready"]
ENTRYPOINT ["codasaurus"]
CMD ["serve", "--host", "0.0.0.0"]
