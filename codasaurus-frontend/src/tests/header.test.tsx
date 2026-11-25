import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { cleanup, render, waitFor } from "@testing-library/react"
import { MemoryRouter } from "react-router"
import { afterEach, describe, expect, it, vi } from "vitest"

import { AppShell } from "@/components/layout/app-shell"
import { AuthProvider } from "@/lib/auth/auth-context"
import { ThemeProvider } from "@/lib/theme/theme-provider"

Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  }),
})

function jsonResponse(body: unknown) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  })
}

function stubApi() {
  vi.stubGlobal("fetch", (input: RequestInfo | URL) => {
    const url = String(input)
    if (url === "/api/auth/me") {
      return Promise.resolve(
        jsonResponse({
          authenticated: true,
          user: { email: "lohit@example.com", role: "owner" },
        })
      )
    }
    if (url === "/api/setup/status") {
      return Promise.resolve(
        jsonResponse({
          database: true,
          llm: true,
          github: true,
          admin: true,
          complete: true,
          github_install_url: null,
          public_url: "http://localhost:3000",
        })
      )
    }
    if (url === "/api/health") {
      return Promise.resolve(
        jsonResponse({ status: "ok", ready: true, offline_mode: false })
      )
    }
    return Promise.resolve(jsonResponse({}))
  })
}

function renderShell(path: string) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  render(
    <QueryClientProvider client={client}>
      <AuthProvider>
        <ThemeProvider>
          <MemoryRouter initialEntries={[path]}>
            <AppShell>
              <div>page content</div>
            </AppShell>
          </MemoryRouter>
        </ThemeProvider>
      </AuthProvider>
    </QueryClientProvider>
  )
}

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

describe("application shell", () => {
  it("shows the active route in the breadcrumb", async () => {
    stubApi()
    renderShell("/app/repos")

    await waitFor(() =>
      expect(
        document.querySelector('[data-slot="breadcrumb-page"]')
      ).toHaveTextContent("Repositories")
    )
  })

  it("shows a detail breadcrumb for a nested review route", async () => {
    stubApi()
    renderShell("/app/reviews/42")

    await waitFor(() =>
      expect(
        document.querySelector('[data-slot="breadcrumb-page"]')
      ).toHaveTextContent("Review detail")
    )
  })
})
