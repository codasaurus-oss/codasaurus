import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react"
import { MemoryRouter, Route, Routes } from "react-router"
import { afterEach, describe, expect, it, vi } from "vitest"

import { AuthProvider } from "@/lib/auth/auth-context"
import SetupCompleteRoute from "@/routes/setup-complete"
import SetupRoute from "@/routes/setup"

const baseStatus = {
  database: true,
  llm: false,
  github: false,
  admin: false,
  complete: false,
  public_url: "https://codasaurus.example.com",
}

function llmConfig(overrides: Record<string, unknown> = {}) {
  return {
    provider: null,
    api_key: null,
    model: null,
    model_cheap: null,
    embedding_model: null,
    upstream_provider: null,
    upstream_model: null,
    upstream_model_cheap: null,
    upstream_model_embedding: null,
    models: [],
    providers: [],
    catalog: [],
    ...overrides,
  }
}

function jsonResponse(body: unknown, init: ResponseInit = {}) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
    ...init,
  })
}

function stubFetch(
  handler: (url: string, init?: RequestInit) => Response | Promise<Response>
) {
  const fetchMock = vi.fn((input: RequestInfo | URL, init?: RequestInit) =>
    Promise.resolve(handler(String(input), init))
  )
  vi.stubGlobal("fetch", fetchMock)
  return fetchMock
}

function newClient() {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } })
}

async function chooseOption(trigger: string, option: string) {
  const control = await screen.findByRole("combobox", { name: trigger })
  await waitFor(() => expect(control).toBeEnabled())
  fireEvent.click(control, { detail: 1 })
  await waitFor(() => expect(control).toHaveAttribute("aria-expanded", "true"))
  await waitFor(() =>
    expect(document.querySelector('[data-slot="popover-content"]')).not.toBeNull()
  )
  const item = await screen.findByRole("option", { name: option })
  fireEvent.click(item)
}

function renderSetup(client: QueryClient) {
  render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={["/setup"]}>
        <Routes>
          <Route path="/setup" element={<SetupRoute />} />
          <Route
            path="/setup/complete"
            element={<div>setup complete destination</div>}
          />
          <Route path="/app/dashboard" element={<div>dashboard destination</div>} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>
  )
}

function renderSetupComplete(client: QueryClient) {
  render(
    <QueryClientProvider client={client}>
      <AuthProvider>
        <MemoryRouter initialEntries={["/setup/complete"]}>
          <Routes>
            <Route path="/setup/complete" element={<SetupCompleteRoute />} />
            <Route path="/setup" element={<div>setup destination</div>} />
          </Routes>
        </MemoryRouter>
      </AuthProvider>
    </QueryClientProvider>
  )
}

// Both wizard routes mounted together so a completion path is exercised
// end to end instead of against a stub destination.
function renderSetupFlow(client: QueryClient) {
  render(
    <QueryClientProvider client={client}>
      <AuthProvider>
        <MemoryRouter initialEntries={["/setup"]}>
          <Routes>
            <Route path="/setup" element={<SetupRoute />} />
            <Route path="/setup/complete" element={<SetupCompleteRoute />} />
            <Route path="/app/dashboard" element={<div>dashboard destination</div>} />
          </Routes>
        </MemoryRouter>
      </AuthProvider>
    </QueryClientProvider>
  )
}

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

