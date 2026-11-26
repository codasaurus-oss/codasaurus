# Test Audit

This file begins with R-01. It will be extended only for tests relevant to the component
being audited; no test simplification is authorized without equivalent behavioral coverage.

## R-02 additions

| Test file/group | Production component | Purpose and coverage | Setup complexity | Classification | Recommendation and validation |
| --- | --- | --- | --- | --- | --- |
| `src/ssrf.rs` unit tests | `ssrf.rs` | Scheme, loopback, metadata, private-IP, and DNS resolution policy. | Public DNS for one test | KEEP | All 10 passed with DNS access; do not make the public-host assertion dependent on sandbox DNS. |
| `src/oidc.rs` and `api/auth.rs` OIDC tests | OIDC/config secret boundary | Redaction and token selection/error handling. | None | KEEP | All four focused tests passed. |
| `src/github_jwt.rs` tests | GitHub credential boundary | PEM decoding, placeholder rejection, preference, and JWT signing. | None | KEEP | All five focused tests passed. |
| Outbound redirect policy | LLM/Jira request clients | Ensures redirect behavior cannot bypass endpoint validation. | Local HTTP redirect test server | ADD | Add deterministic safe/unsafe redirect tests once the integration contract is chosen. |

| Test file/group | Production component | Purpose and coverage | Setup complexity | Classification | Recommendation and validation |
| --- | --- | --- | --- | --- | --- |
| `build.rs::incomplete_frontend_checkout_has_no_manifest` | R-01 build script | Verifies missing frontend checkouts are handled without a false manifest path. | None | KEEP | Fixture uses the canonical path; passed via direct `rustc --test build.rs` execution (build-script tests are not discovered by normal `cargo test`). |
| `src/bot/related_prs.rs::source_paths_outrank_meta` and `select_skips_meta_when_source_available` | R-01 dashboard source-path contract | Ensures dashboard source paths are treated as meaningful source changes, not metadata. | None | IMPROVE | Fixtures migrated to the canonical React path; both focused tests passed. |
| Docker frontend stage | R-01 artifact contract | Verifies the multi-stage frontend artifact is generated under the canonical path consumed by later stages. | Docker daemon | KEEP | `docker build --target frontend` passed; `test -f /app/codasaurus-frontend/dist/index.html` passed inside the image. |

## R-03 additions

| Test file/group | Production component | Purpose and coverage | Setup complexity | Classification | Recommendation and validation |
| --- | --- | --- | --- | --- | --- |
| `src/db/migrations.rs::every_numbered_migration_records_its_version` | Embedded migrations | Guards the invariant that every numbered migration (v8–v29) writes its completion record. | None | ADD | Passed as a focused library test; database smoke validation remains pending local PostgreSQL. |

## R-04 additions

| Test file/group | Production component | Purpose and coverage | Setup complexity | Classification | Recommendation and validation |
| --- | --- | --- | --- | --- | --- |
| OIDC auth integration path | `db/users.rs::upsert_oidc_user` | Concurrent first-login provisioning must return one stored user without a unique-key race. | PostgreSQL and concurrent requests | ADD | Add a PostgreSQL-backed two-task regression test; current source fix is validated by compile/Clippy only until the service is available. |

## R-05 and R-06 review

The existing Redis/API and server-operation tests were retained. Focused Rust-library coverage
passed for the server module (`cargo test --locked --lib serve::`); Redis/PostgreSQL-backed
integration coverage remains environment-dependent and was not promoted to a passing claim.

## R-07 review

The webhook verification, author-ACL, command parsing, reaction, thread, and event-policy tests
were retained. `cargo test --locked --lib bot::` passed 120 tests; two database-backed tests
failed during PostgreSQL connection setup in the restricted environment, before exercising their
assertions. No test was weakened or removed.

## R-08 review

Queue and worker tests were retained. The notification integration test requires PostgreSQL and
could not reach its assertion in the restricted environment; claim, lease, retry, and dead-letter
behavior therefore remains source-reviewed but not database-smoke-validated here.

## R-09 review

