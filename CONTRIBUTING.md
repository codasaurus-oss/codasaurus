# Contributing to Codasaurus

Thank you for your interest in improving Codasaurus. This document explains how to set up a development environment, what we expect in pull requests, and how contributions are licensed.

> **Security issues:** do not open a public issue or PR. Follow [SECURITY.md](SECURITY.md).

## Code of conduct

Be respectful and constructive. Harassment, spam, or bad-faith contributions will be rejected. Maintainers may close issues or PRs that ignore this bar.

## Ways to contribute

| Kind | How |
| ---- | --- |
| Bug report | [GitHub Issues](https://github.com/codasaurus-oss/codasaurus/issues) with steps to reproduce, version (`codasaurus version`), and environment |
| Docs fix | PR against `main` (Settings paths, env vars, and schema versions drift easily—please keep them accurate) |
| Feature | Open an issue first for anything larger than a small UX or API tweak |
| Fix / patch | Focused PR with tests or a clear manual test plan |

## Development environment

### Prerequisites

| Tool | Version |
| ---- | ------- |
| Rust | **1.90+** (pinned in [`rust-toolchain.toml`](rust-toolchain.toml)) |
| Node.js | **20+** (dashboard) |
| PostgreSQL | **16+** (local or Docker) |
| Docker / Compose | Optional, recommended for Postgres |

### Clone and run (API + dashboard from the binary)

```bash
git clone https://github.com/codasaurus-oss/codasaurus.git
cd codasaurus
cp .env.example .env          # optional; Compose defaults work locally
docker compose up -d postgres
export DATABASE_URL="postgres://codasaurus:codasaurus@127.0.0.1:5432/codasaurus"
cargo run -- serve --port 3000
```

Open [http://localhost:3000](http://localhost:3000) and complete the onboarding wizard if this is a fresh database.

### Dashboard

The dashboard lives in [codasaurus-frontend](https://github.com/codasaurus-oss/codasaurus-frontend). Clone it next to this repository and run:

```bash
git clone https://github.com/codasaurus-oss/codasaurus-frontend.git
cd codasaurus-frontend
npm ci
CODASAURUS_API=http://127.0.0.1:3000 npm run dev
```

The Docker image builds that repository and serves the output; a plain `cargo build` produces an API-only binary.

### Useful commands

```bash
# Format + lint (must pass CI)
cargo fmt --check
cargo clippy -- -D warnings

# Tests
CODASAURUS_SKIP_FRONTEND_BUILD=1 cargo test

# Release binary
cargo build --release --locked
```

CI (`.github/workflows/ci.yml`) runs fmt, clippy, tests (with Postgres), `cargo deny`, actionlint, and a release-style build.

### LLM gateway (LiteLLM)

`docker compose up -d` starts the `litellm` sidecar alongside Postgres and Redis. Provider, model and fallback selection are entirely LiteLLM's: define models, provider credentials and failovers in `litellm.config.yaml` (or the LiteLLM admin UI / database), then restart only the sidecar (`docker compose restart litellm`) to pick up a config-file change. Codasaurus keeps one gateway endpoint and one credential, both supplied by compose env, and sends only the alias names you give it (`CODASAURUS_MODEL`, `CODASAURUS_MODEL_CHEAP`, `CODASAURUS_EMBEDDING_MODEL`).

`LITELLM_SALT_KEY` cannot be rotated in place — rotating it invalidates provider keys LiteLLM has stored. That only matters once LiteLLM gets a database; this compose setup has none.

## Pull request guidelines

### Before you open a PR

1. Rebase or merge `main` so CI starts from a current tip.
2. Run the checks above locally.
3. Update docs when you change user-facing behavior, env vars, Settings IA, or schema.
4. Add or adjust tests for non-trivial Rust logic when practical.

### PR shape

- **Small and focused.** One concern per PR when possible.
- **Match existing style.** Prefer the patterns already in `src/`; avoid drive-by refactors.
- **Describe the why.** Summary + test plan in the PR body (what you ran, what to click in the dashboard).
- **No secrets.** Never commit `.env`, private keys, API tokens, or research dumps.

### Commit messages

Prefer short, imperative subjects in the style already used on `main`:

```text
fix: pin Settings nav on scroll and reshape Review UX.
docs: prepare 0.1.0 release notes and fix doc/UI drifts.
```

Optional body for non-obvious context. [Conventional Commits](https://www.conventionalcommits.org/) prefixes (`feat:`, `fix:`, `docs:`, `perf:`, `chore:`) are welcome but not required.

### Changelog

Releases and changelog entries are generated from Conventional Commit messages by release-please, so write the commit subject accordingly and do not edit a changelog by hand.

## Project layout (orientation)

| Repository | Role |
| ---- | ---- |
| `codasaurus` (this repo) | Rust binary: webhooks, queue, detectors, LLM, API, deployment |
| [codasaurus-frontend](https://github.com/codasaurus-oss/codasaurus-frontend) | Svelte 5 dashboard |
| [docs](https://github.com/codasaurus-oss/docs) | Documentation site |

## License

Codasaurus is licensed under **[AGPL-3.0-or-later](LICENSE)** with additional attribution requirements under AGPL §7 (keep credit to the project and author).

By submitting a contribution, you agree that your changes are licensed under the same terms, and that you have the right to submit them under that license.

## Questions

- Product / docs: open a GitHub Discussion or Issue
- Security: [SECURITY.md](SECURITY.md) only
