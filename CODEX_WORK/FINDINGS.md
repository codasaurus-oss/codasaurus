# Findings

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
