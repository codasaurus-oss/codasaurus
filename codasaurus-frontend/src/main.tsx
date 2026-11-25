import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClient } from '@tanstack/react-query'
import './index.css'
import { setUnauthorizedHandler } from '@/lib/api/client'
import { ApiError } from '@/lib/api/errors'
import { clearSession } from '@/lib/auth/session'
import { ThemeProvider } from '@/lib/theme/theme-provider'
import { Root } from '@/root'

const MAX_ATTEMPTS = 3

// Only transport failures and server faults are worth retrying; a rejected
// session or permission would be rejected again identically.
function shouldRetry(failureCount: number, error: unknown) {
  if (failureCount >= MAX_ATTEMPTS - 1) {
    return false
  }
  if (error instanceof ApiError) {
    return error.status === 0 || error.status >= 500
  }
  return true
}

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: shouldRetry,
    },
  },
})

setUnauthorizedHandler(() => clearSession(queryClient))

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <Root client={queryClient} />
    </ThemeProvider>
  </StrictMode>,
)
