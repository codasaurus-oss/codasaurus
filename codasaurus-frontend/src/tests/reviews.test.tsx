import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react"
import { MemoryRouter, Route, Routes, useNavigate } from "react-router"
import { afterEach, describe, expect, it, vi } from "vitest"

import { AuthProvider } from "@/lib/auth/auth-context"
import {
  queryKeys,
  reviewQueryOptions,
  reviewsQueryOptions,
} from "@/lib/api/queries"
import ReviewDetailRoute from "@/routes/review-detail"
import ReviewsRoute from "@/routes/reviews"

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

const findings = [
  {
    id: 1,
    review_id: 42,
    fingerprint: "fp-1234567890",
    file_path: "src/auth.rs",
    line_start: 10,
    line_end: 12,
    column_start: null,
    column_end: null,
    severity: "blocking",
    detector: "secrets",
    rule_id: "hardcoded-key",
    message: "Hardcoded API key",
    suggested_fix: "Read the key from the environment",
    code_snippet: 'let key = "sk-live";',
    context: null,
    category: null,
    confidence: 92,
    judge_rationale: "The literal matches a live key prefix.",
    created_at: "2026-11-23T10:00:20Z",
  },
  {
    id: 2,
    review_id: 42,
    fingerprint: null,
    file_path: "src/db.rs",
    line_start: 4,
    line_end: 4,
    column_start: null,
    column_end: null,
    severity: "warning",
    detector: "vulnerabilities",
    rule_id: "sql-injection",
    message: "Query built from user input",
    suggested_fix: null,
    code_snippet: null,
    context: null,
    category: null,
    confidence: null,
    judge_rationale: null,
    created_at: "2026-11-23T10:00:21Z",
  },
]

function detail(status = "passed", overrides: Record<string, unknown> = {}) {
  return {
    review: { ...review, status, ...overrides },
    findings,
    summary: {
      finding_count: findings.length,
      file_count: 2,
      by_severity: { blocking: 1, warning: 1 },
      by_detector: { secrets: 1, vulnerabilities: 1 },
      duration_secs: 42,
    },
  }
}

function findingsResponse(items = findings) {
  const byFile: Record<string, typeof findings> = {}
  for (const finding of items) {
    const existing = byFile[finding.file_path]
    if (existing) {
      existing.push(finding)
    } else {
      byFile[finding.file_path] = [finding]
    }
  }
  return {
    review_id: 42,
    findings_by_file: byFile,
    total_findings: items.length,
  }
}

function jsonResponse(body: unknown, init: ResponseInit = {}) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
    ...init,
  })
}

function stubApi(
  options: {
    role?: string
    detail?: object
    findings?: object
    reviews?: object
  } = {}
) {
  const fetchMock = vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = init?.method ?? "GET"
    if (url === "/api/auth/me") {
      return Promise.resolve(
        jsonResponse({
          authenticated: true,
          user: {
            email: `${options.role ?? "owner"}@example.com`,
            role: options.role ?? "owner",
            auth_provider: "local",
            is_bootstrap: false,
          },
        })
      )
    }
    if (url === "/api/repos") {
      return Promise.resolve(jsonResponse([repo]))
    }
    if (url === "/api/reviews/dismiss" && method === "POST") {
      return Promise.resolve(jsonResponse({ status: "ok" }))
    }
    if (url === "/api/reviews/42/findings") {
      return Promise.resolve(
        jsonResponse(options.findings ?? findingsResponse())
      )
    }
    if (url === "/api/reviews/42") {
      return Promise.resolve(jsonResponse(options.detail ?? detail()))
    }
    if (url === "/api/reviews/43/findings") {
      return Promise.resolve(jsonResponse(findingsResponse()))
    }
    if (url === "/api/reviews/43") {
      return Promise.resolve(
        jsonResponse(detail("passed", { id: 43, pr_number: 8 }))
      )
    }
    if (url.startsWith("/api/reviews?")) {
      if (options.reviews) {
        return Promise.resolve(jsonResponse(options.reviews))
      }
      const offset = Number(new URLSearchParams(url.split("?")[1]).get("offset"))
      const secondPage = offset === 20
      return Promise.resolve(
        jsonResponse({
          reviews: [
            {
              ...review,
              id: secondPage ? 43 : 42,
              pr_number: secondPage ? 8 : 7,
            },
          ],
          total: 2,
          total_pages: 2,
          limit: 20,
          offset,
        })
      )
    }
    return Promise.resolve(
      jsonResponse(
        { error: `Unmatched request: ${method} ${url}` },
        { status: 500 }
      )
    )
  })
  vi.stubGlobal("fetch", fetchMock)
  return fetchMock
}

