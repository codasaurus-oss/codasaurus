import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react"
import { MemoryRouter } from "react-router"
import { toast } from "sonner"
import { afterEach, describe, expect, it, vi } from "vitest"

import { Header } from "@/components/layout/header"
import { SidebarProvider } from "@/components/ui/sidebar"
import { ChangePasswordDialog } from "@/features/users/change-password-dialog"
import { AuthProvider, useAuth } from "@/lib/auth/auth-context"
import { ThemeProvider } from "@/lib/theme/theme-provider"
import UsersRoute from "@/routes/users"

vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn(), message: vi.fn() },
}))

const roster = [
  {
    id: 1,
    email: "owner@example.com",
    role: "owner",
    auth_provider: "local",
    is_bootstrap: true,
    created_at: "2026-11-20T10:00:00Z",
  },
  {
    id: 2,
    email: "bob@example.com",
    role: "owner",
    auth_provider: "local",
    is_bootstrap: false,
    created_at: "2026-11-21T10:00:00Z",
  },
  {
    id: 3,
    email: "ada@example.com",
    role: "viewer",
    auth_provider: "oidc",
    is_bootstrap: false,
    created_at: "2026-11-22T10:00:00Z",
  },
]

const invite = {
  id: 7,
  email: "new@example.com",
  role: "viewer",
  created_by: "owner@example.com",
  expires_at: "2026-11-30T10:00:00Z",
  created_at: "2026-11-23T10:00:00Z",
}

const createdInvite = {
  id: 8,
  email: "new@example.com",
  role: "viewer",
  expires_at: "2026-11-30T10:00:00Z",
  url: "https://codasaurus.test/#/invite/secret-token-value",
  token: "secret-token-value",
}

function jsonResponse(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

function stubApi(
  options: {
    role?: string
    authProvider?: string
    isBootstrap?: boolean
    users?: object[]
    invites?: object[]
    deleteStatus?: number
    passwordStatus?: number
  } = {}
) {
  const role = options.role ?? "owner"
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
            auth_provider: options.authProvider ?? "local",
            is_bootstrap: options.isBootstrap ?? true,
          },
        })
      )
    }
    if (url === "/api/users" && method === "GET") {
      return Promise.resolve(jsonResponse({ users: options.users ?? roster }))
    }
    if (url === "/api/users/invites" && method === "GET") {
      return Promise.resolve(
        jsonResponse({ invites: options.invites ?? [invite] })
      )
    }
    if (url === "/api/users/invites" && method === "POST") {
      return Promise.resolve(jsonResponse(createdInvite))
    }
    if (url === "/api/users/me/password" && method === "POST") {
      if (options.passwordStatus && options.passwordStatus !== 200) {
        return Promise.resolve(
          jsonResponse({ error: "Current password is incorrect" }, options.passwordStatus)
        )
      }
      return Promise.resolve(jsonResponse({ status: "ok" }))
    }
    if (url.startsWith("/api/users/invites/") && method === "DELETE") {
      return Promise.resolve(jsonResponse({ status: "ok" }))
    }
    if (url.endsWith("/transfer-bootstrap") && method === "POST") {
      return Promise.resolve(
        jsonResponse({
          id: 2,
          email: "bob@example.com",
          role: "owner",
          is_bootstrap: true,
        })
      )
    }
    if (url.startsWith("/api/users/") && method === "PATCH") {
      return Promise.resolve(
        jsonResponse({
          id: 3,
          email: "ada@example.com",
          role: "maintainer",
          is_bootstrap: false,
        })
      )
    }
    if (url.startsWith("/api/users/") && method === "DELETE") {
      if (options.deleteStatus && options.deleteStatus !== 200) {
        return Promise.resolve(
          jsonResponse(
            { error: "Cannot remove the last owner" },
            options.deleteStatus
          )
        )
      }
      return Promise.resolve(jsonResponse({ status: "ok" }))
    }
    return Promise.resolve(
      jsonResponse({ error: `Unmatched request: ${method} ${url}` }, 500)
    )
  })
  vi.stubGlobal("fetch", fetchMock)
  return fetchMock
}

function renderUsers() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  render(
    <QueryClientProvider client={queryClient}>
      <AuthProvider>
        <MemoryRouter>
          <UsersRoute />
        </MemoryRouter>
      </AuthProvider>
    </QueryClientProvider>
  )
  return queryClient
}

