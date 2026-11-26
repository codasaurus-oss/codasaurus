# Audit Progress

## Overall status

- Phase: R-06 auditing in progress.
- Rust components mapped: 17 of 17.
- Components audited: 5 of 17 (R-01 through R-05).
- Components changed: 4 of 17 (R-01, R-03, R-04, plus the R-02 accepted-risk review).
- Current component: R-07 — Webhook authentication and GitHub event dispatch.

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

No Clippy, full test suite, benchmark, dependency audit/tree, or database/Redis integration
command has been run. No such check is claimed as passing.

## Open risks recorded for future verification

1. Rust semantic tooling could not initialize during discovery. The map was assembled with
   Cargo metadata and targeted text/source inspection. Retry only if it becomes useful to
   the selected component; do not treat this as application failure evidence.

## Pending work

1. Start R-07: webhook authentication and GitHub event dispatch.
2. Retain F-R02-001 as a follow-up design item; do not implement arbitrary redirects
   without a compatibility requirement and redirect-specific tests.
3. Update `FINDINGS.md`, `TEST_AUDIT.md`, and this file after the component batch.
