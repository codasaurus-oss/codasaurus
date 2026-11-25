import { useQuery } from "@tanstack/react-query"
import { useState } from "react"
import { Navigate } from "react-router"

import { ErrorState } from "@/components/common/error-state"
import { LoadingState } from "@/components/common/loading-state"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { setupStatusQueryOptions } from "@/lib/api/queries"
import { apiFetch } from "@/lib/api/client"
import { syncResultSchema } from "@/lib/api/schemas"
import { useAuth } from "@/lib/auth/auth-context"
import { can } from "@/lib/auth/roles"

function SetupCompleteRoute() {
  const { status: authStatus, user } = useAuth()
  const setup = useQuery(setupStatusQueryOptions)
  const [message, setMessage] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function syncRepos() {
    setBusy(true)
    setError(null)
    setMessage(null)
    try {
      const result = await apiFetch(
        "/api/repos/sync",
        { method: "POST" },
        syncResultSchema
      )
      setMessage(
        `Synced ${result.synced} ${
          result.synced === 1 ? "repository" : "repositories"
        }`
      )
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Could not sync repos.")
    } finally {
      setBusy(false)
    }
  }

  if (setup.isPending) {
    return (
      <div className="flex min-h-svh items-center justify-center p-6">
        <div className="w-full max-w-md">
          <LoadingState rows={4} />
        </div>
      </div>
    )
  }
  if (setup.isError) {
    return (
      <div className="flex min-h-svh items-center justify-center p-6">
        <div className="w-full max-w-md">
          <ErrorState error={setup.error} onRetry={() => void setup.refetch()} />
        </div>
      </div>
    )
  }

  if (!setup.data.complete) {
    return <Navigate to="/setup" replace />
  }

  const authenticated = authStatus === "authenticated"
  const canSync = can(user?.role, "maintain")
  const installUrl = setup.data.github_install_url

  return (
    <div className="flex min-h-svh items-center justify-center p-6">
      <div className="w-full max-w-md">
        <Card>
          <CardHeader>
            <CardTitle>Setup complete</CardTitle>
            <CardDescription>
              {authStatus === "loading"
                ? "Setup is complete. Checking your session…"
                : authenticated
                  ? "Install the GitHub App and sync repositories to start reviewing pull requests."
                  : "The owner account is created. Creating it does not sign you in, so sign in to open the dashboard."}
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-3">
            {message ? (
              <Alert>
                <AlertTitle>Repositories</AlertTitle>
                <AlertDescription>{message}</AlertDescription>
              </Alert>
            ) : null}
            {error ? (
              <Alert variant="destructive">
                <AlertTitle>Could not sync repositories</AlertTitle>
                <AlertDescription>{error}</AlertDescription>
              </Alert>
            ) : null}
          </CardContent>
          <CardFooter className="flex flex-wrap gap-2">
            {authStatus === "loading" ? null : authenticated ? (
              <>
                {canSync ? (
                  <Button
                    variant="outline"
                    onClick={() => void syncRepos()}
                    disabled={busy}
                  >
                    {busy ? "Syncing…" : "Sync repositories"}
                  </Button>
                ) : null}
                <Button render={<a href="#/app/dashboard" />}>
                  Open dashboard
                </Button>
              </>
            ) : (
              <Button render={<a href="#/login" />}>Sign in</Button>
            )}
            {installUrl ? (
              <Button
                variant="outline"
                render={
                  <a href={installUrl} target="_blank" rel="noreferrer" />
                }
              >
                Install the GitHub App
              </Button>
            ) : null}
          </CardFooter>
        </Card>
      </div>
    </div>
  )
}

export default SetupCompleteRoute
