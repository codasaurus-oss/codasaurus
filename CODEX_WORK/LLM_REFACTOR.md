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
- Review tool calls are bounded to four model rounds, with the final round forced to return the
  structured review instead of requesting another tool.

## Verification

- `cargo fmt --check` passed.
- `cargo clippy --locked --all-targets -- -D warnings` passed.
- `cargo test --locked --lib llm::` compiled and ran; deterministic LLM tests passed. Gateway tests
  requiring a local HTTP listener were blocked by the sandbox's socket permission (`Operation not
  permitted`).

