# Session Handoff

## Current state

R-01 is complete. It migrated the Rust-facing dashboard artifact contract from the removed
`svelte-dashboard` directory to `codasaurus-frontend`: `build.rs`, static serving,
Docker artifact stages, and related-PR path fixtures changed. No frontend source, Cargo
manifest, lockfile, or application behavior beyond that path contract changed.

## Scope

- In scope: the root Rust package, its tests, Cargo/dependency configuration, Rust build
  script, Rust runtime/deployment contract, and CI gates that validate Rust.
- Out of scope: auditing or editing `codasaurus-frontend/`. It is a pinned submodule and
  only its artifact handoff with Rust is in scope.

## Repository facts

- Branch/starting commit: `main` / `92b7e09`.
- Rust package: `codasaurus` 0.1.0, edition 2021, `rust-version = 1.90`.
- Package shape: one library and one CLI binary; no Cargo workspace.
- Durable storage: PostgreSQL; Redis is used for state/rate limiting; queue claims use
  `FOR UPDATE SKIP LOCKED`.
- Optional inference: LiteLLM, with an offline-mode no-network invariant.
- Full integration tests need a dedicated PostgreSQL database and Redis; tests clear
  `review_jobs`.

## Important files

- Architecture: `CODEX_WORK/REPOSITORY_MAP.md`
- Component order/status: `CODEX_WORK/WORK_BREAKDOWN.md`
- Current progress/evidence: `CODEX_WORK/PROGRESS.md`
- Recorded approach: `CODEX_WORK/DECISIONS.md`
- Repository guidance: `AGENTS.md`
- CI truth: `.github/workflows/ci.yml`

## Next safe action

Continue R-08: inspect durable queue claims, leases, retries, worker concurrency, cancellation,
and shutdown behavior. Preserve existing external behavior; make a source change only for a
confirmed defect and bracket it with a focused regression test.

## Tooling note

The Serena Rust language server terminated while initializing. Do not rely on Serena
symbol tools in the next session until it is demonstrably available. Targeted `rg`,
`find`, Cargo metadata, and bounded source reads remain sufficient for the initial R-01
baseline. No command was run to alter toolchains, dependencies, databases, or containers.