function newClient() {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } })
}

function lastReviewsUrl(fetchMock: ReturnType<typeof stubApi>) {
  const urls = fetchMock.mock.calls
    .map(([input]) => String(input))
    .filter((url) => url.startsWith("/api/reviews?"))
  return urls.at(-1) ?? ""
}

function renderReviews(client = newClient()) {
  render(
    <QueryClientProvider client={client}>
      <AuthProvider>
        <MemoryRouter>
          <ReviewsRoute />
        </MemoryRouter>
      </AuthProvider>
    </QueryClientProvider>
  )
  return client
}

function renderDetail(client = newClient(), path = "/app/reviews/42") {
  render(
    <QueryClientProvider client={client}>
      <AuthProvider>
        <MemoryRouter initialEntries={[path]}>
          <Routes>
            <Route path="/app/reviews/:id" element={<ReviewDetailRoute />} />
          </Routes>
        </MemoryRouter>
      </AuthProvider>
    </QueryClientProvider>
  )
  return client
}

function renderDetailNavigable(client = newClient(), path = "/app/reviews/42") {
  const navigateRef: { current: ((to: string) => void) | null } = {
    current: null,
  }
  function CaptureNavigate() {
    navigateRef.current = useNavigate()
    return null
  }
  render(
    <QueryClientProvider client={client}>
      <AuthProvider>
        <MemoryRouter initialEntries={[path]}>
          <CaptureNavigate />
          <Routes>
            <Route path="/app/reviews/:id" element={<ReviewDetailRoute />} />
          </Routes>
        </MemoryRouter>
      </AuthProvider>
    </QueryClientProvider>
  )
  return (to: string) => navigateRef.current?.(to)
}

async function chooseFilter(trigger: string, option: string) {
  fireEvent.mouseDown(screen.getByRole("combobox", { name: trigger }))
  const item = await screen.findByRole("option", { name: option })
  fireEvent.pointerDown(item)
  fireEvent.click(item)
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  vi.unstubAllGlobals()
})

describe("reviews list", () => {
  it("builds the list query from repo, status, limit and offset", async () => {
    const fetchMock = stubApi()
    const client = newClient()

    await client.fetchQuery(
      reviewsQueryOptions({ repoId: 3, status: "failed", limit: 20, offset: 40 })
    )

    expect(String(fetchMock.mock.calls.at(-1)?.[0])).toBe(
      "/api/reviews?limit=20&offset=40&repo_id=3&status=failed"
    )
  })

  it("requests the next page with the right offset", async () => {
    const fetchMock = stubApi()
    renderReviews()

    expect(await screen.findByRole("link", { name: "#7" })).toBeInTheDocument()
    fireEvent.click(screen.getByLabelText("Go to next page"))

    await waitFor(() => {
      const next = fetchMock.mock.calls.find(([input]) =>
        String(input).includes("offset=20")
      )
      expect(next).toBeDefined()
    })
    expect(await screen.findByRole("link", { name: "#8" })).toBeInTheDocument()
  })

  it("shows an empty state when no reviews exist", async () => {
    stubApi({
      reviews: { reviews: [], total: 0, total_pages: 0, limit: 20, offset: 0 },
    })
    renderReviews()

    expect(await screen.findByText("No reviews")).toBeInTheDocument()
  })

  it("drives the repo and status filters into the query", async () => {
    const fetchMock = stubApi()
    renderReviews()

    expect(await screen.findByRole("link", { name: "#7" })).toBeInTheDocument()

    await chooseFilter("Filter by repository", "acme/web")
    await waitFor(() => {
      expect(lastReviewsUrl(fetchMock)).toContain("repo_id=1")
    })

    await chooseFilter("Filter by status", "failed")
    await waitFor(() => {
      const url = lastReviewsUrl(fetchMock)
      expect(url).toContain("repo_id=1")
      expect(url).toContain("status=failed")
      expect(url).toContain("offset=0")
    })
  })

  it("distinguishes no reviews from no matches and clears filters", async () => {
    stubApi({
      reviews: { reviews: [], total: 0, total_pages: 0, limit: 20, offset: 0 },
    })
    renderReviews()

    expect(await screen.findByText("No reviews")).toBeInTheDocument()

    await chooseFilter("Filter by repository", "acme/web")
    expect(await screen.findByText("No matching reviews")).toBeInTheDocument()

    fireEvent.click(screen.getByRole("button", { name: "Clear filters" }))
    expect(await screen.findByText("No reviews")).toBeInTheDocument()
  })
})

