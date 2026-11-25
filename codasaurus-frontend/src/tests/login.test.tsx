import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { MemoryRouter, Route, Routes } from "react-router"
import { afterEach, describe, expect, it, vi } from "vitest"

import { AuthProvider } from "@/lib/auth/auth-context"
import LoginRoute from "@/routes/login"

const user = {
  email: "owner@example.com",
  role: "owner",
  auth_provider: "local",
  is_bootstrap: false,
}

function jsonResponse(body: unknown, init: ResponseInit = {}) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
    ...init,
  })
}

function stubFetch(
  handler: (url: string, init?: RequestInit) => Response
) {
  const fetchMock = vi.fn((input: RequestInfo | URL, init?: RequestInit) =>
    Promise.resolve(handler(String(input), init))
  )
  vi.stubGlobal("fetch", fetchMock)
  return fetchMock
}

function authenticatedResponse(authenticated: boolean) {
  return jsonResponse({
    authenticated,
    user: authenticated ? user : null,
  })
}

function newClient() {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } })
}

function renderLogin(client: QueryClient, from?: string) {
  const entry = from
    ? { pathname: "/login", state: { from } }
    : { pathname: "/login" }

  render(
    <QueryClientProvider client={client}>
      <AuthProvider>
        <MemoryRouter initialEntries={[entry]}>
          <Routes>
            <Route path="/login" element={<LoginRoute />} />
            <Route path="/app/reviews" element={<div>reviews destination</div>} />
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

describe("login route", () => {
  it("shows the SSO link when OIDC is enabled", async () => {
    stubFetch((url) => {
      if (url.includes("/api/auth/oidc/status")) {
        return jsonResponse({ enabled: true })
      }
      return authenticatedResponse(false)
    })
    renderLogin(newClient())

    expect(
      await screen.findByRole("link", { name: "Sign in with SSO" })
    ).toHaveAttribute("href", "/api/auth/oidc/login")
  })

  it("hides the SSO link when OIDC is disabled", async () => {
    stubFetch((url) => {
      if (url.includes("/api/auth/oidc/status")) {
        return jsonResponse({ enabled: false })
      }
      return authenticatedResponse(false)
    })
    const client = newClient()
    renderLogin(client)

    await waitFor(() => expect(client.isFetching()).toBe(0))
    expect(
      screen.queryByRole("link", { name: "Sign in with SSO" })
    ).toBeNull()
  })

  it("shows the backend message when sign-in is rejected", async () => {
    stubFetch((url) => {
      if (url.includes("/api/auth/oidc/status")) {
        return jsonResponse({ enabled: false })
      }
      if (url.includes("/api/auth/login")) {
        return jsonResponse({ error: "Invalid email or password" }, { status: 401 })
      }
      return authenticatedResponse(false)
    })
    renderLogin(newClient())

    fireEvent.change(await screen.findByLabelText("Email"), {
      target: { value: "owner@example.com" },
    })
    fireEvent.change(screen.getByLabelText("Password"), {
      target: { value: "wrong-password" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Sign in" }))

    expect(await screen.findByText("Invalid email or password")).toBeInTheDocument()
  })

  it("shows the retry-after window when rate limited", async () => {
    stubFetch((url) => {
      if (url.includes("/api/auth/oidc/status")) {
        return jsonResponse({ enabled: false })
      }
      if (url.includes("/api/auth/login")) {
        return jsonResponse(
          { error: "Too many attempts. Try again in 15 minutes." },
          { status: 429, headers: { "Retry-After": "60" } }
        )
      }
      return authenticatedResponse(false)
    })
    renderLogin(newClient())

    fireEvent.change(await screen.findByLabelText("Email"), {
      target: { value: "owner@example.com" },
    })
    fireEvent.change(screen.getByLabelText("Password"), {
      target: { value: "wrong-password" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Sign in" }))

    expect(
      await screen.findByText(
        "Too many attempts. Try again in 15 minutes. Please try again shortly."
      )
    ).toBeInTheDocument()
  })

  it("associates field errors with their inputs", async () => {
    stubFetch((url) => {
      if (url.includes("/api/auth/oidc/status")) {
        return jsonResponse({ enabled: false })
      }
      return authenticatedResponse(false)
    })
    renderLogin(newClient())

    fireEvent.click(await screen.findByRole("button", { name: "Sign in" }))

    const email = screen.getByLabelText("Email")
    const emailError = await screen.findByText("Enter a valid email address")
    expect(email).toHaveAttribute("aria-describedby", "login-email-error")
    expect(emailError).toHaveAttribute("id", "login-email-error")
    expect(screen.getByLabelText("Password")).toHaveAttribute(
      "aria-describedby",
      "login-password-error"
    )
  })

  it("returns to the intended route after signing in", async () => {
    let authenticated = false
    stubFetch((url) => {
      if (url.includes("/api/auth/oidc/status")) {
        return jsonResponse({ enabled: false })
      }
      if (url.includes("/api/auth/login")) {
        authenticated = true
        return jsonResponse({ user })
      }
      return authenticatedResponse(authenticated)
    })
    renderLogin(newClient(), "/app/reviews")

    fireEvent.change(await screen.findByLabelText("Email"), {
      target: { value: "owner@example.com" },
    })
    fireEvent.change(screen.getByLabelText("Password"), {
      target: { value: "correct-password" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Sign in" }))

    expect(await screen.findByText("reviews destination")).toBeInTheDocument()
  })
})