describe("setup wizard", () => {
  it("shows 1 of 3 and starts on the LLM step", async () => {
    stubFetch((url) => {
      if (url === "/api/setup/status") {
        return jsonResponse(baseStatus)
      }
      if (url === "/api/setup/llm") {
        return jsonResponse(llmConfig())
      }
      return jsonResponse({})
    })
    renderSetup(newClient())

    expect(await screen.findByText("Step 1 of 3")).toBeInTheDocument()
    expect(screen.getAllByText("Inference").length).toBeGreaterThan(0)
    expect(screen.queryByText(/LiteLLM|Tier-1|Current provider/)).toBeNull()
    expect(
      await screen.findByRole("button", { name: "Continue without AI" })
    ).toBeInTheDocument()
  })

  it("navigates back and next between completed steps", async () => {
    stubFetch((url) => {
      if (url === "/api/setup/status") {
        return jsonResponse({ ...baseStatus, llm: true, github: true })
      }
      return jsonResponse({})
    })
    renderSetup(newClient())

    expect(await screen.findByRole("button", { name: "Create owner" })).toBeInTheDocument()
    fireEvent.click(screen.getByRole("button", { name: "Back" }))
    expect(await screen.findByRole("tab", { name: "Manual" })).toBeInTheDocument()
    fireEvent.click(screen.getByRole("button", { name: "Next" }))
    expect(await screen.findByRole("button", { name: "Create owner" })).toBeInTheDocument()
  })

  it("edits the public URL from the GitHub step", async () => {
    let publicUrl = baseStatus.public_url
    const fetchMock = stubFetch((url, init) => {
      if (url === "/api/setup/status") {
        return jsonResponse({ ...baseStatus, llm: true, public_url: publicUrl })
      }
      if (url === "/api/setup/public-url" && init?.method === "POST") {
        publicUrl = JSON.parse(String(init.body)).url
        return jsonResponse({ status: "ok" })
      }
      return jsonResponse({})
    })
    renderSetup(newClient())

    fireEvent.change(await screen.findByLabelText("Public URL"), {
      target: { value: "https://qa.example.com" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Save public URL" }))

    const post = fetchMock.mock.calls.find(
      ([url, init]) => url === "/api/setup/public-url" && init?.method === "POST"
    )
    expect(JSON.parse(String(post?.[1]?.body))).toEqual({ url: "https://qa.example.com" })
  })

  it("loads provider and model dropdowns from LiteLLM", async () => {
    stubFetch((url) => {
      if (url === "/api/setup/status") {
        return jsonResponse(baseStatus)
      }
      if (url.startsWith("/api/setup/llm")) {
        return jsonResponse(
          llmConfig({
            providers: [{ id: "openai", name: "OpenAI" }],
            catalog: [
              { name: "gpt-4o", mode: "text" },
              { name: "gpt-4o-mini", mode: "text" },
              { name: "text-embedding-3-small", mode: "embedding" },
            ],
          })
        )
      }
      return jsonResponse({})
    })
    renderSetup(newClient())

    expect(
      await screen.findByRole("combobox", { name: "Provider" })
    ).toBeInTheDocument()
    expect(
      screen.getByRole("combobox", { name: "Review model" })
    ).toBeInTheDocument()
    expect(
      screen.getByRole("combobox", { name: "Fallback / cheap model" })
    ).toBeInTheDocument()
    expect(
      screen.getByRole("combobox", { name: "Embedding model" })
    ).toBeInTheDocument()
  })

  it("registers the selected provider key and model roles", async () => {
    const fetchMock = stubFetch((url, init) => {
      if (url === "/api/setup/status") {
        return jsonResponse(baseStatus)
      }
      if (url === "/api/setup/llm/discover") {
        return jsonResponse({
          models: [
            { name: "gpt-4o", mode: "text" },
            { name: "gpt-4o-mini", mode: "text" },
            { name: "text-embedding-3-small", mode: "embedding" },
          ],
        })
      }
      if (url.startsWith("/api/setup/llm") && init?.method === "POST") {
        return jsonResponse({ status: "ok", test_passed: true })
      }
      if (url.startsWith("/api/setup/llm")) {
        return jsonResponse(
          llmConfig({
            providers: [{ id: "openai", name: "OpenAI" }],
            catalog: [
              { name: "gpt-4o", mode: "text" },
              { name: "gpt-4o-mini", mode: "text" },
              { name: "text-embedding-3-small", mode: "embedding" },
            ],
          })
        )
      }
      return jsonResponse({})
    })
    renderSetup(newClient())

    await chooseOption("Provider", "OpenAI")
    fireEvent.change(await screen.findByLabelText("API key"), {
      target: { value: "provider-key" },
    })
    await chooseOption("Review model", "gpt-4o")
    await chooseOption("Fallback / cheap model", "gpt-4o-mini")
    await chooseOption("Embedding model", "text-embedding-3-small")
    fireEvent.click(screen.getByRole("button", { name: "Save and test" }))

    const post = fetchMock.mock.calls.find(
      ([url, init]) => url === "/api/setup/llm?test=true" && init?.method === "POST"
    )
    expect(JSON.parse(String(post?.[1]?.body))).toEqual({
      provider: "litellm",
      api_key: "provider-key",
      upstream_provider: "openai",
      upstream_model: "gpt-4o",
      model: "codasaurus-review",
      upstream_model_cheap: "gpt-4o-mini",
      model_cheap: "codasaurus-cheap",
      upstream_model_embedding: "text-embedding-3-small",
      embedding_model: "codasaurus-embedding",
    })
  })

  it("loads the provider's full model catalog with the API key", async () => {
    const fetchMock = stubFetch((url, init) => {
      if (url === "/api/setup/status") {
        return jsonResponse(baseStatus)
      }
      if (url === "/api/setup/llm/discover" && init?.method === "POST") {
        return jsonResponse({
          models: [
            { name: "full-provider-model", mode: "text" },
            { name: "provider-embedding", mode: "embedding" },
          ],
        })
      }
      if (url.startsWith("/api/setup/llm")) {
        return jsonResponse(
          llmConfig({
            providers: [{ id: "openrouter", name: "OpenRouter" }],
            catalog: [{ name: "limited-model", mode: "text" }],
          })
        )
      }
      return jsonResponse({})
    })
    renderSetup(newClient())

    await chooseOption("Provider", "OpenRouter")
    expect(screen.getByRole("combobox", { name: "Review model" })).toBeDisabled()
    expect(
      screen.getByRole("combobox", { name: "Fallback / cheap model" })
    ).toBeDisabled()
    expect(screen.getByRole("combobox", { name: "Embedding model" })).toBeDisabled()

    const keyInput = await screen.findByLabelText("API key")
    expect(keyInput).toHaveAttribute("type", "password")
    fireEvent.click(screen.getByRole("button", { name: "Show API key" }))
    expect(keyInput).toHaveAttribute("type", "text")
    fireEvent.click(screen.getByRole("button", { name: "Hide API key" }))
    expect(keyInput).toHaveAttribute("type", "password")
    fireEvent.change(keyInput, { target: { value: "provider-key" } })

    expect(screen.queryByRole("button", { name: "Load models" })).toBeNull()
    await chooseOption("Review model", "full-provider-model")
    expect(screen.getByRole("combobox", { name: "Review model" })).toBeEnabled()
    expect(
      screen.getByRole("combobox", { name: "Fallback / cheap model" })
    ).toBeEnabled()
    expect(screen.getByRole("combobox", { name: "Embedding model" })).toBeEnabled()
    const post = fetchMock.mock.calls.find(
      ([url, init]) => url === "/api/setup/llm/discover" && init?.method === "POST"
    )
    expect(JSON.parse(String(post?.[1]?.body))).toEqual({
      provider: "openrouter",
      api_key: "provider-key",
    })
  })

  it("shows a spinner while provider models load", async () => {
    let releaseDiscovery!: (response: Response) => void
    stubFetch((url, init) => {
      if (url === "/api/setup/status") {
        return jsonResponse(baseStatus)
      }
      if (url === "/api/setup/llm/discover" && init?.method === "POST") {
        return new Promise<Response>((resolve) => {
          releaseDiscovery = resolve
        })
      }
      if (url.startsWith("/api/setup/llm")) {
        return jsonResponse(
          llmConfig({
            providers: [{ id: "openrouter", name: "OpenRouter" }],
          })
        )
      }
      return jsonResponse({})
    })
    renderSetup(newClient())

    await chooseOption("Provider", "OpenRouter")
    fireEvent.change(await screen.findByLabelText("API key"), {
      target: { value: "provider-key" },
    })

    await waitFor(() =>
      expect(document.querySelector('[data-slot="spinner"]')).not.toBeNull()
    )
    releaseDiscovery(jsonResponse({ models: [{ name: "one-model", mode: "text" }] }))
    await waitFor(() =>
      expect(document.querySelector('[data-slot="spinner"]')).toBeNull()
    )
  })

  it("keeps a stored upstream model that is absent from the catalog", async () => {
    stubFetch((url) => {
      if (url === "/api/setup/status") {
        return jsonResponse(baseStatus)
      }
      if (url.startsWith("/api/setup/llm")) {
        return jsonResponse(
          llmConfig({
            provider: "litellm",
            api_key: "••••••••",
            model: "legacy-review",
            upstream_provider: "openai",
            upstream_model: "legacy-upstream",
            providers: [{ id: "openai", name: "OpenAI" }],
            catalog: [{ name: "gpt-4o", mode: "text" }],
          })
        )
      }
      return jsonResponse({})
    })
    renderSetup(newClient())

    await waitFor(() =>
      expect(
        screen.getByRole("combobox", { name: "Review model" })
      ).toHaveTextContent("legacy-upstream")
    )
  })

  it("requires a provider key and review model before saving", async () => {
    stubFetch((url) => {
      if (url === "/api/setup/status") {
        return jsonResponse(baseStatus)
      }
      if (url === "/api/setup/llm/discover") {
        return jsonResponse({ models: [{ name: "gpt-4o", mode: "text" }] })
      }
      if (url.startsWith("/api/setup/llm")) {
        return jsonResponse(
          llmConfig({
            providers: [{ id: "openai", name: "OpenAI" }],
            catalog: [{ name: "gpt-4o", mode: "text" }],
          })
        )
      }
      return jsonResponse({})
    })
    renderSetup(newClient())

    await chooseOption("Provider", "OpenAI")
    expect(screen.getByRole("button", { name: "Save and test" })).toBeDisabled()
    fireEvent.change(screen.getByLabelText("API key"), {
      target: { value: "provider-key" },
    })
    expect(screen.getByRole("button", { name: "Save and test" })).toBeDisabled()
    await chooseOption("Review model", "gpt-4o")
    expect(screen.getByRole("button", { name: "Save and test" })).toBeEnabled()
  })

  it("only restricts the embedding picker to embedding models", async () => {
    stubFetch((url) => {
      if (url === "/api/setup/status") {
        return jsonResponse(baseStatus)
      }
      if (url === "/api/setup/llm/discover") {
        return jsonResponse({
          models: [
            { name: "responses-model", mode: "text" },
            { name: "completion-model", mode: "text" },
            { name: "decision-model", mode: "decisions" },
            { name: "evaluation-model", mode: "evaluation" },
            { name: "image-model", mode: "image_generation" },
            { name: "embedding-model", mode: "embedding" },
          ],
        })
      }
      if (url.startsWith("/api/setup/llm")) {
        return jsonResponse(
          llmConfig({
            providers: [{ id: "openai", name: "OpenAI" }],
            catalog: [
              { name: "responses-model", mode: "text" },
              { name: "completion-model", mode: "text" },
              { name: "decision-model", mode: "decisions" },
              { name: "evaluation-model", mode: "evaluation" },
              { name: "image-model", mode: "image_generation" },
              { name: "embedding-model", mode: "embedding" },
            ],
          })
        )
      }
      return jsonResponse({})
    })
    renderSetup(newClient())

    await chooseOption("Provider", "OpenAI")
    fireEvent.change(await screen.findByLabelText("API key"), {
      target: { value: "provider-key" },
    })
    const reviewPicker = screen.getByRole("combobox", { name: "Review model" })
    await waitFor(() => expect(reviewPicker).toBeEnabled())
    fireEvent.click(reviewPicker)
    expect(
      await screen.findByRole("option", { name: "responses-model" })
    ).toBeInTheDocument()
    expect(screen.getByRole("option", { name: "completion-model" })).toBeInTheDocument()
    expect(screen.queryByRole("option", { name: "embedding-model" })).toBeNull()
    expect(screen.queryByRole("option", { name: "decision-model" })).toBeNull()
    expect(screen.queryByRole("option", { name: "evaluation-model" })).toBeNull()
    expect(screen.queryByRole("option", { name: "image-model" })).toBeNull()
    fireEvent.click(screen.getByRole("option", { name: "responses-model" }))

    const cheapPicker = screen.getByRole("combobox", {
      name: "Fallback / cheap model",
    })
    await waitFor(() => expect(cheapPicker).toBeEnabled())
    fireEvent.click(cheapPicker)
    expect(
      await screen.findByRole("option", { name: "completion-model" })
    ).toBeInTheDocument()
    expect(screen.queryByRole("option", { name: "embedding-model" })).toBeNull()
    expect(screen.queryByRole("option", { name: "decision-model" })).toBeNull()
    expect(screen.queryByRole("option", { name: "evaluation-model" })).toBeNull()
    expect(screen.queryByRole("option", { name: "image-model" })).toBeNull()
    fireEvent.click(screen.getByRole("option", { name: "completion-model" }))
    await waitFor(() =>
      expect(screen.queryByRole("option", { name: "completion-model" })).toBeNull()
    )

    const embeddingPicker = screen.getByRole("combobox", {
      name: "Embedding model",
    })
    await waitFor(() => expect(embeddingPicker).toBeEnabled())
    fireEvent.click(embeddingPicker)
    expect(
      await screen.findByRole("option", { name: "embedding-model" })
    ).toBeInTheDocument()
    expect(screen.queryByRole("option", { name: "responses-model" })).toBeNull()
    expect(screen.queryByRole("option", { name: "completion-model" })).toBeNull()
  })

  it("redirects to the dashboard when setup is complete", async () => {
    stubFetch(() =>
      jsonResponse({
        ...baseStatus,
        llm: true,
        github: true,
        admin: true,
        complete: true,
      })
    )
    renderSetup(newClient())

    expect(await screen.findByText("dashboard destination")).toBeInTheDocument()
  })

  it("completes the Inference step when AI is skipped", async () => {
    let llmConfigured = false
    const fetchMock = stubFetch((url, init) => {
      if (url === "/api/setup/status") {
        return jsonResponse({ ...baseStatus, llm: llmConfigured })
      }
      if (url.startsWith("/api/setup/llm")) {
        if (init?.method === "POST") {
          llmConfigured = true
          return jsonResponse({
            status: "ok",
            test_passed: true,
            message: "LLM configuration saved and connection verified",
          })
        }
        return jsonResponse(llmConfig())
      }
      return jsonResponse({})
    })
    renderSetup(newClient())

    fireEvent.click(
      await screen.findByRole("button", { name: "Continue without AI" })
    )

    expect(await screen.findByText("Step 2 of 3")).toBeInTheDocument()
    const post = fetchMock.mock.calls.find(
      ([url, init]) =>
        String(url).startsWith("/api/setup/llm") && init?.method === "POST"
    )
    expect(post).toBeDefined()
    expect(JSON.parse(String(post?.[1]?.body))).toEqual({ provider: "disabled" })
  })

  it("hides the install CTA when no GitHub install URL is known", async () => {
    stubFetch((url) => {
      if (url === "/api/setup/status") {
        return jsonResponse({
          ...baseStatus,
          llm: true,
          github: true,
          admin: true,
          complete: true,
        })
      }
      return jsonResponse({ authenticated: false, user: null })
    })
    renderSetupComplete(newClient())

    expect(await screen.findByText("Setup complete")).toBeInTheDocument()
    expect(
      screen.queryByRole("link", { name: "Install the GitHub App" })
    ).toBeNull()
    expect(await screen.findByRole("link", { name: "Sign in" })).toHaveAttribute(
      "href",
      "#/login"
    )
  })

  it("lands on setup complete after creating the owner", async () => {
    let adminCreated = false
    stubFetch((url) => {
      if (url === "/api/setup/status") {
        return jsonResponse({
          ...baseStatus,
          llm: true,
          github: true,
          admin: adminCreated,
          complete: adminCreated,
        })
      }
      if (url === "/api/setup/admin") {
        adminCreated = true
        return jsonResponse({ status: "ok" })
      }
      return jsonResponse({ authenticated: false, user: null })
    })
    renderSetupFlow(newClient())

    fireEvent.change(await screen.findByLabelText("Email"), {
      target: { value: "owner@example.com" },
    })
    const password = screen.getByLabelText("Password")
    expect(password).toHaveAttribute("type", "password")
    fireEvent.click(screen.getByRole("button", { name: "Show Password" }))
    expect(password).toHaveAttribute("type", "text")
    fireEvent.click(screen.getByRole("button", { name: "Hide Password" }))
    fireEvent.change(password, { target: { value: "correct-horse-battery" } })
    const setupSecret = screen.getByLabelText("Setup secret")
    expect(setupSecret).toHaveAttribute("type", "password")
    fireEvent.click(screen.getByRole("button", { name: "Show Setup secret" }))
    expect(setupSecret).toHaveAttribute("type", "text")
    fireEvent.change(setupSecret, { target: { value: "bootstrap-secret" } })
    fireEvent.click(screen.getByRole("button", { name: "Create owner" }))

    expect(await screen.findByText("Setup complete")).toBeInTheDocument()
    expect(screen.queryByText("First-run setup")).toBeNull()
  })

  it("stays on the GitHub step and shows the install link after saving", async () => {
    let saved = false
    const installUrl = "https://github.com/apps/codasaurus/installations/new"
    stubFetch((url, init) => {
      if (url === "/api/setup/status") {
        return jsonResponse({
          ...baseStatus,
          llm: true,
          github: saved,
          github_install_url: saved ? installUrl : null,
        })
      }
      if (url === "/api/setup/github" && init?.method === "POST") {
        saved = true
        return jsonResponse({ status: "ok" })
      }
      return jsonResponse({})
    })
    renderSetup(newClient())

    fireEvent.click(await screen.findByRole("tab", { name: "Manual" }))
    fireEvent.change(await screen.findByLabelText("App ID"), {
      target: { value: "12345" },
    })
    const privateKey = screen.getByLabelText("Private key")
    fireEvent.click(screen.getByRole("button", { name: "Show Private key" }))
    fireEvent.click(screen.getByRole("button", { name: "Hide Private key" }))
    fireEvent.change(privateKey, { target: { value: "-----BEGIN PRIVATE KEY-----" } })
    const webhookSecret = screen.getByLabelText("Webhook secret")
    expect(webhookSecret).toHaveAttribute("type", "password")
    fireEvent.click(screen.getByRole("button", { name: "Show Webhook secret" }))
    expect(webhookSecret).toHaveAttribute("type", "text")
    fireEvent.click(screen.getByRole("button", { name: "Hide Webhook secret" }))
    fireEvent.change(webhookSecret, { target: { value: "hook-secret" } })
    fireEvent.click(screen.getByRole("button", { name: "Save credentials" }))

    expect(
      await screen.findByRole("link", { name: "Install the app on GitHub" })
    ).toHaveAttribute("href", installUrl)
    expect(screen.getByLabelText("App ID")).toBeInTheDocument()
  })

  it("disables the manifest button when the public URL is localhost", async () => {
    stubFetch((url) => {
      if (url === "/api/setup/status") {
        return jsonResponse({
          ...baseStatus,
          llm: true,
          public_url: "http://localhost:8000",
        })
      }
      return jsonResponse({})
    })
    renderSetup(newClient())

    expect(
      await screen.findByRole("button", { name: "Create app with GitHub" })
    ).toBeDisabled()
  })

  it("redirects to setup when the wizard is still incomplete", async () => {
    stubFetch((url) => {
      if (url === "/api/setup/status") {
        return jsonResponse(baseStatus)
      }
      return jsonResponse({ authenticated: false, user: null })
    })
    renderSetupComplete(newClient())

    expect(await screen.findByText("setup destination")).toBeInTheDocument()
  })

  it("hides repository sync from a viewer", async () => {
    stubFetch((url) => {
      if (url === "/api/setup/status") {
        return jsonResponse({
          ...baseStatus,
          llm: true,
          github: true,
          admin: true,
          complete: true,
        })
      }
      return jsonResponse({
        authenticated: true,
        user: {
          email: "viewer@example.com",
          role: "viewer",
          auth_provider: "local",
          is_bootstrap: false,
        },
      })
    })
    renderSetupComplete(newClient())

    expect(
      await screen.findByRole("link", { name: "Open dashboard" })
    ).toBeInTheDocument()
    expect(
      screen.queryByRole("button", { name: "Sync repositories" })
    ).toBeNull()
  })

  it("keeps the PEM line breaks from the private key field", async () => {
    const pem =
      "-----BEGIN PRIVATE KEY-----\nMIIBVwIBADANBgkqhkiG9w0B\n-----END PRIVATE KEY-----"
    const fetchMock = stubFetch((url, init) => {
      if (url === "/api/setup/status") {
        return jsonResponse({ ...baseStatus, llm: true })
      }
      if (url === "/api/setup/github" && init?.method === "POST") {
        return jsonResponse({ status: "ok" })
      }
      return jsonResponse({})
    })
    renderSetup(newClient())

    fireEvent.click(await screen.findByRole("tab", { name: "Manual" }))
    fireEvent.change(await screen.findByLabelText("App ID"), {
      target: { value: "12345" },
    })
    expect(screen.getByLabelText("Private key").tagName).toBe("TEXTAREA")
    expect(screen.getByLabelText("Private key")).toHaveClass(
      "field-sizing-fixed",
      "overflow-y-auto"
    )
    expect(
      screen.getByRole("button", { name: "Show Private key" })
    ).toBeInTheDocument()
    fireEvent.click(screen.getByRole("button", { name: "Show Private key" }))
    fireEvent.click(screen.getByRole("button", { name: "Hide Private key" }))
    expect(
      screen.getByRole("button", { name: "Show Webhook secret" })
    ).toBeInTheDocument()
    fireEvent.change(screen.getByLabelText("Private key"), {
      target: { value: pem },
    })
    fireEvent.change(screen.getByLabelText("Webhook secret"), {
      target: { value: "webhook-secret" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Save credentials" }))

    expect(
      await screen.findByText("GitHub App credentials saved")
    ).toBeInTheDocument()
    const post = fetchMock.mock.calls.find(
      ([url, init]) =>
        String(url) === "/api/setup/github" && init?.method === "POST"
    )
    expect(post).toBeDefined()
    expect(JSON.parse(String(post?.[1]?.body)).private_key).toBe(pem)
  })

  it("sends the bootstrap secret when creating the first owner", async () => {
    const fetchMock = stubFetch((url) => {
      if (url === "/api/setup/status") {
        return jsonResponse({
          ...baseStatus,
          llm: true,
          github: true,
          admin: false,
        })
      }
      if (url === "/api/setup/admin") {
        return jsonResponse({ status: "ok" })
      }
      return jsonResponse({})
    })
    renderSetup(newClient())

    fireEvent.change(await screen.findByLabelText("Email"), {
      target: { value: "owner@example.com" },
    })
    const password = screen.getByLabelText("Password")
    expect(password).toHaveAttribute("type", "password")
    fireEvent.click(screen.getByRole("button", { name: "Show Password" }))
    expect(password).toHaveAttribute("type", "text")
    fireEvent.click(screen.getByRole("button", { name: "Hide Password" }))
    fireEvent.change(password, { target: { value: "correct-horse-battery" } })
    const setupSecret = screen.getByLabelText("Setup secret")
    expect(setupSecret).toHaveAttribute("type", "password")
    fireEvent.click(screen.getByRole("button", { name: "Show Setup secret" }))
    expect(setupSecret).toHaveAttribute("type", "text")
    fireEvent.change(setupSecret, { target: { value: "bootstrap-secret" } })
    fireEvent.click(screen.getByRole("button", { name: "Create owner" }))

    expect(
      await screen.findByText("setup complete destination")
    ).toBeInTheDocument()
    const post = fetchMock.mock.calls.find(
      ([url]) => String(url) === "/api/setup/admin"
    )
    expect(post).toBeDefined()
    expect(new Headers(post?.[1]?.headers).get("x-bootstrap-secret")).toBe(
      "bootstrap-secret"
    )
  })
})
