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
