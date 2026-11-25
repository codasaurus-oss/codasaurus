<p align="center">
  <img src="assets/logo.svg" alt="Codasaurus" width="96">
</p>

<h1 align="center">Codasaurus</h1>

<p align="center">
  <b>Self-hosted GitHub App that reviews PRs like a senior who actually reads the diff.</b>
</p>

<p align="center">
  Tier-1 detectors first · optional BYOK LLM · zero seat tax · run for $0<br>
  <code>codasaurus</code> on any PR · React dashboard · PostgreSQL only
</p>

<p align="center">
  <a href="https://github.com/codasaurus-oss/codasaurus/releases"><img src="https://img.shields.io/badge/release-v0.1.0-111827" alt="v0.1.0"></a>
  <a href="https://github.com/codasaurus-oss/codasaurus/actions/workflows/ci.yml"><img src="https://github.com/codasaurus-oss/codasaurus/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-1.90+-dea584?logo=rust&logoColor=white" alt="Rust"></a>
  <a href="https://codasaurus-oss.github.io/database/"><img src="https://img.shields.io/badge/db-PostgreSQL-4169E1?logo=postgresql&logoColor=white" alt="PostgreSQL"></a>
  <a href="https://codasaurus-oss.github.io/run-for-free/"><img src="https://img.shields.io/badge/host-run%20for%20%240-2ea44f" alt="Run for free"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0--or--later-blue" alt="License"></a>
</p>

<p align="center">
  <a href="https://codasaurus-oss.github.io/setup-onboarding/"><img src="https://img.shields.io/badge/docs-onboarding-0ea5e9" alt="Onboarding"></a>
  <a href="https://codasaurus-oss.github.io/commands/"><img src="https://img.shields.io/badge/docs-codasaurus%20commands-8b5cf6" alt="Commands"></a>
  <a href="https://codasaurus-oss.github.io/configuration/"><img src="https://img.shields.io/badge/docs-config-64748b" alt="Config"></a>
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

One clone builds the entire stack. The dashboard is a git submodule at `codasaurus-frontend/`, linked to the [frontend repository](https://github.com/codasaurus-oss/codasaurus-frontend), so Docker builds from the pinned source without fetching a separate repo at build time:

```bash
git clone --recurse-submodules https://github.com/codasaurus-oss/codasaurus.git
cd codasaurus
cp .env.example .env
docker compose up -d
```

If you already cloned without the flag: `git submodule update --init --recursive`.

Upgrading an older stack: PostgreSQL 19 cannot start on a 16.x data volume, and its volume mount moved to `/var/lib/postgresql`. Dump and restore your data into a fresh volume before switching images.

Open [http://localhost:3000](http://localhost:3000) and finish the three-step wizard:

1. **Inference** is optional. Skip it for Tier-1 checks, or choose a provider and models. The API key and model list are entered in the wizard, not in config files.
2. **GitHub App** registers the app that receives pull-request webhooks. Use a public HTTPS URL for callbacks when running away from localhost.
3. **Admin** creates the first owner. The setup secret is the `CODASAURUS_BOOTSTRAP_SECRET` value you set on the server. Generate one with `openssl rand -hex 32` and add it to `.env` before the first run.

After setup, trigger a review by commenting `codasaurus review` on a pull request. You do not need a separate frontend checkout; Compose builds `codasaurus-frontend/` into the backend image.

## Dashboard source and sync

`codasaurus-frontend/` is a git submodule pointing at the [codasaurus-frontend](https://github.com/codasaurus-oss/codasaurus-frontend) repository. GitHub shows the directory as a link to that repository. UI work happens upstream; this repo pins the commit that ships.

```bash
# Advance the pinned dashboard to the latest upstream main
git submodule update --remote codasaurus-frontend
git add codasaurus-frontend && git commit -m "chore: update dashboard submodule"
```

Backend CI checks out the submodule recursively and runs the dashboard lint, unit tests, and build from `codasaurus-frontend/`.

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
| [codasaurus](https://github.com/codasaurus-oss/codasaurus) | Review engine, HTTP API, webhook handler, deployment, and dashboard submodule at `codasaurus-frontend/` |
| [codasaurus-frontend](https://github.com/codasaurus-oss/codasaurus-frontend) | Upstream React + shadcn dashboard source, pinned here as a submodule |
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

Codasaurus talks to a [LiteLLM](https://docs.litellm.ai/docs/proxy/quick_start) gateway that fronts every provider. Docker Compose starts the sidecar in database mode. Configure providers, models, provider credentials and the daily LLM budget in Codasaurus onboarding or **Settings → LLM**; Codasaurus registers the model through LiteLLM's management API and issues a model-scoped virtual key for inference. `litellm.config.yaml` contains gateway runtime settings only, not provider or model definitions.

```bash
export LITELLM_MASTER_KEY="sk-..."          # sidecar management only
export LITELLM_DATABASE_URL="postgresql://..." # dedicated LiteLLM database
export LITELLM_SALT_KEY="..."              # keep stable; rotating invalidates stored provider keys
# Compose sets CODASAURUS_BASE_URL=http://litellm:4000/v1.
# CODASAURUS_API_KEY is an optional legacy/direct-endpoint inference credential;
# dashboard-managed setup stores a virtual key in the Codasaurus database.
```

A positive `llm_daily_budget_usd` is enforced by LiteLLM on the virtual key with a one-day reset window. Empty or `0` means unlimited. Codasaurus spend figures are telemetry, not billing truth. Any OpenAI-compatible `/v1` endpoint still works for inference, but managed models and gateway-enforced budgets require the LiteLLM sidecar and its management database.

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
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked              # needs DATABASE_URL, CODASAURUS_TEST_DATABASE_URL, REDIS_URL
cargo build --release --locked
```

Dashboard checks run from `codasaurus-frontend/`: `npm ci && npm run lint && npm test && npm run build`.

Security reports: [private vulnerability reporting](https://github.com/codasaurus-oss/codasaurus/security/advisories/new) or `iam@thelohit.in`. Release history: [GitHub Releases](https://github.com/codasaurus-oss/codasaurus/releases).

---

[AGPL-3.0-or-later](LICENSE) · free forever · keep credit · Copyright (C) 2026 [Lohit Kolluri](https://github.com/lohitkolluri)
