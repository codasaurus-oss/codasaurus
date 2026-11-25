import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useEffect } from "react"

import { ErrorState } from "@/components/common/error-state"
import { LoadingState } from "@/components/common/loading-state"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { queryKeys, setupStatusQueryOptions } from "@/lib/api/queries"

function SetupGithubRoute() {
  const client = useQueryClient()
  const setup = useQuery(setupStatusQueryOptions)

  // The manifest callback saves credentials on the server, so drop the cached
  // status from before the round trip.
  useEffect(() => {
    void client.invalidateQueries({ queryKey: queryKeys.setupStatus })
  }, [client])

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

  const connected = setup.data.github

  return (
    <div className="flex min-h-svh items-center justify-center p-6">
      <div className="w-full max-w-md">
        <Card>
          <CardHeader>
            <CardTitle>GitHub App</CardTitle>
            <CardDescription>
              {connected
                ? "GitHub confirmed the app and its credentials are saved."
                : "GitHub has not confirmed the app yet."}
            </CardDescription>
          </CardHeader>
          <CardContent>
            <p className="text-sm text-muted-foreground">
              {connected
                ? "Return to the wizard to create the owner account."
                : "Return to the wizard and try creating the app again, or enter its credentials manually."}
            </p>
          </CardContent>
          <CardFooter>
            <Button render={<a href="#/setup" />}>Back to setup</Button>
          </CardFooter>
        </Card>
      </div>
    </div>
  )
}

export default SetupGithubRoute
