import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { MemoryRouter, Route, Routes } from "react-router"
import { afterEach, describe, expect, it, vi } from "vitest"

import { ErrorState } from "@/components/common/error-state"
import { apiFetch } from "@/lib/api/client"
import { ApiError } from "@/lib/api/errors"
import { meResponseSchema, usersResponseSchema } from "@/lib/api/schemas"
import { AuthProvider, useAuth } from "@/lib/auth/auth-context"
import { can } from "@/lib/auth/roles"
import { RequireAuth } from "@/routes/router"

function jsonResponse(body: unknown, init: ResponseInit = {}) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
    ...init,
  })
}

function stubMe(role: "owner" | "maintainer" | "viewer") {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(
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
  )
}

function OwnerOnlyActions() {
  const { user } = useAuth()

  return (
    <div>
      <span>{user?.email}</span>
      {can(user?.role, "owner") ? <button>Delete user</button> : null}
    </div>
  )
}

function renderOwnerOnlyActions() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={client}>
      <AuthProvider>
        <OwnerOnlyActions />
      </AuthProvider>
    </QueryClientProvider>
  )
}

const setupStatus = {
  database: true,
  llm: true,
  github: true,
  admin: true,
  complete: true,
  github_install_url: null,
  public_url: "http://localhost:8000",
}

function stubApi(me: () => Response) {
  const fetchMock = vi.fn((input: RequestInfo | URL) =>
    Promise.resolve(
      String(input).includes("/api/auth/me") ? me() : jsonResponse(setupStatus)
    )
  )
  vi.stubGlobal("fetch", fetchMock)
  return fetchMock
}

function renderRequireAuth() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={client}>
      <AuthProvider>
        <MemoryRouter initialEntries={["/app/dashboard"]}>
          <Routes>
            <Route path="/app" element={<RequireAuth />}>
              <Route path="dashboard" element={<div>dashboard</div>} />
            </Route>
            <Route path="/login" element={<div>login screen</div>} />
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

describe("auth gating", () => {
  it("hides owner-only actions from a viewer", async () => {
    stubMe("viewer")
    renderOwnerOnlyActions()

    expect(await screen.findByText("viewer@example.com")).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Delete user" })).toBeNull()
  })

  it("shows the same action to an owner", async () => {
    stubMe("owner")
    renderOwnerOnlyActions()

    expect(await screen.findByRole("button", { name: "Delete user" })).toBeInTheDocument()
  })

  it("renders ErrorState for a direct 403 instead of blank content", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        jsonResponse(
          { error: "Insufficient permissions for this action" },
          { status: 403 }
        )
      )
    )

    const error = await apiFetch("/api/users").catch((caught: unknown) => caught)
    expect(error).toBeInstanceOf(ApiError)
    expect((error as ApiError).status).toBe(403)

    render(<ErrorState error={error} />)

    expect(screen.getByText("Something went wrong")).toBeInTheDocument()
    expect(
      screen.getByText("Insufficient permissions for this action")
    ).toBeInTheDocument()
  })

  it("offers a retry instead of the login screen when the session probe fails", async () => {
    let meStatus = 500
    const fetchMock = stubApi(() =>
      jsonResponse({ error: "Internal server error" }, { status: meStatus })
    )
    renderRequireAuth()

    expect(await screen.findByRole("button", { name: "Try again" })).toBeInTheDocument()
    expect(screen.queryByText("login screen")).toBeNull()

    meStatus = 401
    fireEvent.click(screen.getByRole("button", { name: "Try again" }))

    expect(await screen.findByText("login screen")).toBeInTheDocument()
    expect(
      fetchMock.mock.calls.filter(([input]) => String(input).includes("/api/auth/me"))
    ).toHaveLength(2)
  })

  it("sends the user to login when the session probe is rejected", async () => {
    stubApi(() => jsonResponse({ error: "Bad request" }, { status: 400 }))
    renderRequireAuth()

    expect(await screen.findByText("login screen")).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull()
  })

  it("normalizes a legacy admin role to owner", () => {
    const me = meResponseSchema.parse({
      authenticated: true,
      user: {
        email: "admin@example.com",
        role: "admin",
        auth_provider: "local",
        is_bootstrap: false,
      },
    })

    expect(me.user?.role).toBe("owner")

    const users = usersResponseSchema.parse({
      users: [
        {
          id: 1,
          email: "admin@example.com",
          role: "admin",
          auth_provider: "local",
          is_bootstrap: false,
          created_at: "2026-11-20T10:00:00Z",
        },
      ],
    })

    expect(users.users[0].role).toBe("owner")
  })
})
