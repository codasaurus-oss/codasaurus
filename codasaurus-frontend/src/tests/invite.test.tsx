import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { MemoryRouter, Route, Routes } from "react-router"
import { afterEach, describe, expect, it, vi } from "vitest"

import { AuthProvider } from "@/lib/auth/auth-context"
import InviteRoute from "@/routes/invite"

const expiresAt = "2026-12-01T10:00:00Z"

function jsonResponse(body: unknown, init: ResponseInit = {}) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
    ...init,
  })
}

function stubInviteFetch(
  invite: Response,
  handler?: (url: string) => Response | undefined
) {
  const fetchMock = vi.fn((input: RequestInfo | URL) => {
    const url = String(input)
    const handled = handler?.(url)
    if (handled) {
      return Promise.resolve(handled)
    }
    if (url.includes("/api/auth/invite/")) {
      return Promise.resolve(invite)
    }
    return Promise.resolve(
      jsonResponse({ authenticated: false, user: null })
    )
  })
  vi.stubGlobal("fetch", fetchMock)
  return fetchMock
}

function newClient() {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } })
}

function renderInvite(client: QueryClient, token = "invite-token") {
  render(
    <QueryClientProvider client={client}>
      <AuthProvider>
        <MemoryRouter initialEntries={[`/invite/${token}`]}>
          <Routes>
            <Route path="/invite/:token" element={<InviteRoute />} />
            <Route path="/login" element={<div>login screen</div>} />
            <Route path="/app/dashboard" element={<div>dashboard</div>} />
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

describe("invite route", () => {
  it("shows an empty state with a login link for an expired invite", async () => {
    stubInviteFetch(
      jsonResponse({ error: "Invite not found or expired" }, { status: 404 })
    )
    renderInvite(newClient())

    expect(await screen.findByText("Invite unavailable")).toBeInTheDocument()
    expect(screen.getByText("Invite not found or expired")).toBeInTheDocument()
    expect(
      screen.getByRole("link", { name: "Back to sign in" })
    ).toHaveAttribute("href", "#/login")
  })

  it("disables the email field when the invite is email-locked", async () => {
    stubInviteFetch(
      jsonResponse({
        email: "locked@example.com",
        role: "admin",
        expires_at: expiresAt,
        email_locked: true,
      })
    )
    renderInvite(newClient())

    const email = await screen.findByLabelText("Email")
    expect(email).toHaveAttribute("readonly")
    expect(email).toHaveValue("locked@example.com")
    expect(screen.getByText("Owner")).toBeInTheDocument()
  })

  it("requires an email when the invite is not locked to one", async () => {
    stubInviteFetch(
      jsonResponse({
        email: null,
        role: "viewer",
        expires_at: expiresAt,
        email_locked: false,
      })
    )
    renderInvite(newClient())

    await screen.findByLabelText("Email")
    fireEvent.change(screen.getByLabelText("Password"), {
      target: { value: "long-enough-password" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Accept invite" }))

    expect(
      await screen.findByText("Enter a valid email address")
    ).toBeInTheDocument()
  })

  it("associates descriptions and errors with the invite fields", async () => {
    stubInviteFetch(
      jsonResponse({
        email: null,
        role: "viewer",
        expires_at: expiresAt,
        email_locked: false,
      })
    )
    renderInvite(newClient())

    const email = await screen.findByLabelText("Email")
    expect(email).toHaveAttribute(
      "aria-describedby",
      "invite-email-description"
    )
    expect(
      screen.getByText("Use the email address you want to sign in with.")
    ).toHaveAttribute("id", "invite-email-description")

    fireEvent.click(screen.getByRole("button", { name: "Accept invite" }))

    const emailError = await screen.findByText("Enter a valid email address")
    expect(email).toHaveAttribute(
      "aria-describedby",
      "invite-email-description invite-email-error"
    )
    expect(emailError).toHaveAttribute("id", "invite-email-error")
  })

  it("rejects a password shorter than ten characters", async () => {
    stubInviteFetch(
      jsonResponse({
        email: "locked@example.com",
        role: "owner",
        expires_at: expiresAt,
        email_locked: true,
      })
    )
    renderInvite(newClient())

    fireEvent.change(await screen.findByLabelText("Password"), {
      target: { value: "short" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Accept invite" }))

    expect(
      await screen.findByText("Password must be at least 10 characters")
    ).toBeInTheDocument()
  })

  it("rejects a password equal to the invite email", async () => {
    stubInviteFetch(
      jsonResponse({
        email: "locked@example.com",
        role: "owner",
        expires_at: expiresAt,
        email_locked: true,
      })
    )
    renderInvite(newClient())

    fireEvent.change(await screen.findByLabelText("Password"), {
      target: { value: "LOCKED@example.com" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Accept invite" }))

    expect(
      await screen.findByText("Password must not be the same as your email")
    ).toBeInTheDocument()
  })

  it("shows a retry-after message when accept is rate limited", async () => {
    stubInviteFetch(
      jsonResponse({
        email: "locked@example.com",
        role: "owner",
        expires_at: expiresAt,
        email_locked: true,
      }),
      (url) =>
        url.includes("/accept")
          ? jsonResponse(
              { error: "Too many attempts. Try again in 15 minutes." },
              { status: 429, headers: { "Retry-After": "900" } }
            )
          : undefined
    )
    renderInvite(newClient())

    fireEvent.change(await screen.findByLabelText("Password"), {
      target: { value: "a-very-good-password" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Accept invite" }))

    expect(
      await screen.findByText(
        "Too many attempts. Try again in 15 minutes. Please try again shortly."
      )
    ).toBeInTheDocument()
  })

  it("accepts a valid invite and lands on the dashboard", async () => {
    let accepted = false
    stubInviteFetch(
      jsonResponse({
        email: "locked@example.com",
        role: "owner",
        expires_at: expiresAt,
        email_locked: true,
      }),
      (url) => {
        if (url.includes("/accept")) {
          accepted = true
          return jsonResponse({
            user: { email: "locked@example.com", role: "owner" },
          })
        }
        return undefined
      }
    )
    renderInvite(newClient())

    fireEvent.change(await screen.findByLabelText("Password"), {
      target: { value: "a-very-good-password" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Accept invite" }))

    expect(await screen.findByText("dashboard")).toBeInTheDocument()
    await waitFor(() => expect(accepted).toBe(true))
  })
})