function renderHeader(authProvider: "local" | "oidc") {
  stubApi({ role: "owner", authProvider })
  vi.stubGlobal(
    "matchMedia",
    vi.fn().mockReturnValue({
      matches: false,
      media: "",
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })
  )
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  render(
    <QueryClientProvider client={queryClient}>
      <AuthProvider>
        <ThemeProvider>
          <SidebarProvider>
            <MemoryRouter>
              <Header />
            </MemoryRouter>
          </SidebarProvider>
        </ThemeProvider>
      </AuthProvider>
    </QueryClientProvider>
  )
}

function AuthProbe() {
  const { status, refresh } = useAuth()
  return (
    <div>
      <span>{status}</span>
      <button onClick={() => void refresh(true)}>Silent refresh</button>
    </div>
  )
}

async function openAccountMenu() {
  const account = await screen.findByRole("button", { name: "Account" })
  fireEvent.mouseDown(account)
  fireEvent.click(account)
  return account
}

async function chooseRole(email: string, option: string) {
  fireEvent.mouseDown(
    screen.getByRole("combobox", { name: `Role for ${email}` })
  )
  const item = await screen.findByRole("option", { name: option })
  fireEvent.pointerDown(item)
  fireEvent.click(item)
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  vi.unstubAllGlobals()
})

