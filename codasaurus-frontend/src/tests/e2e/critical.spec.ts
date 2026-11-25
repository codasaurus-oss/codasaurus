import { expect, test, type Page } from "@playwright/test"

const owner = {
  email: "owner@example.com",
  role: "owner",
  auth_provider: "local",
  is_bootstrap: false,
}

const anonymous = { authenticated: false, user: null }
const authenticated = { authenticated: true, user: owner }

const setupComplete = {
  database: true,
  llm: true,
  github: true,
  admin: true,
  complete: true,
  github_install_url: null,
  public_url: "https://codasaurus.example.com",
}

const health = {
  status: "ok",
  db: true,
  data_dir: true,
  ready: true,
  version: "e2e",
  egress_profile: "online",
  offline_mode: false,
  network: { llm: true, registries: true, osv: true },
  note: "",
}

const stats = {
  total_repos: 1,
  total_reviews_today: 1,
  pass_rate: 100,
  total_findings: 0,
  reviews_last_7_days: 1,
  dismissals_last_7_days: 0,
}

const repo = {
  id: 1,
  github_id: 42,
  full_name: "acme/web",
  owner: "acme",
  name: "web",
  default_branch: "main",
  installation_id: 7,
  private: false,
  active: true,
  config_json: null,
  created_at: "2026-11-20T10:00:00Z",
  updated_at: "2026-11-20T10:00:00Z",
}

const review = {
  id: 42,
  repo_id: 1,
  pr_number: 7,
  pr_title: "Harden auth",
  pr_author: "ada",
  pr_base_branch: "main",
  pr_head_branch: "feature",
  pr_head_sha: "abcdef1234567890",
  status: "passed",
  summary_json: null,
  started_at: "2026-11-23T10:00:00Z",
  completed_at: "2026-11-23T10:00:42Z",
  created_at: "2026-11-23T09:59:00Z",
  repo_full_name: "acme/web",
}

type Reply = {
  body: unknown
  status?: number
  headers?: Record<string, string>
}

// Every critical flow drives the real UI against a stubbed backend, so the
// suite needs no database and no running Codasaurus server.
async function mockApi(
  page: Page,
  handler: (method: string, path: string) => Reply | null
) {
  await page.route("**/api/**", async (route) => {
    const request = route.request()
    const reply = handler(request.method(), new URL(request.url()).pathname)
    if (!reply) {
      await route.fulfill({ status: 404, json: { error: "not_found" } })
      return
    }
    await route.fulfill({
      status: reply.status ?? 200,
      json: reply.body,
      headers: reply.headers,
    })
  })
  await page.route("**/health", (route) => route.fulfill({ json: health }))
}

