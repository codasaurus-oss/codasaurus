import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react"
import { MemoryRouter } from "react-router"
import { toast } from "sonner"
import { afterEach, describe, expect, it, vi } from "vitest"

import { SettingField } from "@/features/settings/setting-field"
import { settingsMapSchema } from "@/lib/api/schemas"
import { AuthProvider } from "@/lib/auth/auth-context"
import SettingsRoute from "@/routes/settings"

vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn(), message: vi.fn() },
}))

const settings: Record<string, unknown> = {
  public_url: "https://bots.example.com",
  offline_mode: "false",
  offline_mode_effective: false,
  offline_mode_source: "db",
  llm_provider: "litellm",
  llm_api_key: "••••••••",
  llm_model: "gpt-5",
  queue_workers: "4",
  max_concurrent_reviews: "8",
  metrics_token: "••••••••",
  jira_api_token: "••••••••",
  github_app_id: "123",
  github_app_name: "codasaurus",
  github_app_slug: "codasaurus",
  github_private_key: "••••••••",
  github_webhook_secret: "••••••••",
  // The backend omits hidden keys from GET /api/settings; if one leaks through
  // the console still must not render it.
  database_url: "postgres://user:pass@db/app",
  llm_base_url: "http://localhost:4000",
  allow_local_llm: "true",
  openrouter_api_key: "sk-or-secret",
}

const llmConfig = {
  provider: "litellm",
  api_key: "••••••••",
  model: "codasaurus-review",
  model_cheap: "codasaurus-cheap",
  embedding_model: "codasaurus-embedding",
  upstream_provider: "openrouter",
  upstream_model: "openai/gpt-5",
  upstream_model_cheap: "openai/gpt-5-mini",
  upstream_model_embedding: "openai/text-embedding-3-small",
  models: [],
  providers: [{ id: "openrouter", name: "OpenRouter" }],
  catalog: [
    { name: "openai/gpt-5", mode: "text" },
    { name: "openai/gpt-5-mini", mode: "text" },
    { name: "openai/text-embedding-3-small", mode: "embedding" },
  ],
}

function jsonResponse(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

// Copied verbatim from ALLOWED_KEYS in codasaurus/src/api/settings.rs; the
// console must render each of these exactly once and nothing writable besides.
const ALLOWED_KEYS = [
  "llm_daily_budget_usd",
  "public_url",
  "default_severity",
  "review_strictness",
  "max_warnings",
  "max_blocking",
  "forbidden_paths",
  "request_reviewers",
  "create_check_run",
  "hallucinated_imports_enabled",
  "phantom_deps_enabled",
  "vulnerabilities_enabled",
  "secrets_enabled",
  "over_engineering_enabled",
  "boilerplate_enabled",
  "todo_leaks_enabled",
  "stale_api_enabled",
  "risky_patterns_enabled",
  "graph_enabled",
  "guidelines_enabled",
  "iac_enabled",
  "auto_labels_enabled",
  "exclude_patterns",
  "update_pr_description",
  "custom_instructions",
  "allow_auto_fix",
  "auto_approve",
  "pr_title_fix",
  "offline_mode",
  "audit_retention_days",
  "queue_workers",
  "max_concurrent_reviews",
  "hsts",
  "metrics_token",
  "review_timeout_secs",
  "max_inline_comments",
  "max_reviewer_files",
  "max_comment_bytes",
  "max_llm_diff_chars",
  "auto_improve_max_files",
  "auto_improve_max_diff",
  "insecure_cookies",
  "secure_cookies",
  "jira_base_url",
  "jira_email",
  "jira_api_token",
  "linear_api_key",
  "oidc_issuer",
  "oidc_client_id",
  "oidc_client_secret",
  "oidc_redirect_uri",
  "oidc_scopes",
  "oidc_allow_open_join",
  "oidc_allow_unverified_email",
  "oidc_allow_public_client",
]

const TAB_NAMES = ["General", "GitHub", "LLM", "OIDC", "Tickets", "Advanced"]

async function renderedSettingKeys() {
  const seen: string[] = []
  for (const name of TAB_NAMES) {
    const tab = screen.getByRole("tab", { name })
    fireEvent.click(tab)
    await waitFor(() => expect(tab).toHaveAttribute("aria-selected", "true"))
    await waitFor(() =>
      expect(
        document.querySelectorAll(
          '[data-slot="tabs-content"]:not([data-hidden])'
        )
      ).toHaveLength(1)
    )
    const panel = document.querySelector(
      '[data-slot="tabs-content"]:not([data-hidden])'
    )
    const ids = Array.from(
      panel?.querySelectorAll('[id^="setting-"]') ?? [],
      (element) => element.id.slice("setting-".length)
    )
    seen.push(...ids)
    expect(screen.queryByText("postgres://user:pass@db/app")).toBeNull()
  }
  return seen
}

function stubApi(
  options: {
    role?: string
    settings?: Record<string, unknown>
    deleteStatus?: number
  } = {}
) {
  const role = options.role ?? "owner"
  const values = options.settings ?? settings
  const fetchMock = vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = init?.method ?? "GET"
    if (url === "/api/auth/me") {
      return Promise.resolve(
        jsonResponse({
          authenticated: true,
          user: {
            email: `${role}@example.com`,
            role,
            auth_provider: "local",
            is_bootstrap: false,
          },
        })
      )
    }
    if (url.startsWith("/api/setup/llm") && method === "GET") {
      return Promise.resolve(jsonResponse(llmConfig))
    }
    if (url === "/api/setup/llm" && method === "POST") {
      return Promise.resolve(jsonResponse({ status: "ok" }))
    }
    if (url === "/api/settings" && method === "GET") {
      return Promise.resolve(jsonResponse(values))
    }
    if (url === "/api/settings/github" && method === "GET") {
      return Promise.resolve(
        jsonResponse({
          app_id: "123",
          app_name: "codasaurus",
          slug: "codasaurus",
          configured: true,
        })
      )
    }
    if (url.startsWith("/api/settings/") && method === "PUT") {
      const key = url.slice("/api/settings/".length)
      return Promise.resolve(
        jsonResponse({
          status: "ok",
          key,
          restart_required: key === "queue_workers",
        })
      )
    }
    if (url === "/api/settings/jira/test") {
      return Promise.resolve(
        jsonResponse({ status: "ok", ok: true, message: "Connected as Ada" })
      )
    }
    if (url === "/api/settings/linear/test") {
      return Promise.resolve(
        jsonResponse({ error: "Linear rejected credentials (HTTP 401)" }, 400)
      )
    }
    if (url.endsWith("/test") && method === "POST") {
      return Promise.resolve(jsonResponse({ status: "ok", ok: true }))
    }
    if (method === "DELETE") {
      if (options.deleteStatus && options.deleteStatus !== 200) {
        return Promise.resolve(
          jsonResponse(
            { error: "Insufficient permissions for this action" },
            options.deleteStatus
          )
        )
      }
      return Promise.resolve(jsonResponse({ status: "ok", message: "Cleared" }))
    }
    return Promise.resolve(
      jsonResponse({ error: `Unmatched request: ${method} ${url}` }, 500)
    )
  })
  vi.stubGlobal("fetch", fetchMock)
  return fetchMock
}

