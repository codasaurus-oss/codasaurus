import { useQuery } from "@tanstack/react-query"
import { useLocation } from "react-router"

import { LoginForm } from "@/components/login-form"
import { oidcStatusQueryOptions } from "@/lib/api/queries"

const DEFAULT_ROUTE = "/app/dashboard"

// RequireAuth hands the blocked route over in `state.from` so a return trip
// after sign-in lands where the user was headed.
function intendedRoute(state: unknown) {
  if (typeof state === "object" && state !== null && "from" in state) {
    const from = (state as { from?: unknown }).from
    if (typeof from === "string" && from.startsWith("/")) {
      return from
    }
  }
  return DEFAULT_ROUTE
}

function LoginRoute() {
  const location = useLocation()
  const oidc = useQuery(oidcStatusQueryOptions)

  return (
    <div className="grid min-h-svh lg:grid-cols-2">
      <div className="flex flex-col gap-6 p-6 md:p-10">
        <a href="#/" className="flex w-fit items-center gap-2 font-medium">
          <img src="/codasaurus-logo.svg" alt="" className="size-7 rounded-lg" />
          Codasaurus
        </a>
        <div className="flex flex-1 items-center justify-center">
          <div className="w-full max-w-sm">
            <LoginForm
              from={intendedRoute(location.state)}
              oidcEnabled={oidc.data?.enabled ?? false}
            />
          </div>
        </div>
      </div>
      <div className="relative hidden flex-col justify-between overflow-hidden bg-muted/40 p-10 lg:flex">
        <img
          src="/codasaurus-logo.svg"
          alt=""
          className="size-10 rounded-xl"
        />
        <div className="flex flex-col gap-2">
          <p className="font-heading text-2xl leading-snug font-medium">
            Security review for every pull request.
          </p>
          <p className="text-sm text-muted-foreground">
            Codasaurus flags risky changes before they reach main.
          </p>
        </div>
      </div>
    </div>
  )
}

export default LoginRoute