describe("users", () => {
  it("shows a viewer a read-only roster without the invites tab", async () => {
    const fetchMock = stubApi({ role: "viewer" })
    renderUsers()

    expect(await screen.findByText("ada@example.com")).toBeInTheDocument()
    expect(screen.queryByRole("tab", { name: "Invites" })).toBeNull()
    expect(screen.queryByRole("button", { name: "Create invite" })).toBeNull()
    expect(
      screen.queryByRole("combobox", { name: "Role for ada@example.com" })
    ).toBeNull()
    expect(
      screen.queryByRole("button", { name: "Remove ada@example.com" })
    ).toBeNull()
    expect(
      fetchMock.mock.calls.some(
        ([input]) => String(input) === "/api/users/invites"
      )
    ).toBe(false)
  })

  it("creates an invite and shows only the link, not the token", async () => {
    const fetchMock = stubApi({ role: "owner" })
    renderUsers()

    fireEvent.click(await screen.findByRole("tab", { name: "Invites" }))
    fireEvent.click(await screen.findByRole("button", { name: "Create invite" }))

    fireEvent.change(await screen.findByLabelText("Email (optional)"), {
      target: { value: "new@example.com" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Create link" }))

    const url = await screen.findByLabelText("Invite URL")
    expect(url).toHaveValue("https://codasaurus.test/#/invite/secret-token-value")
    expect(screen.queryByText("secret-token-value")).toBeNull()

    await waitFor(() => {
      const post = fetchMock.mock.calls.find(
        ([input, init]) =>
          String(input) === "/api/users/invites" && init?.method === "POST"
      )
      expect(post).toBeDefined()
      expect(JSON.parse(String(post?.[1]?.body))).toEqual({
        email: "new@example.com",
        role: "viewer",
      })
    })
  })

  it("revokes an invite only after confirmation", async () => {
    const fetchMock = stubApi({ role: "owner" })
    renderUsers()

    fireEvent.click(await screen.findByRole("tab", { name: "Invites" }))
    fireEvent.click(
      await screen.findByRole("button", {
        name: "Revoke invite for new@example.com",
      })
    )

    const deleteCalls = () =>
      fetchMock.mock.calls.filter(([, init]) => init?.method === "DELETE")
    expect(deleteCalls()).toHaveLength(0)

    fireEvent.click(screen.getByRole("button", { name: "Revoke" }))

    await waitFor(() => {
      const [call] = deleteCalls()
      expect(call).toBeDefined()
      expect(String(call?.[0])).toBe("/api/users/invites/7")
    })
  })

  it("sends the selected role only after confirmation", async () => {
    const fetchMock = stubApi({ role: "owner" })
    renderUsers()

    await screen.findByText("ada@example.com")
    await chooseRole("ada@example.com", "Maintainer")

    const confirm = await screen.findByRole("button", { name: "Confirm" })
    expect(
      fetchMock.mock.calls.some(([, init]) => init?.method === "PATCH")
    ).toBe(false)

    fireEvent.click(confirm)

    await waitFor(() => {
      const patch = fetchMock.mock.calls.find(
        ([input, init]) =>
          String(input) === "/api/users/3" && init?.method === "PATCH"
      )
      expect(patch).toBeDefined()
      expect(JSON.parse(String(patch?.[1]?.body))).toEqual({
        role: "maintainer",
      })
    })
  })

  it("deletes a user only after confirmation", async () => {
    const fetchMock = stubApi({ role: "owner" })
    renderUsers()

    fireEvent.click(
      await screen.findByRole("button", { name: "Remove ada@example.com" })
    )

    const deleteCalls = () =>
      fetchMock.mock.calls.filter(([, init]) => init?.method === "DELETE")
    expect(deleteCalls()).toHaveLength(0)

    fireEvent.click(screen.getByRole("button", { name: "Delete" }))

    await waitFor(() => {
      const [call] = deleteCalls()
      expect(call).toBeDefined()
      expect(String(call?.[0])).toBe("/api/users/3")
    })
  })

  it("shows a last-owner rejection inline", async () => {
    stubApi({ role: "owner", deleteStatus: 400 })
    renderUsers()

    fireEvent.click(
      await screen.findByRole("button", { name: "Remove ada@example.com" })
    )
    fireEvent.click(screen.getByRole("button", { name: "Delete" }))

    expect(
      await screen.findByText("Cannot remove the last owner")
    ).toBeInTheDocument()
  })

  it("requires the target email typed exactly to transfer bootstrap", async () => {
    const fetchMock = stubApi({ role: "owner", isBootstrap: true })
    renderUsers()

    fireEvent.click(
      await screen.findByRole("button", {
        name: "Transfer bootstrap to bob@example.com",
      })
    )

    const action = screen.getByRole("button", { name: "Transfer bootstrap" })
    const input = await screen.findByLabelText("Confirmation")
    expect(action).toBeDisabled()

    fireEvent.change(input, { target: { value: "bob@example" } })
    expect(action).toBeDisabled()

    fireEvent.change(input, { target: { value: "bob@example.com" } })
    expect(action).toBeEnabled()

    fireEvent.keyDown(input, { key: "Enter" })

    await waitFor(() => {
      const post = fetchMock.mock.calls.find(
        ([input, init]) =>
          String(input) === "/api/users/2/transfer-bootstrap" &&
          init?.method === "POST"
      )
      expect(post).toBeDefined()
    })

    fireEvent.click(
      await screen.findByRole("button", {
        name: "Transfer bootstrap to bob@example.com",
      })
    )
    expect(await screen.findByLabelText("Confirmation")).toHaveValue("")

    fireEvent.change(screen.getByLabelText("Confirmation"), {
      target: { value: "bob" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }))

    fireEvent.click(
      await screen.findByRole("button", {
        name: "Transfer bootstrap to bob@example.com",
      })
    )
    expect(await screen.findByLabelText("Confirmation")).toHaveValue("")
  })

  it("hides transfer bootstrap from a non-bootstrap owner", async () => {
    const fetchMock = stubApi({ role: "owner", isBootstrap: false })
    renderUsers()

    expect(await screen.findByText("ada@example.com")).toBeInTheDocument()
    expect(
      screen.queryByRole("button", { name: /Transfer bootstrap/ })
    ).toBeNull()
    expect(
      fetchMock.mock.calls.some(([input]) =>
        String(input).endsWith("/transfer-bootstrap")
      )
    ).toBe(false)
  })

  it("hides remove for the bootstrap owner and the current user", async () => {
    const members = [
      {
        id: 1,
        email: "bootstrap@example.com",
        role: "owner",
        auth_provider: "local",
        is_bootstrap: true,
        created_at: "2026-11-20T10:00:00Z",
      },
      {
        id: 2,
        email: "owner@example.com",
        role: "owner",
        auth_provider: "local",
        is_bootstrap: false,
        created_at: "2026-11-21T10:00:00Z",
      },
      {
        id: 3,
        email: "ada@example.com",
        role: "viewer",
        auth_provider: "local",
        is_bootstrap: false,
        created_at: "2026-11-22T10:00:00Z",
      },
    ]
    stubApi({ role: "owner", isBootstrap: false, users: members })
    renderUsers()

    expect(await screen.findByText("ada@example.com")).toBeInTheDocument()
    expect(
      screen.queryByRole("button", { name: "Remove bootstrap@example.com" })
    ).toBeNull()
    expect(
      screen.queryByRole("button", { name: "Remove owner@example.com" })
    ).toBeNull()
    expect(
      screen.getByRole("button", { name: "Remove ada@example.com" })
    ).toBeInTheDocument()
  })

  it("disables role changes and removal for the last owner", async () => {
    stubApi({
      role: "owner",
      users: [
        {
          id: 1,
          email: "owner@example.com",
          role: "owner",
          auth_provider: "local",
          is_bootstrap: true,
          created_at: "2026-11-20T10:00:00Z",
        },
      ],
    })
    renderUsers()

    expect(
      await screen.findByRole("combobox", {
        name: "Role for owner@example.com",
      })
    ).toBeDisabled()
    expect(
      screen.queryByRole("button", { name: "Remove owner@example.com" })
    ).toBeNull()
  })
})

describe("silent auth refresh", () => {
  it("keeps the signed-in state without a loading flash", async () => {
    const me = {
      authenticated: true,
      user: {
        email: "owner@example.com",
        role: "owner",
        auth_provider: "local",
        is_bootstrap: true,
      },
    }
    let resolveRefresh: ((response: Response) => void) | undefined
    let calls = 0
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        if (String(input) !== "/api/auth/me") {
          return Promise.resolve(jsonResponse({}, 500))
        }
        calls += 1
        if (calls === 1) {
          return Promise.resolve(jsonResponse(me))
        }
        return new Promise<Response>((resolve) => {
          resolveRefresh = resolve
        })
      })
    )

    render(
      <AuthProvider>
        <AuthProbe />
      </AuthProvider>
    )

    expect(await screen.findByText("authenticated")).toBeInTheDocument()

    fireEvent.click(screen.getByRole("button", { name: "Silent refresh" }))

    await waitFor(() => expect(resolveRefresh).toBeDefined())
    expect(screen.getByText("authenticated")).toBeInTheDocument()
    expect(screen.queryByText("loading")).toBeNull()

    await act(async () => {
      resolveRefresh?.(jsonResponse(me))
    })
    expect(screen.getByText("authenticated")).toBeInTheDocument()
  })
})