function renderSettings() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  render(
    <QueryClientProvider client={queryClient}>
      <AuthProvider>
        <MemoryRouter>
          <SettingsRoute />
        </MemoryRouter>
      </AuthProvider>
    </QueryClientProvider>
  )
  return queryClient
}

function putCalls(fetchMock: ReturnType<typeof stubApi>) {
  return fetchMock.mock.calls.filter(([, init]) => init?.method === "PUT")
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  vi.unstubAllGlobals()
})

describe("settings", () => {
  it("parses the flat map and renders grouped values", async () => {
    expect(
      settingsMapSchema.parse({
        public_url: "https://bots.example.com",
        offline_mode_effective: true,
        offline_mode_source: "env",
      })
    ).toEqual({
      public_url: "https://bots.example.com",
      offline_mode_effective: true,
      offline_mode_source: "env",
    })
    expect(() => settingsMapSchema.parse({ public_url: 1 })).toThrow()

    stubApi()
    renderSettings()

    expect(await screen.findByLabelText("Public URL")).toHaveValue(
      "https://bots.example.com"
    )
    expect(screen.getByText("Effective")).toBeInTheDocument()
    expect(screen.getByText("db")).toBeInTheDocument()
  })

  it("renders every allowed key exactly once and no hidden keys", async () => {
    stubApi()
    renderSettings()

    await screen.findByLabelText("Public URL")
    const seen = await renderedSettingKeys()

    expect([...seen].sort()).toEqual([...ALLOWED_KEYS].sort())
    expect(seen).toHaveLength(ALLOWED_KEYS.length)
    for (const key of [
      "database_url",
      "llm_base_url",
      "allow_local_llm",
      "openrouter_api_key",
      "github_private_key",
      "github_webhook_secret",
    ]) {
      expect(document.querySelector(`#setting-${key}`)).toBeNull()
      expect(screen.queryByText(key)).toBeNull()
    }
  })

  it("shows configured LLM values and never submits the masked key", async () => {
    const fetchMock = stubApi()
    renderSettings()

    await screen.findByLabelText("Public URL")
    fireEvent.click(screen.getByRole("tab", { name: "LLM" }))

    expect(
      await screen.findByRole("combobox", { name: "Provider" })
    ).toHaveTextContent("OpenRouter")
    expect(
      screen.getByRole("combobox", { name: "Review model" })
    ).toHaveTextContent("openai/gpt-5")
    expect(screen.getByLabelText("API key")).toHaveValue("")
    expect(screen.getByText("Configured")).toBeInTheDocument()

    fireEvent.click(screen.getByRole("button", { name: "Save configuration" }))

    await waitFor(() => {
      const post = fetchMock.mock.calls.find(
        ([url, init]) => url === "/api/setup/llm" && init?.method === "POST"
      )
      expect(post).toBeDefined()
      const body = JSON.parse(String(post?.[1]?.body))
      expect(body).not.toContain("•")
      expect(body.upstream_provider).toBeUndefined()
      expect(body.upstream_model).toBeUndefined()
    })
  })

  it("requires a provider key before saving changed models", async () => {
    stubApi()
    renderSettings()

    await screen.findByLabelText("Public URL")
    fireEvent.click(screen.getByRole("tab", { name: "LLM" }))

    const review = await screen.findByRole("combobox", { name: "Review model" })
    fireEvent.click(review)
    fireEvent.click(
      await screen.findByRole("option", { name: "openai/gpt-5-mini" })
    )

    expect(screen.getByRole("button", { name: "Save configuration" })).toBeDisabled()
    expect(screen.getByText(/Enter the provider API key/)).toBeInTheDocument()
  })

  it("warns instead of submitting a starred draft", async () => {
    const fetchMock = vi.fn()
    vi.stubGlobal("fetch", fetchMock)

    render(
      <SettingField
        settingKey="llm_api_key"
        label="API key"
        value="••••••••"
        onSaved={vi.fn()}
      />
    )

    fireEvent.change(screen.getByLabelText("API key"), {
      target: { value: "***" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Replace API key" }))

    expect(await screen.findByRole("status")).toHaveTextContent(
      "Masked and starred values are not submitted"
    )
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it("warns instead of submitting a masked bullet draft", async () => {
    const fetchMock = vi.fn()
    vi.stubGlobal("fetch", fetchMock)

    render(
      <SettingField
        settingKey="llm_api_key"
        label="API key"
        value="••••••••"
        onSaved={vi.fn()}
      />
    )

    fireEvent.change(screen.getByLabelText("API key"), {
      target: { value: "••••••••" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Replace API key" }))

    expect(await screen.findByRole("status")).toHaveTextContent(
      "Masked and starred values are not submitted"
    )
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it("clears only the metrics token with an empty value", async () => {
    const fetchMock = stubApi()
    renderSettings()

    await screen.findByLabelText("Public URL")
    fireEvent.click(screen.getByRole("tab", { name: "Advanced" }))

    fireEvent.click(await screen.findByRole("button", { name: "Clear Metrics token" }))

    await waitFor(() => {
      const [put] = putCalls(fetchMock)
      expect(put).toBeDefined()
      expect(String(put?.[0])).toBe("/api/settings/metrics_token")
      expect(JSON.parse(String(put?.[1]?.body))).toEqual({ value: "" })
    })
  })

  it("keeps a restart alert visible across tabs", async () => {
    const fetchMock = stubApi()
    renderSettings()

    await screen.findByLabelText("Public URL")
    fireEvent.click(screen.getByRole("tab", { name: "Advanced" }))

    const workers = await screen.findByLabelText("Queue workers")
    fireEvent.change(workers, { target: { value: "6" } })
    fireEvent.click(screen.getByRole("button", { name: "Save Queue workers" }))

    await waitFor(() => {
      const [put] = putCalls(fetchMock)
      expect(JSON.parse(String(put?.[1]?.body))).toEqual({ value: "6" })
    })
    expect(await screen.findByText("Restart required")).toBeInTheDocument()

    fireEvent.click(screen.getByRole("tab", { name: "General" }))
    fireEvent.click(screen.getByRole("tab", { name: "Advanced" }))
    expect(screen.getByText("Restart required")).toBeInTheDocument()
  })

  it("refreshes the caches that depend on saved settings", async () => {
    stubApi()
    const client = renderSettings()
    const invalidate = vi.spyOn(client, "invalidateQueries")

    const input = await screen.findByLabelText("Public URL")
    fireEvent.change(input, {
      target: { value: "https://new.example.com" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Save Public URL" }))

    await waitFor(() => {
      expect(invalidate).toHaveBeenCalledWith({ queryKey: ["settings"] })
      expect(invalidate).toHaveBeenCalledWith({ queryKey: ["repos"] })
      expect(invalidate).toHaveBeenCalledWith({
        queryKey: ["auth", "oidc-status"],
      })
      expect(invalidate).toHaveBeenCalledWith({ queryKey: ["health"] })
      expect(invalidate).toHaveBeenCalledWith({ queryKey: ["setup", "status"] })
    })
    expect(invalidate).not.toHaveBeenCalledWith({
      queryKey: ["settings", "github"],
    })
  })

  it("renders integration test success and failure inline", async () => {
    stubApi()
    renderSettings()

    await screen.findByLabelText("Public URL")
    fireEvent.click(screen.getByRole("tab", { name: "Tickets" }))

    fireEvent.click(await screen.findByRole("button", { name: "Test Jira" }))
    expect(await screen.findByText("Connected")).toBeInTheDocument()
    expect(screen.getByText("Connected as Ada")).toBeInTheDocument()

    fireEvent.click(screen.getByRole("button", { name: "Test Linear" }))
    expect(await screen.findByText("Connection failed")).toBeInTheDocument()
    expect(
      screen.getByText("Linear rejected credentials (HTTP 401)")
    ).toBeInTheDocument()
  })

  it("confirms before deleting stored config", async () => {
    const fetchMock = stubApi()
    renderSettings()

    await screen.findByLabelText("Public URL")
    fireEvent.click(screen.getByRole("tab", { name: "GitHub" }))

    fireEvent.click(
      await screen.findByRole("button", { name: "Clear GitHub config" })
    )
    expect(
      fetchMock.mock.calls.filter(([, init]) => init?.method === "DELETE")
    ).toHaveLength(0)
    expect(
      await screen.findByText("Clear GitHub App config")
    ).toBeInTheDocument()

    fireEvent.click(screen.getByRole("button", { name: "Delete" }))

    await waitFor(() => {
      const deletes = fetchMock.mock.calls.filter(
        ([, init]) => init?.method === "DELETE"
      )
      expect(deletes).toHaveLength(1)
      expect(String(deletes[0]?.[0])).toBe("/api/settings/github")
    })
    expect(toast.success).toHaveBeenCalledWith("Cleared")
  })

  it("scopes a destructive-action error to its own tab", async () => {
    stubApi({ deleteStatus: 403 })
    renderSettings()

    await screen.findByLabelText("Public URL")
    fireEvent.click(screen.getByRole("tab", { name: "GitHub" }))
    fireEvent.click(
      await screen.findByRole("button", { name: "Clear GitHub config" })
    )
    fireEvent.click(await screen.findByRole("button", { name: "Delete" }))

    expect(
      await screen.findByText("Insufficient permissions for this action")
    ).toBeInTheDocument()

    fireEvent.click(screen.getByRole("tab", { name: "Tickets" }))
    await waitFor(() =>
      expect(
        screen.queryByText("Insufficient permissions for this action")
      ).toBeNull()
    )
  })

  it("reports an unchanged save without a success toast", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        jsonResponse({ status: "ok", key: "public_url", skipped: "unchanged" })
      )
    )

    render(
      <SettingField
        settingKey="public_url"
        label="Public URL"
        value="https://old.example.com"
        onSaved={vi.fn()}
      />
    )

    fireEvent.change(screen.getByLabelText("Public URL"), {
      target: { value: "https://new.example.com" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Save Public URL" }))

    await waitFor(() =>
      expect(toast.message).toHaveBeenCalledWith("Public URL unchanged")
    )
    expect(toast.success).not.toHaveBeenCalled()
  })

  it("renders a rejected save inline", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        jsonResponse(
          { error: "queue_workers must be between 1 and 8" },
          400
        )
      )
    )
    const onSaved = vi.fn()

    render(
      <SettingField
        settingKey="queue_workers"
        label="Queue workers"
        value="4"
        onSaved={onSaved}
      />
    )

    fireEvent.change(screen.getByLabelText("Queue workers"), {
      target: { value: "9" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Save Queue workers" }))

    expect(
      await screen.findByText("queue_workers must be between 1 and 8")
    ).toBeInTheDocument()
    expect(screen.getByText("Something went wrong")).toBeInTheDocument()
    expect(onSaved).not.toHaveBeenCalled()
    expect(toast.success).not.toHaveBeenCalled()
  })

  it.each(["viewer", "maintainer"])(
    "shows the 403 state to a %s",
    async (role) => {
      const fetchMock = stubApi({ role })
      renderSettings()

      expect(
        await screen.findByText("Only workspace owners can manage settings.")
      ).toBeInTheDocument()
      expect(screen.getByText("Something went wrong")).toBeInTheDocument()
      expect(
        fetchMock.mock.calls.some(
          ([input]) => String(input) === "/api/settings"
        )
      ).toBe(false)
    }
  )
})
