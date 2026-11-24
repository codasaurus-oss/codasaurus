# LiteLLM virtual-key budget enforcement

**Verified:** 2026-11-24
**Pinned deployment:** `ghcr.io/berriai/litellm:v1.110.2`

## Decision

LiteLLM is the authority for LLM spend limits. Codasaurus stores the operator's
preference and configures the model-scoped virtual key through LiteLLM's
management API. The application does not pre-block requests using its local
spend estimate.

## Verified behavior

- `/key/generate` accepts `max_budget` and `budget_duration`. The pinned source
  maps them to the key's `key_max_budget` and `key_budget_duration` columns.
  Codasaurus sends `budget_duration: "1d"` for a positive daily budget.
- Request authorization calls `virtual_key_max_budget_check`. It reads current
  spend through `get_current_spend`, using the Redis-first cross-pod counter
  `key_counter(token)` with the cached key spend as fallback. When the configured
  limit is finite and `spend >= max_budget`, it raises
  `litellm.BudgetExceededError`. A key can opt into throttling instead of a hard
  block; Codasaurus does not enable that option.
- `/key/update` is a merge patch. Omitted fields retain their stored values;
  explicit `null` clears key columns. `prepare_key_update_data` also clears
  `budget_reset_at` when `budget_duration` is explicitly `null`. Codasaurus sends
  both `max_budget: null` and `budget_duration: null` when the budget is cleared.
- Empty or zero means unlimited in Codasaurus. Key creation omits the budget
  fields in that case; clearing an existing key sends explicit nulls.
- Management calls use `LITELLM_MASTER_KEY`. Inference uses the virtual key
  returned by `/key/generate`, scoped to the configured model alias.
- `/key/info` exposes `spend`, `max_budget`, `budget_duration`,
  `budget_reset_at`, and `soft_budget` for operational inspection.

## Operational implications

- LiteLLM needs its Postgres database for durable key and spend state. Compose
  uses a dedicated `litellm` database on the same Postgres server.
- The request-time check is a spend threshold, not a provider invoice guarantee.
  Codasaurus' `agent_events` and process metrics remain telemetry only.
- The daily reset is represented by `budget_duration: "1d"` and
  `budget_reset_at`. The scheduled reset worker and dedicated spend-report
  endpoints were not independently verified in this pass.
- PyPI reported `1.110.3` as the current release on 2026-11-24. Compose remains
  pinned to `v1.110.2`; upgrade only after repeating the key lifecycle and
  enforcement checks.

## Primary sources

- Request-time budget check, `v1.110.2`:
  https://raw.githubusercontent.com/BerriAI/litellm/v1.110.2/litellm/proxy/auth/auth_checks.py
  (`virtual_key_max_budget_check`)
- Key generation and update, `v1.110.2`:
  https://raw.githubusercontent.com/BerriAI/litellm/v1.110.2/litellm/proxy/management_endpoints/key_management_endpoints.py
  (`_common_key_generation_helper`, `prepare_key_update_data`, `/key/update`)
- Virtual keys and spend tracking:
  https://docs.litellm.ai/docs/proxy/virtual_keys
- Release metadata:
  https://pypi.org/pypi/litellm/json
