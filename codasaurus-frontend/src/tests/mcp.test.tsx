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

import { AuthProvider } from "@/lib/auth/auth-context"
import McpRoute from "@/routes/mcp"

vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn(), message: vi.fn() },
}))

const servers = [
  {
    id: "context7",
    name: "Context7",
    base_url: "https://mcp.context7.com/mcp",
    docs_url: "https://context7.com",
    enabled: true,
    key_configured: true,
    is_custom: false,
  },
  {
    id: "custom-1",
    name: "Internal",
    base_url: "https://mcp.internal.example",
    docs_url: null,
    enabled: false,
    key_configured: false,
    is_custom: true,
  },
]

function jsonResponse(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

function stubApi(role = "owner") {
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
    if (url === "/api/mcp/servers" && method === "GET") {
      return Promise.resolve(jsonResponse({ servers }))
    }
    if (url === "/api/mcp/context7/test" && method === "POST") {
      return Promise.resolve(
        jsonResponse({ status: "ok", ok: true, tool_count: 3 })
      )
    }
    if (url.startsWith("/api/mcp/servers/") && method === "PUT") {
      return Promise.resolve(jsonResponse({ status: "ok" }))
    }
    if (url.startsWith("/api/mcp/servers/") && method === "DELETE") {
      return Promise.resolve(jsonResponse({ status: "ok" }))
    }
    return Promise.resolve(
      jsonResponse({ error: `Unmatched request: ${method} ${url}` }, 500)
    )
  })
  vi.stubGlobal("fetch", fetchMock)
  return fetchMock
}

function renderMcp() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  render(
    <QueryClientProvider client={queryClient}>
      <AuthProvider>
        <MemoryRouter>
          <McpRoute />
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

describe("mcp servers", () => {
  it("blocks viewers with a 403", async () => {
    const fetchMock = stubApi("viewer")
    renderMcp()

    expect(
      await screen.findByText(
        "Only maintainers and owners can manage MCP servers."
      )
    ).toBeInTheDocument()
    expect(
      fetchMock.mock.calls.some(
        ([input]) => String(input) === "/api/mcp/servers"
      )
    ).toBe(false)
  })

  it("lets a maintainer test a server on the exact test path", async () => {
    const fetchMock = stubApi("maintainer")
    renderMcp()

    expect(await screen.findByText("Context7")).toBeInTheDocument()
    expect(
      screen.getByRole("button", { name: "Test Context7" })
    ).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Edit Context7" })).toBeNull()
    expect(screen.queryByRole("button", { name: "Delete Context7" })).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: "Test Context7" }))

    await waitFor(() => {
      expect(
        fetchMock.mock.calls.some(
          ([input, init]) =>
            String(input) === "/api/mcp/context7/test" &&
            init?.method === "POST"
        )
      ).toBe(true)
    })
    expect(
      fetchMock.mock.calls.some(([input]) =>
        String(input).includes("/api/mcp/servers/context7/test")
      )
    ).toBe(false)
    expect(toast.success).toHaveBeenCalledWith("Context7: 3 tools available")
  })

  it("does not send a blank API key when saving", async () => {
    const fetchMock = stubApi("owner")
    renderMcp()

    fireEvent.click(await screen.findByRole("button", { name: "Edit Context7" }))

    expect(await screen.findByLabelText("API key")).toHaveValue("")

    fireEvent.click(screen.getByRole("switch", { name: "Enabled" }))
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => {
      const [put] = putCalls(fetchMock)
      expect(put).toBeDefined()
      expect(String(put?.[0])).toBe("/api/mcp/servers/context7")
      expect(JSON.parse(String(put?.[1]?.body))).toEqual({ enabled: false })
    })
    expect(JSON.stringify(putCalls(fetchMock))).not.toContain("•")
  })

  it("sends the custom base URL and name when editing a custom server", async () => {
    const fetchMock = stubApi("owner")
    renderMcp()

    fireEvent.click(await screen.findByRole("button", { name: "Edit Internal" }))

    expect(await screen.findByLabelText("Base URL")).toHaveValue(
      "https://mcp.internal.example"
    )
    fireEvent.change(screen.getByLabelText("Name"), {
      target: { value: "Internal tools" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => {
      const [put] = putCalls(fetchMock)
      expect(put).toBeDefined()
      expect(String(put?.[0])).toBe("/api/mcp/servers/custom-1")
      expect(JSON.parse(String(put?.[1]?.body))).toEqual({
        enabled: false,
        base_url: "https://mcp.internal.example",
        name: "Internal tools",
      })
    })
  })

  it("deletes an owner-only server after confirmation", async () => {
    const fetchMock = stubApi("owner")
    renderMcp()

    fireEvent.click(
      await screen.findByRole("button", { name: "Delete Context7" })
    )

    const deleteCalls = () =>
      fetchMock.mock.calls.filter(([, init]) => init?.method === "DELETE")
    expect(deleteCalls()).toHaveLength(0)

    fireEvent.click(screen.getByRole("button", { name: "Delete" }))

    await waitFor(() => {
      const [call] = deleteCalls()
      expect(call).toBeDefined()
      expect(String(call?.[0])).toBe("/api/mcp/servers/context7")
    })
    expect(toast.success).toHaveBeenCalledWith("Context7 removed")
  })
})
