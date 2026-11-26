# LiteLLM integration refactor

## Decisions

- LiteLLM is the owner of provider retries, cooldowns, and fallback behavior. The Rust client sends
  each chat or embedding request once to avoid duplicate billable completions.
- The LiteLLM router uses two provider retries (`router_settings.num_retries: 2`) while its request
  timeout remains below the Rust client timeout.
- Rust disables HTTP redirects for both inference and management clients. Endpoint SSRF validation
  therefore remains meaningful and credentials cannot be forwarded after a redirect.
- Successful and error response bodies are bounded before JSON or text decoding. Embedding responses
  have a larger bound because they contain vectors; diagnostics remain small.
- Review MCP servers are discovered only to apply Codasaurus' read-only allowlist, reconciled into
  LiteLLM, and sent as `litellm_proxy/mcp/<alias>` descriptors. LiteLLM owns discovery, execution,
  retries, and result injection; Rust no longer runs an MCP tool-call loop.
- LiteLLM MCP servers are reconciled with stable `codasaurus_<id>` aliases and `allow_all_keys` so
  the existing model-scoped application key can reach only the descriptors explicitly included for
  an opted-in repository. The Rust allowlist still excludes write-looking tools.

## Verification

- `cargo fmt --check` passed.
- `cargo clippy --locked --all-targets -- -D warnings` passed.
- `cargo test --locked --lib llm::` compiled and ran; deterministic LLM tests passed. Gateway tests
  requiring a local HTTP listener were blocked by the sandbox's socket permission (`Operation not
  permitted`).
- MCP descriptor and allowlist tests passed. LiteLLM MCP reconciliation uses the documented
  `/v1/mcp/server` control-plane API and `litellm_proxy/mcp/<alias>` inference selectors.

