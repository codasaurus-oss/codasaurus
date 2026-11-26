# Audit Progress

## Overall status

- Phase: R-17 mapped; R-16 complete.
- Rust components mapped: 17 of 17.
- Components audited: 16 of 17 (R-01 through R-16).
- Components changed: 11 of 17 (R-01, R-03, R-04, R-09, R-13, R-14, R-15, plus accepted-risk reviews).
- Current component: R-17 — Dependency and supply-chain governance.

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
- Completed R-14: reviewed registry and OSV request construction, transient status handling,
  retry behavior, cache TTL/eviction, batch/detail parsing, bounded prefetch, and detector
  handling of unknown results. Added shared retries for transient registry 429/5xx responses;
  missing and soft-failure semantics remain fail-open.
- Completed R-15: reviewed authenticated route composition, per-handler RBAC, secret redaction,
  setup/config validation, audit pagination, GitHub credential paths, and MCP SSRF/redirect/body
  limits and tool allowlisting. Added strict dynamic MCP server-ID validation.
- Completed R-16: classified the complete Rust test inventory and retained the existing separation
  between deterministic tests and PostgreSQL/Redis, local HTTP-server, and public-network tests.
  No flaky-test workaround or coverage-reducing skip was introduced.
- Completed R-11: reviewed detector registry and parallel execution, finding contracts, SARIF output,
  golden fixtures, and network/offline detector behavior; no confirmed detector defect was found.
- Completed R-12: reviewed LLM config precedence, endpoint SSRF checks, offline blocking, prompt
  fencing, structured-response validation, budget parsing, and cost estimates; no confirmed defect was found.
- Completed R-13: reviewed learning scope precedence, dismissal/rule matching, semantic index
  extraction and persistence, embedding validation, graph traversal, and context parsing; fixed
  the embedding batch limit and found no additional confirmed defect.
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
| R-13 focused modules | Passed: config batch-limit regression (1), index/extraction (13), learning scope/mine (8), and locked Clippy. |
| R-14 focused registry tests | 12 of 13 registry tests passed, including retry/status classification, OSV parsing helpers, metadata extractors, cache/URL tests, and offline behavior; the local OSV mock-server test failed at socket bind with sandbox `Operation not permitted`. Locked Clippy passed. |
| R-15 focused API tests | MCP server-ID validation regression passed; formatting and locked Clippy passed. Database-backed user-journey/auth tests remain environment-dependent. |
| R-16 suite inventory and gates | Library target contains 392 tests; 370 passed and 22 were blocked by unavailable PostgreSQL/Redis, local socket permissions, or DNS/public network. Deterministic integration targets passed: detectors 19/19, golden 3/3, docs links 1/1. |
| R-09 completion gate | No live GitHub or PostgreSQL contract test claimed; all available focused Rust checks passed. |
| R-10 focused modules | Passed: parser/dep-parser (19), gates/policy (8), confidence (8), baseline (3), and offline/LLM/registry (7) tests. |

No Clippy, full test suite, benchmark, dependency audit/tree, or database/Redis integration
command has been run. No such check is claimed as passing.

## Open risks recorded for future verification

1. Rust semantic tooling could not initialize during discovery. The map was assembled with
   Cargo metadata and targeted text/source inspection. Retry only if it becomes useful to
   the selected component; do not treat this as application failure evidence.

## Pending work

1. Start R-17: dependency and supply-chain governance.
2. Retain F-R02-001 as a follow-up design item; do not implement arbitrary redirects
   without a compatibility requirement and redirect-specific tests.
3. Update `FINDINGS.md`, `TEST_AUDIT.md`, and this file after the component batch.
