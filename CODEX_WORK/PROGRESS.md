# Audit Progress

## Overall status

- Phase: R-12 auditing in progress.
- Rust components mapped: 17 of 17.
- Components audited: 11 of 17 (R-01 through R-11).
- Components changed: 9 of 17 (R-01, R-03, R-04, R-09, plus accepted-risk reviews).
- Current component: R-13 — Learning, semantic index, and graph.

## Completed in this batch

- Read root `AGENTS.md` and the supplied engineering brief.
- Confirmed a single Rust package at `Cargo.toml`; edition 2021, MSRV/toolchain 1.90,
  library plus CLI binary, committed lockfile, and no Cargo workspace members.
- Mapped Rust source directories, test targets, embedded PostgreSQL schema, queue/worker
  pattern, HTTP/API topology, external integrations, CI, container delivery, and direct
  dependency/features from Cargo metadata.
- Created the required repository map, work breakdown, decision log, and handoff record.
- Audited R-01 and resolved F-R01-001: migrated the Rust build, server, container
  artifact handoff, and related-PR source-path heuristic from the removed
  `svelte-dashboard` directory to `codasaurus-frontend`.
- Started and audited R-02: mapped SSRF, LLM endpoint, MCP endpoint, OIDC, GitHub key,
  and issue-tracker configuration boundaries. Core controls passed focused tests; redirect
  behavior is retained as an accepted-risk follow-up pending an explicit compatibility contract.
- Audited R-03: verified migration ordering and corrected missing schema-version records for
  v20 and v21 without changing their DDL.
- Audited R-04: traced user, session, invite, repo, review, and config repository paths;
  identified and fixed concurrent OIDC provisioning.
- Audited R-05: reviewed session/bootstrap state, auth middleware, RBAC, rate limiting,
  and trusted-proxy handling; no additional confirmed source defect was found.
- Audited R-06: reviewed startup ordering, router and middleware layering, static/API fallback,
  health/readiness, metrics authorization, and graceful shutdown; no confirmed source defect was found.
- Audited R-07: verified constant-time HMAC validation, empty-secret fail-closed behavior,
  author ACLs, delivery deduplication, and event dispatch branches. No confirmed auth defect was found;
  detached task failure/retry semantics remain an operational follow-up.
- Audited R-08 at source level: reviewed atomic `SKIP LOCKED` claims, lease ownership checks,
  heartbeats, exponential retry/dead-letter transitions, LISTEN wakeups, bounded worker permits,
  and shutdown/task behavior. No confirmed queue defect was found; PostgreSQL-backed assertions remain pending.
- Started R-09: fixed installation repository pagination so non-success GitHub responses propagate
  as errors instead of being reported as an empty successful sync, and bounded same-SHA review
  detection across ten review-history pages. The PR commit-history fetch now validates status,
  retries failures, and logs its explicit best-effort fallback. Review SHA completion now follows
  durable persistence and propagates failures to queue retry handling.
- Completed R-09 source review: external fetch/status handling, bounded pagination, review output,
  persistence ordering, claim release, and check-run/label best-effort paths were reviewed.
- Completed R-10: reviewed parsing, dependency extraction, baseline filtering, confidence floors,
  gate semantics, policy ordering, and offline fail-closed paths; no confirmed defect was found.
- Completed R-11: reviewed detector registry and parallel execution, finding contracts, SARIF output,
  golden fixtures, and network/offline detector behavior; no confirmed detector defect was found.
- Completed R-12: reviewed LLM config precedence, endpoint SSRF checks, offline blocking, prompt
  fencing, structured-response validation, budget parsing, and cost estimates; no confirmed defect was found.
- Completed R-10: reviewed parsing, dependency extraction, baseline filtering, confidence floors,
  gate semantics, policy ordering, and offline fail-closed paths; no confirmed defect was found.

## Validation performed

| Check | Result |
| --- | --- |
| `git status --short` before documentation | Clean working tree. |
| `cargo locate-project --workspace` | Root manifest is `Cargo.toml`. |
| `cargo metadata --no-deps --format-version 1` | One package (`codasaurus`), lib/bin/eight integration-test targets, build script. |
| `rustc --version --verbose`; `cargo --version` | Rust/Cargo 1.90.0; host `aarch64-apple-darwin`. |
| Targeted source/config inspection | Completed for module maps, server startup/router, queue/worker indicators, migrations, Cargo/CI/container configuration. |
| `cargo fmt --check` | Passed after the R-01 patch. |
| `cargo build --locked` | Passed after allowing utoipa-swagger-ui to download its pinned Swagger UI bundle. |
| Focused build/related-PR tests | Passed: direct `build.rs` test plus `source_paths_outrank_meta` and `select_skips_meta_when_source_available`. |
| Docker frontend stage | Passed; produced and verified `codasaurus-frontend/dist/index.html` in the image. |
| R-02 focused security tests with DNS access | Passed: 10 SSRF, 4 OIDC/redaction, and 5 GitHub JWT tests. |
| `cargo fmt --check`; `cargo clippy --locked --all-targets -- -D warnings` | Passed for the current tree. |
| R-03 focused migration regression | Passed: `every_numbered_migration_records_its_version` (v8–v29). |
| R-04 focused database-library tests | Passed: 17 tests, including OIDC provisioning coverage. |
| R-06 focused server-library tests | Passed: 2 tests in `serve::`. |
| R-07 bot-library tests | 120 passed; 2 PostgreSQL-backed tests failed before assertions because local DB access is unavailable in the sandbox. |
| R-08 queue/worker tests | Queue notification test reached PostgreSQL setup and failed with sandbox `Operation not permitted`; no assertion result claimed. |
| R-09 focused Rust gates | `cargo fmt --check`, API test target (0 module tests), and locked Clippy passed. |
| R-09 review-history tests | Passed 2 focused `bot::review::github::tests`; locked Clippy passed after pagination fix. |
| R-09 pipeline status handling | Passed formatting, focused review tests, and locked Clippy after commit-history fix. |
| R-09 persistence ordering | Passed formatting, focused review tests, and locked Clippy after SHA/persistence fix. |
| R-10 focused modules | Passed: parser/dep-parser (19), gates/policy (8), confidence (8), baseline (3), and offline/LLM/registry (7) tests. |
| R-11 detector integration | Passed: 19 integration tests and 3 golden detector tests. |
| R-11 detector unit suite | 54 of 55 passed; the one failure was the network-dependent public npm lookup test under restricted networking. |
| R-12 LLM module tests | 32 passed; 13 gateway tests failed during local test-server setup with sandbox `Operation not permitted`. |
| R-09 completion gate | No live GitHub or PostgreSQL contract test claimed; all available focused Rust checks passed. |
| R-10 focused modules | Passed: parser/dep-parser (19), gates/policy (8), confidence (8), baseline (3), and offline/LLM/registry (7) tests. |

No Clippy, full test suite, benchmark, dependency audit/tree, or database/Redis integration
command has been run. No such check is claimed as passing.

## Open risks recorded for future verification

1. Rust semantic tooling could not initialize during discovery. The map was assembled with
   Cargo metadata and targeted text/source inspection. Retry only if it becomes useful to
   the selected component; do not treat this as application failure evidence.

## Pending work

1. Start R-13: learning, semantic index, and graph.
2. Retain F-R02-001 as a follow-up design item; do not implement arbitrary redirects
   without a compatibility requirement and redirect-specific tests.
3. Update `FINDINGS.md`, `TEST_AUDIT.md`, and this file after the component batch.
