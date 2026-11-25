import { QueryClient } from "@tanstack/react-query"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { apiFetch, setUnauthorizedHandler } from "@/lib/api/client"
import { ApiError } from "@/lib/api/errors"
import { repoListSchema, summaryJsonSchema } from "@/lib/api/schemas"
import { clearSession } from "@/lib/auth/session"

const repo = {
  id: 1,
  github_id: 42,
  full_name: "acme/widgets",
  owner: "acme",
  name: "widgets",
  default_branch: "main",
  installation_id: 7,
  private: false,
  active: true,
  config_json: null,
  created_at: "2026-11-20T10:00:00Z",
  updated_at: "2026-11-20T10:00:00Z",
}

function stubFetch(response: Response) {
  const fetchMock = vi.fn().mockResolvedValue(response)
  vi.stubGlobal("fetch", fetchMock)
  return fetchMock
}

function jsonResponse(body: unknown, init: ResponseInit = {}) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
    ...init,
  })
}

beforeEach(() => {
  window.location.hash = ""
})

afterEach(() => {
  vi.useRealTimers()
  setUnauthorizedHandler(null)
  vi.unstubAllGlobals()
})

describe("apiFetch", () => {
  it("parses a 200 JSON body into the typed shape", async () => {
    stubFetch(jsonResponse([repo]))

    const repos = await apiFetch("/api/repos", undefined, repoListSchema)

    expect(repos[0].full_name).toBe("acme/widgets")
    expect(repos[0].default_branch).toBe("main")
  })

  it("does not send a JSON content type on a bodyless GET", async () => {
    const fetchMock = stubFetch(jsonResponse([]))

    await apiFetch("/api/repos", undefined, repoListSchema)

    const init = fetchMock.mock.calls[0][1] as RequestInit
    expect(new Headers(init.headers).get("Content-Type")).toBeNull()
  })

  it("sends credentials and a JSON content type when a body is present", async () => {
    const fetchMock = stubFetch(jsonResponse([]))

    await apiFetch("/api/repos", { method: "POST", body: "{}" }, repoListSchema)

    const init = fetchMock.mock.calls[0][1] as RequestInit
    expect(init.credentials).toBe("include")
    expect(new Headers(init.headers).get("Content-Type")).toBe("application/json")
  })

  it("honours a per-request timeout override without forwarding it", async () => {
    vi.useFakeTimers()
    const fetchMock = vi.fn(
      (_input: RequestInfo | URL, init?: RequestInit) =>
        new Promise<Response>((_resolve, reject) => {
          init?.signal?.addEventListener("abort", () => {
            reject(new DOMException("The operation was aborted.", "AbortError"))
          })
        })
    )
    vi.stubGlobal("fetch", fetchMock)

    const pending = apiFetch("/api/repos/sync", {
      method: "POST",
      timeoutMs: 120_000,
    }).catch((caught: unknown) => caught)

    await vi.advanceTimersByTimeAsync(30_000)
    const requestInit = fetchMock.mock.calls[0][1] as Record<string, unknown>
    expect("timeoutMs" in requestInit).toBe(false)
    expect((requestInit.signal as AbortSignal).aborted).toBe(false)

    await vi.advanceTimersByTimeAsync(90_000)
    const error = await pending
    expect(error).toBeInstanceOf(ApiError)
    expect((error as ApiError).status).toBe(0)
    expect((error as ApiError).message).toBe("The request timed out. Try again.")
  })

  it("rejects a 401, calls the global handler and clears the query cache", async () => {
    const client = new QueryClient()
    client.setQueryData(["repos"], [repo])
    const handler = vi.fn(() => clearSession(client))
    setUnauthorizedHandler(handler)
    stubFetch(jsonResponse({ error: "Authentication required" }, { status: 401 }))

    await expect(apiFetch("/api/repos", undefined, repoListSchema)).rejects.toMatchObject({
      status: 401,
      message: "Authentication required",
    })

    expect(handler).toHaveBeenCalledOnce()
    expect(client.getQueryCache().getAll()).toHaveLength(0)
    expect(window.location.hash).toBe("#/login")
  })

  it("does not redirect-loop for 401s from auth or setup endpoints", async () => {
    const handler = vi.fn()
    setUnauthorizedHandler(handler)
    stubFetch(jsonResponse({ error: "Authentication required" }, { status: 401 }))

    const error = await apiFetch("/api/auth/me").catch((caught: unknown) => caught)

    expect(error).toBeInstanceOf(ApiError)
    expect(handler).not.toHaveBeenCalled()
  })

  it("surfaces Retry-After on a 429", async () => {
    stubFetch(
      jsonResponse(
        { error: "Too many attempts. Try again in 15 minutes." },
        { status: 429, headers: { "Retry-After": "90" } }
      )
    )

    const error = await apiFetch("/api/auth/login", { method: "POST" }).catch(
      (caught: unknown) => caught
    )

    expect(error).toBeInstanceOf(ApiError)
    expect((error as ApiError).status).toBe(429)
    expect((error as ApiError).retryAfter).toBe(90)
  })
})

describe("summaryJsonSchema", () => {
  it("maps null and malformed strings to null", () => {
    expect(summaryJsonSchema.parse(null)).toBeNull()
    expect(summaryJsonSchema.parse("{not json")).toBeNull()
    expect(summaryJsonSchema.parse(undefined)).toBeNull()
  })

  it("parses a valid JSON string", () => {
    expect(summaryJsonSchema.parse('{"total":3}')).toEqual({ total: 3 })
    expect(summaryJsonSchema.parse("null")).toBeNull()
  })
})
