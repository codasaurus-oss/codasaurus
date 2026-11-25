import type { QueryClient } from "@tanstack/react-query"

function clearSession(client: QueryClient) {
  client.clear()
  if (window.location.hash !== "#/login") {
    window.location.hash = "#/login"
  }
}

export { clearSession }