describe("change password", () => {
  it("enforces the password policy locally, then submits", async () => {
    const fetchMock = stubApi()
    const onOpenChange = vi.fn()
    render(
      <ChangePasswordDialog
        open
        onOpenChange={onOpenChange}
        email="user@example.com"
      />
    )

    const current = screen.getByLabelText("Current password")
    const next = screen.getByLabelText("New password")
    const submit = screen.getByRole("button", { name: "Change password" })

    fireEvent.change(current, { target: { value: "old-password-1" } })
    fireEvent.change(next, { target: { value: "short" } })
    fireEvent.click(submit)

    expect(
      await screen.findByText("Password must be at least 10 characters")
    ).toBeInTheDocument()
    expect(
      fetchMock.mock.calls.some(([, init]) => init?.method === "POST")
    ).toBe(false)

    fireEvent.change(next, { target: { value: "User@Example.com" } })
    fireEvent.click(submit)
    expect(
      await screen.findByText("Password must not be the same as your email")
    ).toBeInTheDocument()

    fireEvent.change(next, { target: { value: "a-long-enough-password" } })
    fireEvent.click(submit)

    await waitFor(() => {
      const post = fetchMock.mock.calls.find(
        ([input, init]) =>
          String(input) === "/api/users/me/password" && init?.method === "POST"
      )
      expect(post).toBeDefined()
      expect(JSON.parse(String(post?.[1]?.body))).toEqual({
        current_password: "old-password-1",
        new_password: "a-long-enough-password",
      })
    })
    expect(toast.success).toHaveBeenCalledWith("Password changed")
    expect(onOpenChange).toHaveBeenCalledWith(false)
  })

  it("offers change password to a local-password account", async () => {
    renderHeader("local")
    await openAccountMenu()

    expect(await screen.findByText("Log out")).toBeInTheDocument()
    expect(screen.getByText("Change password")).toBeInTheDocument()
  })

  it("hides change password from an OIDC account", async () => {
    renderHeader("oidc")
    await openAccountMenu()

    expect(await screen.findByText("Log out")).toBeInTheDocument()
    expect(screen.queryByText("Change password")).toBeNull()
  })
})
