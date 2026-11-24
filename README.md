<p align="center">
  <img src="assets/logo.svg" alt="Codasaurus" width="96">
</p>

<h1 align="center">Codasaurus</h1>

<p align="center">
  <b>Self-hosted GitHub App that reviews PRs like a senior who actually reads the diff.</b>
</p>

<p align="center">
  Tier-1 detectors first · optional BYOK LLM · zero seat tax · run for $0<br>
  <code>codasaurus</code> on any PR · Svelte dashboard · PostgreSQL only
</p>

<p align="center">
  <a href="https://github.com/codasaurus-oss/codasaurus/releases"><img src="https://img.shields.io/badge/release-v0.1.0-111827" alt="v0.1.0"></a>
  <a href="https://github.com/codasaurus-oss/codasaurus/actions/workflows/ci.yml"><img src="https://github.com/codasaurus-oss/codasaurus/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-1.90+-dea584?logo=rust&logoColor=white" alt="Rust"></a>
  <a href="https://codasaurus-oss.github.io/docs/database.html"><img src="https://img.shields.io/badge/db-PostgreSQL-4169E1?logo=postgresql&logoColor=white" alt="PostgreSQL"></a>
  <a href="https://codasaurus-oss.github.io/docs/run-for-free.html"><img src="https://img.shields.io/badge/host-run%20for%20%240-2ea44f" alt="Run for free"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0--or--later-blue" alt="License"></a>
</p>

<p align="center">
  <a href="https://codasaurus-oss.github.io/docs/setup-onboarding.html"><img src="https://img.shields.io/badge/docs-onboarding-0ea5e9" alt="Onboarding"></a>
  <a href="https://codasaurus-oss.github.io/docs/commands.html"><img src="https://img.shields.io/badge/docs-codasaurus%20commands-8b5cf6" alt="Commands"></a>
  <a href="https://codasaurus-oss.github.io/docs/configuration.html"><img src="https://img.shields.io/badge/docs-config-64748b" alt="Config"></a>
  <a href="https://github.com/codasaurus-oss/codasaurus/releases"><img src="https://img.shields.io/badge/changelog-releases-orange" alt="Changelog"></a>
</p>

---

Agents ship volume. Models invent packages, paste last year's APIs, and leave secrets in the hunk. SaaS reviewers charge per seat and still miss the boring breaks.

**Codasaurus** is one Rust binary you run yourself. On every PR it proves what it can (registry HEAD, manifests, secrets, OSV, IaC), then optionally asks _your_ LLM. Never a silent cloud fallback.

| It catches               | How                           |
| ------------------------ | ----------------------------- |
| Hallucinated imports     | npm / PyPI / crates.io / Go HEAD |
| Phantom deps             | Import vs manifest            |
| Lockfile drift           | Manifest vs lockfile          |
| License drift            | Copyleft-style dep licenses |
| Secrets and vulns        | Pattern + OSV                 |
| IaC footguns             | Open CIDR, privileged pods    |
| Agent-shaped PRs         | Tier-1 first, LLM nits capped |
| Blast radius / dep delta | Walkthrough + `impact`       |

Fail-closed offline mode. Finding provenance. Learning from dismissals. Air-gap honest.

---

## Quick start

```bash
git clone https://github.com/codasaurus-oss/codasaurus.git
cd codasaurus
cp .env.example .env   # optional
docker compose up
```

