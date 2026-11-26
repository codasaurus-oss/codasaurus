# Test Audit

This file begins with R-01. It will be extended only for tests relevant to the component
being audited; no test simplification is authorized without equivalent behavioral coverage.

| Test file/group | Production component | Purpose and coverage | Setup complexity | Classification | Recommendation and validation |
| --- | --- | --- | --- | --- | --- |
| `build.rs::incomplete_frontend_checkout_has_no_manifest` | R-01 build script | Verifies missing frontend checkouts are handled without a false manifest path. | None | KEEP | Fixture uses the canonical path; passed via direct `rustc --test build.rs` execution (build-script tests are not discovered by normal `cargo test`). |
| `src/bot/related_prs.rs::source_paths_outrank_meta` and `select_skips_meta_when_source_available` | R-01 dashboard source-path contract | Ensures dashboard source paths are treated as meaningful source changes, not metadata. | None | IMPROVE | Fixtures migrated to the canonical React path; both focused tests passed. |
| Docker frontend stage | R-01 artifact contract | Verifies the multi-stage frontend artifact is generated under the canonical path consumed by later stages. | Docker daemon | KEEP | `docker build --target frontend` passed; `test -f /app/codasaurus-frontend/dist/index.html` passed inside the image. |
