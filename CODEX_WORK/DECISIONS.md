# Architecture and Audit Decisions

| ID | Decision | Rationale | Alternatives rejected | Status |
| --- | --- | --- | --- | --- |
| D-001 | Audit the repository as one Rust package with components, not as a proposed multi-crate migration. | Cargo metadata reports one cohesive application package; no current evidence of independent publication, target, or lifecycle boundaries that justifies a crate split. | Premature workspace/crate extraction. | Accepted for audit scope. |
| D-002 | Decompose by runtime ownership and data-flow boundaries. | Server/API, webhook/queue, review pipeline, persistence, detectors, LLM, and adapters have distinct contracts and test prerequisites. | Arbitrary file-count batches or alphabetical review. | Accepted. |
| D-003 | Treat PostgreSQL queue/lease correctness and webhook authentication as critical-path components. | They govern duplicate reviews, durable work and untrusted external input; errors here affect every review. | Auditing detectors first because they are numerous. | Accepted. |
| D-004 | Keep frontend source out of this audit. | The user explicitly limited scope to Rust. The Rust static-asset handoff remains relevant to build/server behavior. | Reviewing the React submodule or changing UI code. | Accepted. |
| D-005 | Do not create findings or test-audit classifications during the mapping-only phase. | The supplied workflow requires structural discovery before exhaustive component audit; premature classifications would lack component-level baseline evidence. | Speculative file-by-file findings. | Accepted. |
| D-006 | Replace the removed `svelte-dashboard` artifact contract with `codasaurus-frontend`. | The user confirmed the migration; targeted Cargo and Docker validation confirmed the canonical build output is there. | Retaining an obsolete compatibility directory. | Implemented in R-01. |
| D-007 | Preserve the existing single-package layout unless measured evidence justifies a structural change. | The project is an application with heavily interconnected modules; splitting crates has public/build/CI impact and must not be inferred from size alone. | Workspace conversion as cleanup. | Accepted. |
