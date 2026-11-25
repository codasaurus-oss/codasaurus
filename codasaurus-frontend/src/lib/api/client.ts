import { ApiError } from "@/lib/api/errors"

const TIMEOUT_MS = 30_000

type Schema<T> = { parse: (value: unknown) => T }

type ApiInit = RequestInit & { timeoutMs?: number }

let unauthorizedHandler: (() => void) | null = null

function setUnauthorizedHandler(handler: (() => void) | null) {
  unauthorizedHandler = handler
}

// /api/auth/* and /api/setup/* answer 401 as part of their own flow; the
// global redirect would loop between the login screen and itself.
function isSessionProbePath(path: string) {
  return path.startsWith("/api/auth/") || path.startsWith("/api/setup/")
}

async function readBody(response: Response): Promise<unknown> {
  const text = await response.text()
  if (!text) {
    return null
  }
  try {
    return JSON.parse(text)
  } catch {
    return null
  }
}

function errorMessage(body: unknown, response: Response) {
  const envelope =
    typeof body === "object" && body !== null && "error" in body
      ? body.error
      : undefined
  if (typeof envelope === "string") {
    return envelope
  }
  return response.statusText || `Request failed with status ${response.status}`
}

function retryAfterSeconds(response: Response) {
  const raw = response.headers.get("Retry-After")
  if (!raw) {
    return undefined
  }
  const seconds = Number(raw)
  return Number.isFinite(seconds) ? seconds : undefined
}

async function apiFetch<T>(
  path: string,
  init?: ApiInit,
  schema?: Schema<T>
): Promise<T> {
  const { timeoutMs = TIMEOUT_MS, ...requestInit } = init ?? {}
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(), timeoutMs)
  const headers = new Headers(requestInit.headers)
  if (requestInit.body !== undefined && requestInit.body !== null) {
    headers.set("Content-Type", "application/json")
  }

  let response: Response
  try {
    response = await fetch(path, {
      ...requestInit,
      credentials: "include",
      headers,
      signal: controller.signal,
    })
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") {
      throw new ApiError(0, "The request timed out. Try again.")
    }
    throw error
  } finally {
    clearTimeout(timeout)
  }

  const body = await readBody(response)

  if (!response.ok) {
    if (response.status === 401 && !isSessionProbePath(path)) {
      unauthorizedHandler?.()
    }
    throw new ApiError(
      response.status,
      errorMessage(body, response),
      retryAfterSeconds(response)
    )
  }

  return schema ? schema.parse(body) : (body as T)
}

export { apiFetch, setUnauthorizedHandler }
