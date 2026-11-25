import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react"
import { MemoryRouter } from "react-router"
import { afterEach, describe, expect, it, vi } from "vitest"

import { AuthProvider } from "@/lib/auth/auth-context"
import AuditRoute from "@/routes/audit"

const entries = [
  {
    id: 1,
    event_type: "user.login",
    actor: "owner@example.com",
    target_type: "user",
    target_id: 3,
    metadata_json:
      '{"ip":"127.0.0.1","ok":true,"auth":{"api_key":"sk-live-123","nested":[{"token":"tkn-456"},{"safe":"kept"}]}}',
    created_at: "2026-11-23T10:00:00Z",
  },
  {
    id: 2,
    event_type: "settings.updated",
    actor: "owner@example.com",
    target_type: null,
    target_id: null,
    metadata_json: "<b>not json</b>",
    created_at: "2026-11-23T09:00:00Z",
  },
]

function jsonResponse(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

function stubApi(role = "owner", totalPages = () => 2) {
  const fetchMock = vi.fn((input: RequestInfo | URL) => {
    const url = String(input)
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
    if (url.startsWith("/api/audit")) {
      return Promise.resolve(
        jsonResponse({
          entries,
          total: 60,
          total_pages: totalPages(),
          limit: 50,
          offset: 0,
          retention_days: 30,
        })
      )
    }
    return Promise.resolve(
      jsonResponse({ error: `Unmatched request: ${url}` }, 500)
    )
  })
  vi.stubGlobal("fetch", fetchMock)
  return fetchMock
}

function renderAudit() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  render(
    <QueryClientProvider client={queryClient}>
      <AuthProvider>
        <MemoryRouter>
          <AuditRoute />
        </MemoryRouter>
      </AuthProvider>
    </QueryClientProvider>
  )
  return queryClient
}

function auditCalls(fetchMock: ReturnType<typeof stubApi>) {
  return fetchMock.mock.calls
    .map(([input]) => String(input))
    .filter((url) => url.startsWith("/api/audit"))
}

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

describe("audit", () => {
  it("filters by event type and pages with limit 50", async () => {
    const fetchMock = stubApi()
    renderAudit()

    expect(await screen.findByText("user.login")).toBeInTheDocument()
    expect(
      screen.getByText("Entries older than 30 days are purged automatically.")
    ).toBeInTheDocument()

    fireEvent.mouseDown(
      screen.getByRole("combobox", { name: "Filter by event type" })
    )
    const option = await screen.findByRole("option", { name: "user.login" })
    fireEvent.pointerDown(option)
    fireEvent.click(option)

    await waitFor(() => {
      expect(
        auditCalls(fetchMock).some(
          (url) =>
            url.includes("event_type=user.login") && url.includes("offset=0")
        )
      ).toBe(true)
    })

    fireEvent.click(screen.getByLabelText("Go to next page"))

    await waitFor(() => {
      expect(
        auditCalls(fetchMock).some(
          (url) =>
            url.includes("event_type=user.login") && url.includes("offset=50")
        )
      ).toBe(true)
    })
    expect(screen.getByText("Page 2 of 2")).toBeInTheDocument()
  })

  it("clamps the page when the result set shrinks", async () => {
    let pages = 3
    const fetchMock = stubApi("owner", () => pages)
    const client = renderAudit()

    expect(await screen.findByText("user.login")).toBeInTheDocument()
    fireEvent.click(screen.getByLabelText("Go to next page"))
    fireEvent.click(await screen.findByLabelText("Go to next page"))

    await waitFor(() => {
      expect(
        auditCalls(fetchMock).some((url) => url.includes("offset=100"))
      ).toBe(true)
    })
    expect(await screen.findByText("Page 3 of 3")).toBeInTheDocument()

    pages = 2
    await client.invalidateQueries({ queryKey: ["audit"] })

    await waitFor(() => {
      expect(
        auditCalls(fetchMock).some((url) => url.includes("offset=50"))
      ).toBe(true)
    })
    expect(await screen.findByText("Page 2 of 2")).toBeInTheDocument()
  })

  it("shows JSON metadata as formatted text and never renders HTML", async () => {
    stubApi()
    renderAudit()

    fireEvent.click(
      await screen.findByRole("button", {
        name: "View user.login metadata for owner@example.com",
      })
    )

    expect(
      await screen.findByText(/"ip": "127.0.0.1"/)
    ).toBeInTheDocument()

    fireEvent.click(screen.getByRole("button", { name: "Close" }))

    fireEvent.click(
      await screen.findByRole("button", {
        name: "View settings.updated metadata for owner@example.com",
      })
    )

    expect(
      await screen.findByText("Metadata cannot be displayed.")
    ).toBeInTheDocument()
    expect(screen.queryByText("<b>not json</b>")).toBeNull()
    expect(document.querySelector("b")).toBeNull()
  })

  it("redacts sensitive values at every nesting depth", async () => {
    stubApi()
    renderAudit()

    fireEvent.click(
      await screen.findByRole("button", {
        name: "View user.login metadata for owner@example.com",
      })
    )

    const metadata = await screen.findByText(/"ip": "127.0.0.1"/)
    const text = metadata.textContent ?? ""
    expect(text).not.toContain("sk-live-123")
    expect(text).not.toContain("tkn-456")
    expect(text.match(/\[redacted\]/g)).toHaveLength(2)
    expect(text).toContain('"safe": "kept"')
  })

  it("blocks viewers with a 403", async () => {
    const fetchMock = stubApi("viewer")
    renderAudit()

    expect(
      await screen.findByText(
        "Only maintainers and owners can view the audit log."
      )
    ).toBeInTheDocument()
    expect(auditCalls(fetchMock)).toHaveLength(0)
  })
})
