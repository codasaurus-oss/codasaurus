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
import LearningRoute from "@/routes/learning"

vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn(), message: vi.fn() },
}))

const rules = [
  {
    id: "rule-1",
    detector: "secrets",
    file_pattern: "src/**",
    message_pattern: "AWS access key",
    action: "suppress",
    reason: "maintainer dismissal",
    status: "suggested",
    source_count: 3,
    repo_full_name: "acme/widgets",
    created_at: "2026-11-23T10:00:00Z",
  },
  {
    id: "rule-2",
    detector: "todo_leaks",
    file_pattern: null,
    message_pattern: null,
    action: "downgrade",
    reason: "low signal",
    status: "approved",
    source_count: 5,
    repo_full_name: null,
    created_at: "2026-11-22T10:00:00Z",
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
    if (url === "/api/learning/rules" && method === "GET") {
      return Promise.resolve(jsonResponse({ rules }))
    }
    if (url.endsWith("/approve") && method === "POST") {
      return Promise.resolve(jsonResponse({ status: "ok", id: "rule-1" }))
    }
    if (url.endsWith("/archive") && method === "POST") {
      return Promise.resolve(jsonResponse({ status: "ok", id: "rule-2" }))
    }
    if (url.startsWith("/api/learning/rules/") && method === "DELETE") {
      return Promise.resolve(jsonResponse({ status: "ok", id: "rule-1" }))
    }
    return Promise.resolve(
      jsonResponse({ error: `Unmatched request: ${method} ${url}` }, 500)
    )
  })
  vi.stubGlobal("fetch", fetchMock)
  return fetchMock
}

function renderLearning() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  render(
    <QueryClientProvider client={queryClient}>
      <AuthProvider>
        <MemoryRouter>
          <LearningRoute />
        </MemoryRouter>
      </AuthProvider>
    </QueryClientProvider>
  )
  return queryClient
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  vi.unstubAllGlobals()
})

describe("learning rules", () => {
  it("blocks viewers with a 403 and fetches nothing", async () => {
    const fetchMock = stubApi("viewer")
    renderLearning()

    expect(
      await screen.findByText(
        "Only maintainers and owners can manage learning rules."
      )
    ).toBeInTheDocument()
    expect(
      fetchMock.mock.calls.some(
        ([input]) => String(input) === "/api/learning/rules"
      )
    ).toBe(false)
  })

  it("approves a suggested rule and invalidates the list", async () => {
    const fetchMock = stubApi("maintainer")
    const client = renderLearning()
    const invalidate = vi.spyOn(client, "invalidateQueries")

    fireEvent.click(
      await screen.findByRole("button", { name: "Approve rule rule-1" })
    )

    await waitFor(() => {
      expect(
        fetchMock.mock.calls.some(
          ([input, init]) =>
            String(input) === "/api/learning/rules/rule-1/approve" &&
            init?.method === "POST"
        )
      ).toBe(true)
    })
    await waitFor(() =>
      expect(invalidate).toHaveBeenCalledWith({
        queryKey: ["learning", "rules"],
      })
    )
    expect(toast.success).toHaveBeenCalledWith("Rule approved")
  })

  it("archives an approved rule without a confirmation", async () => {
    const fetchMock = stubApi("maintainer")
    renderLearning()

    expect(
      await screen.findByRole("button", { name: "Approve rule rule-1" })
    ).toBeInTheDocument()
    expect(
      screen.queryByRole("button", { name: "Approve rule rule-2" })
    ).toBeNull()

    fireEvent.click(
      screen.getByRole("button", { name: "Archive rule rule-2" })
    )

    await waitFor(() => {
      expect(
        fetchMock.mock.calls.some(
          ([input, init]) =>
            String(input) === "/api/learning/rules/rule-2/archive" &&
            init?.method === "POST"
        )
      ).toBe(true)
    })
  })

  it("deletes a rule only after confirmation", async () => {
    const fetchMock = stubApi("maintainer")
    renderLearning()

    fireEvent.click(
      await screen.findByRole("button", { name: "Delete rule rule-1" })
    )

    const deleteCalls = () =>
      fetchMock.mock.calls.filter(([, init]) => init?.method === "DELETE")
    expect(deleteCalls()).toHaveLength(0)

    fireEvent.click(screen.getByRole("button", { name: "Delete" }))

    await waitFor(() => {
      const [call] = deleteCalls()
      expect(call).toBeDefined()
      expect(String(call?.[0])).toBe("/api/learning/rules/rule-1")
    })
    expect(toast.success).toHaveBeenCalledWith("Rule deleted")
  })
})