GitHub review helper tests were retained and passed. The two source fixes cover non-success
installation-sync responses and review-history pagination; live GitHub contract testing remains
outside the local environment.

R-09 persistence ordering was validated through compilation, focused review tests, formatting,
and locked Clippy; PostgreSQL-backed failure/retry behavior remains an integration-test follow-up.

## R-11 review

Detector integration (19/19) and golden detector tests (3/3) passed. Detector unit coverage had
one environment-dependent failure in `dependency_confusion::flags_scoped_dep_that_exists_publicly`
because the public npm lookup cannot run in the restricted network; no test was weakened or removed.

## R-12 review

LLM config, budget, cost, prompt, schema, offline, and response tests passed. Gateway tests that
bind local HTTP servers failed during setup with sandbox `Operation not permitted`; no test was
weakened or removed.

## R-10 review

Focused module coverage passed: parser/dependency parsing (19), gates/policy (8), confidence (8),
baseline (3), and offline/LLM/registry invariants (7). No tests were weakened or removed.

## R-13 review

Learning scope/mine tests (8), index/extraction tests (13), and the embedding batch-limit
regression passed. PostgreSQL/pgvector persistence smoke remains environment-dependent.

## R-14 review

Registry unit coverage was retained and extended with transient-status retry classification.
OSV batch parsing, unique detail hydration, fallback vulnerability records, URL encoding,
metadata extraction, cache eviction, and offline cache-miss behavior remain covered. Tests that
bind local HTTP servers may fail during setup under the restricted sandbox; no test was weakened.

## R-15 review

The protected router and each dashboard handler's RBAC checks were reviewed together with settings
masking, setup secret handling, GitHub credential usage, audit pagination, and MCP SSRF, redirect,
response-size, tool-filtering, and cache boundaries. Added a unit regression for the MCP dynamic
server-ID contract; database-backed authorization/user-journey coverage remains environment-dependent.

## R-16 suite architecture review

| Target or artifact | Classification | Evidence / disposition |
| --- | --- | --- |
| `cargo test --locked --lib` | Deterministic plus environment-sensitive unit tests | 370/392 passed. The 22 failures were isolated to PostgreSQL/Redis setup, local HTTP socket binding, public DNS/network access, or the network-backed npm lookup; no assertion was weakened. |
| `tests/detectors_integration.rs` | Deterministic integration | 19/19 passed. |
| `tests/golden_detectors.rs` and `tests/fixtures/golden/` | Byte-stable golden fixtures | 3/3 passed; fixture inputs/expectations remain tracked and readable. |
| `tests/docs_links.rs` | Repository/documentation integrity | 1/1 passed. |
| `tests/production_hardening.rs`, `full_backend.rs`, `postgres_smoke.rs`, `redis_state.rs`, `user_journey_qa.rs` | Service-backed integration | Retained as real Postgres/Redis gates; local services were unavailable, so no false pass or skip was recorded. |
| Local HTTP-server tests in gateway, MCP, setup, and registry modules | Transport contract tests | Retained; sandbox denied loopback socket binding. |
| Public DNS/npm tests | External-network contract tests | Retained; restricted DNS/network prevented execution. |
| Build script and doctest inventory | Build/documentation coverage | Build-script unit test and rustdoc examples were inventoried; no ignored or `should_panic` tests were found requiring disposition. |

R-16 made no production-code changes. The test suite keeps deterministic tests runnable offline
while preserving service-backed tests as mandatory CI/environment gates.

## R-17 supply-chain review

The manifest uses crates.io sources only; the committed lockfile is required by CI and locked
build/test commands. Feature inspection found no unused direct production dependency that can be
removed safely without changing behavior. Duplicate versions are transitive and arise from
independent crypto/runtime constraints; no unsafe broad update was attempted. `deny.toml` covers
allowed licenses, unknown registries/git sources, and duplicate-version warnings. Cargo package
listing completed successfully. `cargo audit` passed with the three documented exceptions and
reported only the accepted unmaintained-`chrono` warning. `cargo-deny` was not installed locally;
the authoritative CI compliance job remains the execution gate.
