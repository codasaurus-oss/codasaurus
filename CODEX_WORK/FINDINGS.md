# Findings

## F-R04-001 — OIDC user provisioning used a read-then-insert race

- Component: R-04 (Database repositories and identity state)
- Category: concurrency / identity correctness
- Severity: HIGH
- Location: `src/db/users.rs::upsert_oidc_user`
- Problem: The function first queried by email and then inserted a new user. Concurrent
  OIDC callbacks for a new email could both observe absence and one would fail on the
  unique `users.email` constraint.
- Evidence: the old implementation had separate `SELECT` and `INSERT` statements;
  `users.email` is unique in the embedded schema.
- Impact: an otherwise valid concurrent login could return an internal database error.
- Proposed solution: use PostgreSQL `INSERT ... ON CONFLICT (email) DO NOTHING RETURNING *`,
  then select the existing row when another request won the race.
- Risk: low; existing users and roles are unchanged, and the function still returns the
  stored user rather than overwriting identity state.
- Confidence: confirmed.
- Validation method: focused database-library tests, formatting, Clippy, and PostgreSQL
  concurrency smoke coverage when the local service is available.
- Status: RESOLVED (focused library tests passed; PostgreSQL concurrency smoke remains environment-dependent)

## F-R03-001 — v20 and v21 migrations did not record completion

- Component: R-03 (Database foundation and schema)
- Category: migration correctness / operational efficiency
- Severity: MEDIUM
- Location: `src/db/migrations.rs` (`migrate_v20_pre_merge_checks`, `migrate_v21_learning_rule_status`)
- Problem: Both migrations applied their DDL and returned without inserting their version
  into `schema_version`.
- Evidence: the migration runner invokes v20 then v21; neighboring numbered migrations
  record completion, while these two did not.
- Impact: inaccurate migration history and repeated idempotent DDL checks on startup until
  a later migration masks the omission.
- Proposed solution: record versions 20 and 21 after successful DDL using the existing
  idempotent `ON CONFLICT` pattern.
- Risk: low; only records already-applied schema changes.
- Confidence: confirmed by source inspection.
- Validation: focused migration regression passed; `cargo fmt` and locked Clippy passed.
- Status: RESOLVED

## F-R02-001 — Redirect policy for user-configured LLM and Jira endpoints needs an explicit contract

- Component: R-02 (Configuration and trust boundaries)
- Category: SSRF / outbound transport
- Severity: HIGH
- Location: `src/llm/client.rs`, `src/bot/repo_context.rs`
- Problem: URL validation occurs before an outbound request, but the corresponding
  reqwest clients use the default redirect policy. A user-configured public endpoint can
  redirect the server to a private/metadata address without re-validation. Jira also uses
  the non-resolving URL guard before its request.
- Evidence: MCP's separately configured client explicitly uses
  `reqwest::redirect::Policy::none()`, whereas these two clients did not. The Jira
  callsite used `validate_http_url` rather than `validate_http_url_resolved`.
- Proposed solution: establish whether redirects are required for these integrations. If
  they are, use a bounded manual redirect loop that validates every `Location` target
  (scheme, host, resolved addresses) before the next request and defines whether auth is
  retained cross-origin. Otherwise, explicitly disable redirects and document that API
  endpoints must be canonical URLs.
- Risk: medium; either alternative changes an unspoken external integration behavior.
- Validation method: deterministic local redirect tests covering safe same-origin,
  unsafe metadata/private targets, redirect limits, and credential forwarding rules.
- Decision: API base URLs are treated as canonical integration endpoints. Redirects are
  not followed by MCP, and this audit does not silently alter LLM/Jira behavior without
  a compatibility requirement and a deterministic redirect test. The residual risk is
  retained for a dedicated outbound-transport change with an explicit contract.
- Status: ACCEPTED RISK — follow-up required before enabling arbitrary redirect support

## F-R07-001 — Detached webhook tasks have no delivery-level completion signal

- Component: R-07 (Webhook authentication and GitHub event dispatch)
- Category: operational reliability / task lifecycle
- Severity: MEDIUM
- Location: `src/bot/mod.rs::handle_webhook`
- Observation: authenticated events are acknowledged after spawning detached tasks. Failures
  in command, reaction, installation, or indexing tasks are logged inside the task (or can be
  lost during process shutdown), while GitHub has already received a successful response.
- Impact: a deploy or transient process failure can lose non-queue webhook work without a retry.
- Decision: retain current behavior for now because changing acknowledgements requires an explicit
  bounded processing/outbox contract and integration tests; review with R-08 worker lifecycle.
- Status: ACCEPTED RISK — follow-up in durable queue/worker phase

## F-R09-001 — Installation repository sync accepted non-2xx responses

- Component: R-09 (GitHub PR data and review pipeline)
- Category: external API error handling
- Severity: MEDIUM
- Location: `src/api/repos.rs::installation_repos`
- Problem: the paginated repository request decoded the response body without calling
  `error_for_status`, so a GitHub 401/403/5xx response could be treated as an empty page
  and reported as a successful zero-repository sync.
- Fix: require a successful HTTP status before decoding each page, preserving retry/error
  propagation to the authenticated API caller.
- Validation: `cargo fmt --check`, focused API-library test target (no unit tests present),
  and locked Clippy passed.
- Status: RESOLVED

## F-R01-001 — Confirmed obsolete dashboard directory contract

- Component: R-01 (Build, package, and runtime contracts)
- Category: correctness / build and runtime integration
- Severity: HIGH
- Location: `build.rs`, `src/serve.rs`, `Dockerfile`, and `src/bot/related_prs.rs`
- Current implementation: Rust build, static serving, Docker artifact copying, and
  dashboard-path relevance scoring referred to `svelte-dashboard`.
- Problem: `svelte-dashboard` no longer exists; the canonical dashboard submodule is
  `codasaurus-frontend`. A local Cargo invocation skipped the dashboard build, the Rust
  server did not look in the canonical directory for assets, and Docker preserved an
  obsolete compatibility directory.
- Evidence: targeted path check confirmed `codasaurus-frontend/` exists and
  `svelte-dashboard/` does not; `cargo test --locked frontend_manifest` emitted
  `svelte-dashboard/package.json not found, skipping frontend build`.
- Impact: local builds silently omit dashboard generation; non-container server startup
  cannot serve dashboard assets from the canonical directory; path relevance scoring is
  stale.
- Proposed solution: make `codasaurus-frontend` the sole frontend path in the Rust build
  script, static server, Docker stages, and related-PR source-path heuristics. The Docker
  frontend stage leaves Vite's output in that canonical path rather than copying `dist`
  onto itself.
- Risk: low. The change replaces an absent path with the repository's existing submodule
  path and preserves the `CODASAURUS_SKIP_FRONTEND_BUILD` contract.
- Confidence: confirmed.
- Validation method: focused build-script and related-path tests; locked Rust build;
  Docker build if local Docker is available.
- Validation: `cargo build --locked` passed after allowing the dependency's pinned Swagger
  UI download; a forced verbose Cargo build invoked `codasaurus-frontend` and emitted its
  canonical rerun paths; direct build-script test passed; focused related-PR tests passed;
  Docker's `frontend` target built successfully and contained
  `/app/codasaurus-frontend/dist/index.html`.
- Status: RESOLVED
