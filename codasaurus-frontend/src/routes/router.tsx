import { useQuery } from "@tanstack/react-query"
import type { ComponentType } from "react"
import { createHashRouter, Navigate, Outlet, useLocation, useRouteError } from "react-router"

import { ErrorState } from "@/components/common/error-state"
import { LoadingState } from "@/components/common/loading-state"
import { AppShell } from "@/components/layout/app-shell"
import { ErrorBoundary } from "@/components/layout/error-boundary"
import { NotFound } from "@/components/layout/not-found"
import { setupStatusQueryOptions } from "@/lib/api/queries"
import { useAuth } from "@/lib/auth/auth-context"

type RouteModule = { default: ComponentType }

function lazyRoute(loader: () => Promise<RouteModule>) {
  return {
    lazy: async () => ({ Component: (await loader()).default }),
  }
}

function PageLoading() {
  return (
    <div className="flex min-h-svh items-center justify-center p-6">
      <div className="w-full max-w-md">
        <LoadingState rows={4} />
      </div>
    </div>
  )
}

function RouteError() {
  const error = useRouteError()
  return (
    <div className="flex min-h-svh items-center justify-center p-6">
      <div className="w-full max-w-md">
        <ErrorState error={error} />
      </div>
    </div>
  )
}

function RootLayout() {
  return <Outlet />
}

function AuthOutage() {
  const { refresh } = useAuth()
  return (
    <div className="flex min-h-svh items-center justify-center p-6">
      <div className="w-full max-w-md">
        <ErrorState
          error="Cannot reach the server. Check your connection and try again."
          onRetry={() => void refresh()}
        />
      </div>
    </div>
  )
}

function RootRedirect() {
  const { status } = useAuth()
  const setup = useQuery(setupStatusQueryOptions)

  if (status === "loading") {
    return <PageLoading />
  }
  if (status === "error") {
    return <AuthOutage />
  }
  if (setup.isPending) {
    return <PageLoading />
  }
  if (setup.data && !setup.data.complete) {
    return <Navigate to="/setup" replace />
  }
  return (
    <Navigate
      to={status === "authenticated" ? "/app/dashboard" : "/login"}
      replace
    />
  )
}

function RequireAuth() {
  const { status } = useAuth()
  const location = useLocation()
  const setup = useQuery(setupStatusQueryOptions)

  if (status === "loading") {
    return <PageLoading />
  }
  if (status === "error") {
    return <AuthOutage />
  }
  if (setup.isPending) {
    return <PageLoading />
  }
  if (setup.data && !setup.data.complete) {
    return <Navigate to="/setup" replace />
  }
  if (status === "anonymous") {
    return (
      <Navigate
        to="/login"
        replace
        state={{ from: location.pathname + location.search }}
      />
    )
  }
  return (
    <AppShell>
      <ErrorBoundary key={location.pathname}>
        <Outlet />
      </ErrorBoundary>
    </AppShell>
  )
}

const router = createHashRouter([
  {
    element: <RootLayout />,
    errorElement: <RouteError />,
    children: [
      { index: true, element: <RootRedirect /> },
      { path: "login", ...lazyRoute(() => import("@/routes/login")) },
      { path: "invite/:token", ...lazyRoute(() => import("@/routes/invite")) },
      { path: "setup", ...lazyRoute(() => import("@/routes/setup")) },
      {
        path: "setup/github",
        ...lazyRoute(() => import("@/routes/setup-github")),
      },
      {
        path: "setup/complete",
        ...lazyRoute(() => import("@/routes/setup-complete")),
      },
      {
        path: "app",
        element: <RequireAuth />,
        children: [
          {
            path: "dashboard",
            ...lazyRoute(() => import("@/routes/dashboard")),
          },
          {
            path: "reviews",
            ...lazyRoute(() => import("@/routes/reviews")),
          },
          {
            path: "reviews/:id",
            ...lazyRoute(() => import("@/routes/review-detail")),
          },
          { path: "repos", ...lazyRoute(() => import("@/routes/repos")) },
          {
            path: "settings",
            ...lazyRoute(() => import("@/routes/settings")),
          },
          { path: "users", ...lazyRoute(() => import("@/routes/users")) },
          { path: "audit", ...lazyRoute(() => import("@/routes/audit")) },
          {
            path: "learning",
            ...lazyRoute(() => import("@/routes/learning")),
          },
          { path: "mcp", ...lazyRoute(() => import("@/routes/mcp")) },
        ],
      },
      { path: "*", element: <NotFound /> },
    ],
  },
])

export { RequireAuth, router }