Open [http://localhost:3000](http://localhost:3000), finish the [onboarding wizard](https://codasaurus-oss.github.io/setup-onboarding/) (~5 min), install the GitHub App.

Want **$0 forever** (no trials)? [Run on always-free infra](https://codasaurus-oss.github.io/run-for-free/): Render free web + Aiven Free Postgres (or Neon). LLM optional / off.

```bash
# From source (Postgres must be reachable)
cargo build --release
export DATABASE_URL="postgres://codasaurus:codasaurus@127.0.0.1:5432/codasaurus"
./target/release/codasaurus serve --port 3000
```

Prebuilt Linux binary: [GitHub Releases](https://github.com/codasaurus-oss/codasaurus/releases) (`codasaurus-v*-x86_64-unknown-linux-gnu.tar.gz`).

```text
codasaurus serve | health | version | reset-password
codasaurus queue replay --id <job-id>
codasaurus queue schedule --repo <owner/repo> --pr 42 --sha <sha> --at 2026-11-25T10:00:00Z [--priority 100]
```

---

## Testing note

`cargo test` uses `DATABASE_URL`/`CODASAURUS_TEST_DATABASE_URL` and clears
`review_jobs`. Stop the running app or use a separate test database before
running the suite, otherwise live workers claim the test jobs.

## Docs

| Guide                                                   | Topic                              |
| ------------------------------------------------------- | ---------------------------------- |
| [Onboarding](https://codasaurus-oss.github.io/setup-onboarding/)                  | First-run wizard                   |
| [GitHub App](https://codasaurus-oss.github.io/setup-github-app/)                  | Permissions, manifest, manual keys |
| [Run for free](https://codasaurus-oss.github.io/run-for-free/)                    | Always-free host + Postgres        |
| [Database](https://codasaurus-oss.github.io/database/)                            | PostgreSQL, pool, schema           |
| [Configuration](https://codasaurus-oss.github.io/configuration/)                  | Env vars, TOML, offline, OIDC      |
| [`.codasaurus.toml` schema](https://codasaurus-oss.github.io/codasaurus-toml/)    | In-repo config reference           |
| [Commands](https://codasaurus-oss.github.io/commands/)                            | `codasaurus` on PRs               |
| [Backup and restore](https://codasaurus-oss.github.io/operations-backup-restore/) | Postgres backup, HA, health        |

Full index: [documentation site](https://codasaurus-oss.github.io/).

---

## On a PR

```text
codasaurus review      codasaurus describe     codasaurus improve
codasaurus security    codasaurus impact       codasaurus similar
codasaurus ask …       codasaurus fix [fp]     codasaurus ignore <fp>
codasaurus digest      codasaurus help
```

Also: `summarize`, `labels`, `changelog`, `add_docs`. React 👎 on a finding comment to dismiss. Full table in [commands](https://codasaurus-oss.github.io/commands/).

---

## Repository layout

| Repository | Contents |
| --- | --- |
| [codasaurus](https://github.com/codasaurus-oss/codasaurus) | Review engine, HTTP API, webhook handler, deployment |
| [codasaurus-frontend](https://github.com/codasaurus-oss/codasaurus-frontend) | Svelte dashboard; built `dist/` is served by the backend |
| [docs](https://github.com/codasaurus-oss/docs) | Documentation site (Astro Starlight), published at <https://codasaurus-oss.github.io/> |

## vs the field

|                      | Hosted SaaS | Typical OSS bots | **Codasaurus**                    |
| -------------------- | ----------- | ---------------- | --------------------------------- |
| Cost                 | Per seat    | OSS / commercial | **Self-host, free**               |
| Deploy               | Their cloud | Your infra       | **One Docker binary**             |
| Hallucinated imports | Rare        | Rare             | **Tier-1**                        |
| LLM                  | Bundled     | BYOK / vendor    | **BYOK, fail-closed offline**     |
| Trust                | Opaque      | Varies           | **Provenance + dismiss learning** |

---

## LLM (optional)

Codasaurus talks to a [LiteLLM](https://docs.litellm.ai/docs/proxy/quick_start) gateway that fronts every provider. Docker Compose starts the sidecar for you. Provider, model and fallback selection are entirely LiteLLM's: define the models, provider credentials and fallbacks in `litellm.config.yaml`, the LiteLLM admin UI, or its database.

```bash
export LITELLM_MASTER_KEY="sk-..."               # compose: sidecar admin key
export CODASAURUS_API_KEY="$LITELLM_MASTER_KEY"  # app → gateway credential
export CODASAURUS_MODEL="<alias defined in LiteLLM>"
# optional: CODASAURUS_MODEL_CHEAP, CODASAURUS_EMBEDDING_MODEL
```

`CODASAURUS_BASE_URL` is required: it points at the gateway and compose injects `http://litellm:4000/v1`. A key alone is not enough. Any OpenAI-compatible `/v1` endpoint still works directly — point `CODASAURUS_BASE_URL` at it and skip the sidecar. Codasaurus ships no provider or model names; it sends only the aliases configured above, so failover and routing are whatever you set up in LiteLLM.

Toggle per repo in the dashboard. `offline_mode` / `CODASAURUS_OFFLINE=1` never opens an LLM socket. Details: [configuration.md](https://codasaurus-oss.github.io/configuration/).

---

## Config sketch

```toml
[checks]
hallucinated_imports = true
phantom_deps = true
vulnerabilities = true
secrets = true
iac = true
```

Repo overlays live in the dashboard (`config_json`). OIDC: `OIDC_ISSUER`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET`, `PUBLIC_URL`.

---

## Develop

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup and PR checks.

```bash
cargo fmt --check
cargo clippy -- -D warnings
CODASAURUS_SKIP_FRONTEND_BUILD=1 cargo test
cargo build --release
```

Security reports: [SECURITY.md](SECURITY.md). Release history: [GitHub Releases](https://github.com/codasaurus-oss/codasaurus/releases).

---

[AGPL-3.0-or-later](LICENSE) · free forever · keep credit · Copyright (C) 2026 [Lohit Kolluri](https://github.com/lohitkolluri)