test("sign-in stores the session cookie and opens the dashboard", async ({
  page,
  context,
}) => {
  let signedIn = false
  await mockApi(page, (method, path) => {
    if (path === "/api/auth/login" && method === "POST") {
      signedIn = true
      return {
        body: { user: owner },
        headers: {
          "set-cookie":
            "codasaurus_session=e2e-session; Path=/; HttpOnly; SameSite=Lax",
        },
      }
    }
    if (path === "/api/auth/me") {
      return { body: signedIn ? authenticated : anonymous }
    }
    if (path === "/api/setup/status") {
      return { body: setupComplete }
    }
    if (path === "/api/stats") {
      return { body: stats }
    }
    return null
  })

  await page.goto("/#/login")
  await expect(page).toHaveURL(/#\/login/)

  await page.getByLabel("Email").fill("owner@example.com")
  await page.getByLabel("Password").fill("correct-horse-battery")
  await page.getByRole("button", { name: "Sign in" }).click()

  await expect(page).toHaveURL(/#\/app\/dashboard/)
  const cookie = (await context.cookies()).find(
    (item) => item.name === "codasaurus_session"
  )
  expect(cookie?.value).toBe("e2e-session")
})

test("accepting an invite signs the user in and opens the dashboard", async ({
  page,
  context,
}) => {
  let accepted = false
  await mockApi(page, (method, path) => {
    if (path === "/api/auth/invite/invite-token" && method === "GET") {
      return {
        body: {
          email: "invitee@example.com",
          role: "maintainer",
          expires_at: "2026-12-01T10:00:00Z",
          email_locked: true,
        },
      }
    }
    if (path === "/api/auth/invite/invite-token/accept" && method === "POST") {
      accepted = true
      return {
        body: { user: owner },
        headers: {
          "set-cookie": "codasaurus_session=invite-session; Path=/; HttpOnly",
        },
      }
    }
    if (path === "/api/auth/me") {
      return { body: accepted ? authenticated : anonymous }
    }
    if (path === "/api/setup/status") {
      return { body: setupComplete }
    }
    if (path === "/api/stats") {
      return { body: stats }
    }
    return null
  })

  await page.goto("/#/invite/invite-token")
  await expect(page).toHaveURL(/#\/invite\/invite-token/)
  await expect(page.getByText("Accept your invitation")).toBeVisible()

  await page.getByLabel("Password").fill("a-very-good-password")
  await page.getByRole("button", { name: "Accept invite" }).click()

  await expect(page).toHaveURL(/#\/app\/dashboard/)
  const cookie = (await context.cookies()).find(
    (item) => item.name === "codasaurus_session"
  )
  expect(cookie?.value).toBe("invite-session")
})

test("the setup wizard completes and lands on the completion screen", async ({
  page,
}) => {
  let admin = false
  await mockApi(page, (method, path) => {
    if (path === "/api/setup/status") {
      return {
        body: {
          database: true,
          llm: true,
          github: true,
          admin,
          complete: admin,
          github_install_url: null,
          public_url: "https://codasaurus.example.com",
        },
      }
    }
    if (path === "/api/setup/admin" && method === "POST") {
      admin = true
      return { body: { status: "ok" } }
    }
    if (path === "/api/auth/me") {
      return { body: anonymous }
    }
    return null
  })

  await page.goto("/#/setup")
  await expect(page).toHaveURL(/#\/setup/)
  await expect(page.getByText("First-run setup")).toBeVisible()

  await page.getByLabel("Email").fill("owner@example.com")
  await page.getByRole("textbox", { name: "Password" }).fill("correct-horse-battery")
  await page.getByRole("textbox", { name: "Setup secret" }).fill("bootstrap-secret")
  await page.getByRole("button", { name: "Create owner" }).click()

  await expect(page).toHaveURL(/#\/setup\/complete/)
  await expect(page.getByText("Setup complete")).toBeVisible()
})

test("reviews render with data and fall back to the empty state", async ({
  page,
}) => {
  let hasReviews = true
  await mockApi(page, (_method, path) => {
    if (path === "/api/auth/me") {
      return { body: authenticated }
    }
    if (path === "/api/setup/status") {
      return { body: setupComplete }
    }
    if (path === "/api/repos") {
      return { body: [repo] }
    }
    if (path === "/api/reviews") {
      return {
        body: hasReviews
          ? { reviews: [review], total: 1, total_pages: 1, limit: 20, offset: 0 }
          : { reviews: [], total: 0, total_pages: 0, limit: 20, offset: 0 },
      }
    }
    if (path === "/api/stats") {
      return { body: stats }
    }
    return null
  })

  await page.goto("/#/app/reviews")
  await expect(page).toHaveURL(/#\/app\/reviews/)
  await expect(page.getByRole("link", { name: "#7" })).toBeVisible()
  await expect(page.getByText("acme/web")).toBeVisible()

  hasReviews = false
  await page.reload()
  await expect(page.getByText("No reviews")).toBeVisible()
})

test("the theme choice persists across a reload", async ({ page }) => {
  await mockApi(page, (_method, path) => {
    if (path === "/api/auth/me") {
      return { body: authenticated }
    }
    if (path === "/api/setup/status") {
      return { body: setupComplete }
    }
    if (path === "/api/stats") {
      return { body: stats }
    }
    return null
  })

  await page.goto("/#/app/dashboard")
  await expect(page).toHaveURL(/#\/app\/dashboard/)

  await page.getByRole("button", { name: "Theme" }).click()
  await page.getByRole("menuitemradio", { name: "Dark" }).click()

  await expect(page.locator("html")).toHaveClass(/dark/)
  expect(
    await page.evaluate(() => localStorage.getItem("vite-ui-theme"))
  ).toBe("dark")

  await page.reload()

  await expect(page.locator("html")).toHaveClass(/dark/)
  expect(
    await page.evaluate(() => localStorage.getItem("vite-ui-theme"))
  ).toBe("dark")
})
