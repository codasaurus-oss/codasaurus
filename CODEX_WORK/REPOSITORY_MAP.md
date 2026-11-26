# Rust Repository Map

## Scope and snapshot

This map covers the Rust package at repository commit `92b7e09` on branch `main`.
The working tree was clean before this document set was created. The React dashboard
submodule (`codasaurus-frontend/`) is explicitly out of scope for this audit phase;
it is recorded only where the Rust build and server integrate with it.

## Package topology

Codasaurus is one Cargo package, `codasaurus` (version `0.1.0`, edition 2021,
MSRV 1.90), with both a library (`src/lib.rs`) and a CLI binary (`src/main.rs`).
It is not a Cargo workspace. `Cargo.lock` is committed and CI uses `--locked`.

The binary exposes `serve`, `health`, `version`, `reset-password`, and queue
replay/scheduling commands. Release builds use LTO, one codegen unit, stripping,
`opt-level = 3`, and `panic = "abort"`.

## Runtime architecture

```text
CLI (main.rs)
  -> unified Axum server (serve.rs)
       -> REST API (/api/*) ------> PostgreSQL (db/*)
       -> GitHub webhook (/webhook) -> durable review queue -> workers
       -> static dashboard assets (codasaurus-frontend/dist; external React submodule builds them)
       -> Redis (rate limiting, bootstrap/session/OIDC state)

workers -> review pipeline -> parser/config/context -> Tier-1 detectors
                            -> registry/OSV/GitHub APIs (subject to offline mode)
                            -> optional LLM through LiteLLM (budgeted/fail-closed)
                            -> persistence, PR comments, learning/audit events
```

## Major module responsibilities

| Area | Paths | Responsibility |
| --- | --- | --- |
| Entrypoint and composition | `src/main.rs`, `src/lib.rs`, `src/serve.rs` | CLI, process startup, dependency wiring, Axum router, HTTP safety layers, shutdown. |
| Dashboard API | `src/api/` | Setup/bootstrap, authentication/OIDC, RBAC, rate limiting, repositories, settings, review/audit/learning/MCP endpoints, OpenAPI. |
| GitHub review orchestration | `src/bot/`, `src/bot_runtime.rs`, `src/state.rs` | Webhook verification and dispatch, command handling, GitHub API, PR concurrency, durable queue and leases, background workers, review comments and policies. |
| Review pipeline | `src/bot/review/` | Fetches PR data, parses changes, executes deterministic checks and optional reviewer/LLM work, persists and publishes output. |
| Deterministic analysis | `src/detectors/`, `src/parser.rs`, `src/dep_parser.rs`, `src/gates.rs`, `src/confidence.rs`, `src/baseline.rs` | Source/manifest parsing, Tier-1 findings, SARIF and finding identity/classification. |
| Persistence | `src/db/` | PostgreSQL pool, embedded schema migration SQL, dialect macros, data access models for config/users/repos/reviews/audit/events. |
| Configuration and safety | `src/config.rs`, `src/env.rs`, `src/ssrf.rs`, `src/github_jwt.rs`, `src/oidc.rs`, `src/retry.rs` | Configuration resolution, endpoint validation, credential/JWT support, OIDC and retry policy. |
| External service adapters | `src/redis.rs`, `src/registry/`, `src/mcp/`, `src/llm/` | Redis-backed state; package registries; MCP server catalogs/clients; LiteLLM inference, spend and budgets. |
| Context and learning | `src/context/`, `src/learning/`, `src/index/`, `src/graph/`, `src/storage.rs` | Repository guidance/rules, dismissal-derived learning, semantic/code extraction and storage, dependency graphs. |
| Operational support | `src/metrics.rs`, `src/util.rs` | Metrics exposition and shared helpers. |

## Important execution flows

### Server boot

`main` resolves the database URL and command, creates a Tokio runtime, then calls
`serve::serve`. Startup creates the PostgreSQL pool and applies migrations, synchronizes
approved environment-backed settings, resolves bot configuration, sets offline mode,
connects Redis, arms first-owner setup when needed, builds the router, and starts queue
workers. The server owns graceful shutdown and closes the pool afterwards.

### Webhook to completed review

`/webhook` first applies a Redis-backed rate limit; `bot::handle_webhook` authenticates
and de-duplicates deliveries, dispatches GitHub events, and schedules durable
`review_jobs`. Workers claim pending jobs using PostgreSQL `FOR UPDATE SKIP LOCKED`,
apply timeout/retry/lease semantics, invoke the review pipeline, write reviews/findings,
and publish GitHub feedback. Per-PR locks prevent duplicate in-process reviews; database
claims coordinate replicas.

### Review analysis

The pipeline loads repository policy and configuration, gets changed files and PR context,
parses supported languages/manifests, runs the detector registry, applies learning and
quality gates, emits persistence/SARIF/comment material, and may call the LLM subsystem.
Offline mode is an invariant: it must prohibit LLM connections and external registry work.

## External integrations and contracts

| Integration | Rust boundary | Contract/safety focus |
| --- | --- | --- |
| PostgreSQL | `db/*`, `state.rs`, queue | Sole durable store; embedded migrations; leases and review queue correctness. |
| Redis | `redis.rs`, API/bootstrap/rate-limit | Rate limiting and ephemeral security/session state. |
| GitHub | `bot/*`, `github_jwt.rs`, API GitHub setup | Signed webhooks, GitHub App JWT/installation tokens, PR/comment APIs. |
| LiteLLM / LLM provider | `llm/*`, `bot/review/llm.rs` | Optional BYOK inference; SSRF protections, budgets, and offline fail-closed behavior. |
| Registries and OSV | `registry/*`, detector modules | Remote lookup behavior, caching/retry, offline policy. |
| MCP endpoints | `mcp/*`, `api/mcp.rs` | Stored server configuration, client calls, SSRF/credential handling. |
| Dashboard assets | `serve.rs`, `build.rs`, `Dockerfile` | Rust builds and serves `codasaurus-frontend/dist`; the separate React submodule is the canonical artifact location. Frontend source is out of scope. |

## Test and delivery topology

Rust tests are split between module-local unit tests and integration tests under `tests/`:
detectors, golden fixtures, PostgreSQL, Redis, complete backend/hardening, docs links,
and user journeys. Full integration tests need dedicated PostgreSQL and Redis endpoints;
they clear `review_jobs` and must not use a live database.

CI checks formatting, Clippy with warnings denied, locked tests, `cargo audit` with two
documented lockfile-only ignores, cargo-deny license/ban/source policy, release build,
workflow linting, Docker build, DCO, and a separate frontend job. Rust work in this
project must preserve those gates.

## Discovery limits and next evidence

This is a structural map, not a completed audit. Semantic Rust tooling failed to initialize
during discovery, so the map uses targeted source inspection and Cargo metadata. The first
component audit should establish a test baseline and then validate the server/build boundary
before reaching conclusions about implementation defects.