describe("review detail", () => {
  it("polls pending and running reviews every ten seconds, then stops", () => {
    const refetchInterval = reviewQueryOptions(42).refetchInterval
    if (typeof refetchInterval !== "function") {
      throw new Error("expected a polling function")
    }
    const intervalFor = (status: string) =>
      refetchInterval({ state: { data: detail(status) } } as never)

    expect(intervalFor("pending")).toBe(10_000)
    expect(intervalFor("running")).toBe(10_000)
    expect(intervalFor("passed")).toBe(false)
  })

  it("renders a null summary_json without crashing", async () => {
    stubApi({ detail: detail("passed", { summary_json: null }) })
    renderDetail()

    expect(
      await screen.findByRole("heading", { name: "Findings" })
    ).toBeInTheDocument()
  })

  it("renders a malformed summary_json without crashing", async () => {
    stubApi({ detail: detail("passed", { summary_json: "{not json" }) })
    renderDetail()

    expect(
      await screen.findByRole("heading", { name: "Findings" })
    ).toBeInTheDocument()
  })

  it("renders the server findings grouped by file", async () => {
    stubApi()
    renderDetail()

    expect(await screen.findByText("src/auth.rs")).toBeInTheDocument()
    expect(screen.getByText("src/db.rs")).toBeInTheDocument()
    expect(screen.getByText("Hardcoded API key")).toBeInTheDocument()
    expect(screen.queryByText("Query built from user input")).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: /src\/db\.rs/ }))
    expect(
      await screen.findByText("Query built from user input")
    ).toBeInTheDocument()
    expect(screen.getByText(/Suggested fix:/)).toBeInTheDocument()
    expect(screen.getByText(/Rationale:/)).toBeInTheDocument()
  })

  it("shows an empty state when the review has no findings", async () => {
    stubApi({
      findings: { review_id: 42, findings_by_file: {}, total_findings: 0 },
    })
    renderDetail()

    expect(await screen.findByText("No findings")).toBeInTheDocument()
  })

  it("shows a note instead of an empty container when findings are ungrouped", async () => {
    stubApi({
      detail: detail("passed"),
      findings: { review_id: 42, findings_by_file: {}, total_findings: 3 },
    })
    renderDetail()

    expect(
      await screen.findByText(
        "This review reports findings, but none are available to display."
      )
    ).toBeInTheDocument()
    expect(
      document.querySelector('[data-slot="collapsible"]')
    ).toBeNull()
  })

  it("shows a skeleton while a review is running", async () => {
    stubApi({ detail: detail("running") })
    renderDetail()

    expect(await screen.findByText(/Review in progress/)).toBeInTheDocument()
    expect(document.querySelector('[data-slot="skeleton"]')).not.toBeNull()
  })

  it("links to the SARIF export", async () => {
    stubApi()
    renderDetail()

    const link = await screen.findByRole("link", { name: "Download SARIF" })
    expect(link).toHaveAttribute("href", "/api/reviews/42/sarif")
    expect(link).toHaveAttribute("download", "review-42.sarif")
  })

  it("hides dismiss from a viewer", async () => {
    stubApi({ role: "viewer" })
    renderDetail()

    expect(await screen.findByText("Hardcoded API key")).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: /^Dismiss/ })).toBeNull()
  })

  it("disables dismiss when the finding has no fingerprint", async () => {
    stubApi({ role: "maintainer" })
    renderDetail()

    fireEvent.click(await screen.findByRole("button", { name: /src\/db\.rs/ }))
    const button = await screen.findByRole("button", {
      name: "Dismiss sql-injection",
    })
    expect(button).toBeDisabled()
    expect(button).toHaveAttribute(
      "title",
      "This finding has no fingerprint, so it cannot be dismissed."
    )
  })

  it("sends the finding, message and invalidates review queries", async () => {
    const fetchMock = stubApi({ role: "maintainer" })
    const client = renderDetail()
    const invalidate = vi.spyOn(client, "invalidateQueries")

    fireEvent.click(
      await screen.findByRole("button", { name: "Dismiss hardcoded-key" })
    )

    fireEvent.change(await screen.findByLabelText(/Message/), {
      target: { value: "Test fixture key" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Confirm" }))

    await waitFor(() => {
      const post = fetchMock.mock.calls.find(
        ([input, init]) =>
          String(input) === "/api/reviews/dismiss" && init?.method === "POST"
      )
      expect(post).toBeDefined()
      expect(JSON.parse(String(post?.[1]?.body))).toEqual({
        fingerprint: "fp-1234567890",
        detector: "secrets",
        file: "src/auth.rs",
        pr_number: 7,
        repo: "acme/web",
        message: "Test fixture key",
      })
    })

    await waitFor(() => {
      expect(invalidate).toHaveBeenCalledWith({
        queryKey: queryKeys.review(42),
      })
      expect(invalidate).toHaveBeenCalledWith({
        queryKey: queryKeys.reviewFindings(42),
      })
      expect(invalidate).toHaveBeenCalledWith({ queryKey: queryKeys.reviews })
      expect(invalidate).toHaveBeenCalledWith({ queryKey: queryKeys.stats })
    })

    expect(
      await screen.findByRole("button", { name: "Dismiss hardcoded-key" })
    ).toBeDisabled()
    expect(screen.getByText("Dismissed")).toBeInTheDocument()
  })

  it("clears dismissed state when navigating to another review", async () => {
    stubApi({ role: "maintainer" })
    const client = newClient()
    client.setQueryData(queryKeys.review(43), detail("passed", { id: 43, pr_number: 8 }))
    client.setQueryData(queryKeys.reviewFindings(43), findingsResponse())
    const navigate = renderDetailNavigable(client)

    fireEvent.click(
      await screen.findByRole("button", { name: "Dismiss hardcoded-key" })
    )
    fireEvent.click(await screen.findByRole("button", { name: "Confirm" }))

    expect(await screen.findByText("Dismissed")).toBeInTheDocument()

    navigate("/app/reviews/43")

    expect(
      await screen.findByRole("heading", { name: "#8 Harden auth" })
    ).toBeInTheDocument()
    await waitFor(() => {
      expect(screen.queryByText("Dismissed")).toBeNull()
    })
    expect(
      screen.getByRole("button", { name: "Dismiss hardcoded-key" })
    ).toBeEnabled()
  })

  it.each(["abc", "-1", "1.5", "99999999999999999999"])(
    "renders not found for the unsafe review id %s without calling the API",
    (value) => {
      const fetchMock = stubApi()
      renderDetail(newClient(), `/app/reviews/${value}`)

      expect(screen.getByText("Page not found")).toBeInTheDocument()
      expect(
        fetchMock.mock.calls.some(([input]) =>
          String(input).startsWith("/api/reviews/")
        )
      ).toBe(false)
    }
  )
})
