import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react"

import { apiFetch } from "@/lib/api/client"
import { ApiError } from "@/lib/api/errors"
import { meResponseSchema, type UserInfo } from "@/lib/api/schemas"

type AuthStatus = "loading" | "authenticated" | "anonymous" | "error"

type AuthContextValue = {
  user: UserInfo | null
  status: AuthStatus
  refresh: (silent?: boolean) => Promise<void>
  setUser: (user: UserInfo | null) => void
}

const AuthContext = createContext<AuthContextValue | undefined>(undefined)

// Only transport failures and server faults mean the session is unknown; a
// rejection means the session is genuinely absent.
function isOutage(error: unknown) {
  if (error instanceof TypeError) {
    return true
  }
  return error instanceof ApiError && (error.status === 0 || error.status >= 500)
}

function AuthProvider({ children }: { children: ReactNode }) {
  const [user, setUserState] = useState<UserInfo | null>(null)
  const [status, setStatus] = useState<AuthStatus>("loading")
  const userRef = useRef<UserInfo | null>(null)

  const setUser = useCallback((next: UserInfo | null) => {
    userRef.current = next
    setUserState(next)
    setStatus(next ? "authenticated" : "anonymous")
  }, [])

  const refresh = useCallback(
    async (silent = false) => {
      // A silent refresh of a signed-in user must not unmount the app behind
      // the guards while it is in flight.
      const quiet = silent && userRef.current !== null
      if (!quiet) {
        setStatus("loading")
      }
      try {
        const me = await apiFetch("/api/auth/me", undefined, meResponseSchema)
        setUser(me.authenticated ? me.user : null)
      } catch (error) {
        if (!isOutage(error)) {
          setUser(null)
          return
        }
        if (quiet) {
          return
        }
        // An outage is not a logout: keep the session unknown so the guards can
        // offer a retry instead of bouncing the user to the login screen.
        userRef.current = null
        setUserState(null)
        setStatus("error")
      }
    },
    [setUser]
  )

  useEffect(() => {
    void refresh()
  }, [refresh])

  const value = useMemo(
    () => ({ user, status, refresh, setUser }),
    [user, status, refresh, setUser]
  )

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>
}

function useAuth() {
  const context = useContext(AuthContext)
  if (!context) {
    throw new Error("useAuth must be used within an AuthProvider")
  }
  return context
}

export { AuthProvider, useAuth }
export type { AuthStatus }
