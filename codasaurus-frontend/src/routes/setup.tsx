import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useEffect, useState } from "react"
import { Navigate, useNavigate } from "react-router"

import { ErrorState } from "@/components/common/error-state"
import { LoadingState } from "@/components/common/loading-state"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ButtonGroup } from "@/components/ui/button-group"
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  Progress,
  ProgressLabel,
  ProgressValue,
} from "@/components/ui/progress"
import { AdminStep } from "@/features/setup/steps/admin-step"
import { GithubStep } from "@/features/setup/steps/github-step"
import { LlmStep } from "@/features/setup/steps/llm-step"
import {
  completedCount,
  firstIncompleteStep,
  isStepUnlocked,
  setupSteps,
  type SetupStepId,
} from "@/features/setup/steps"
import { queryKeys, setupStatusQueryOptions } from "@/lib/api/queries"
import type { SetupStatus } from "@/lib/api/schemas"

function SetupRoute() {
  const client = useQueryClient()
  const navigate = useNavigate()
  const setup = useQuery(setupStatusQueryOptions)
  const [selected, setSelected] = useState<SetupStepId | null>(null)
  const [finishing, setFinishing] = useState(false)

  // The flag only covers the hand-off to /setup/complete. A status that comes
  // back incomplete means the hand-off did not stick, so the next completion
  // must be able to redirect again.
  useEffect(() => {
    if (!setup.data?.complete) {
      setFinishing(false)
    }
  }, [setup.data?.complete])

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
          <ErrorState
            error={setup.error}
            onRetry={() => void setup.refetch()}
          />
        </div>
      </div>
    )
  }

  const status = setup.data
  if (status.complete && !finishing) {
    return <Navigate to="/app/dashboard" replace />
  }

  const activeId =
    selected && isStepUnlocked(status, selected)
      ? selected
      : firstIncompleteStep(status)

  function refresh() {
    setSelected(null)
    void client.invalidateQueries({ queryKey: queryKeys.setupStatus })
  }

  // The cached status has to read complete before /setup/complete mounts,
  // otherwise its guard bounces straight back here. `finishing` suppresses this
  // route's own dashboard redirect during that hand-off.
  function finish() {
    setFinishing(true)
    client.setQueryData<SetupStatus>(queryKeys.setupStatus, (previous) =>
      previous ? { ...previous, admin: true, complete: true } : previous
    )
    void navigate("/setup/complete")
    void client.invalidateQueries({ queryKey: queryKeys.setupStatus })
  }

  // Hold the GitHub step open: once github turns true the wizard would jump to
  // Admin and skip the install link the user needs before the owner exists.
  function holdGithubStep() {
    setSelected("github")
    void client.invalidateQueries({ queryKey: queryKeys.setupStatus })
  }

  function activeStep() {
    switch (activeId) {
      case "llm":
        return <LlmStep complete={status.llm} onComplete={refresh} />
      case "github":
        return (
          <GithubStep
            complete={status.github}
            installUrl={status.github_install_url}
            publicUrl={status.public_url}
            onComplete={refresh}
            onSaved={holdGithubStep}
          />
        )
      case "admin":
        return (
          <AdminStep
            complete={status.admin}
            firstOwner={!status.admin}
            onComplete={finish}
          />
        )
      default:
        return null
    }
  }

  const activeIndex = setupSteps.findIndex((step) => step.id === activeId)
  const currentStep = setupSteps[activeIndex] ?? setupSteps[0]
  const previousStep = setupSteps[activeIndex - 1]
  const nextStep = setupSteps[activeIndex + 1]

  return (
    <div className="flex min-h-svh justify-center p-6 md:p-10">
      <div className="flex w-full max-w-xl flex-col gap-6">
        <div className="flex items-center gap-2 font-medium">
          <img
            src="/codasaurus-logo.svg"
            alt=""
            className="size-7 rounded-lg"
          />
          Codasaurus setup
        </div>

        <Card>
          <CardHeader>
            <CardTitle>First-run setup</CardTitle>
            <CardDescription>
              Finish each step to open the dashboard.
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-4">
            <Progress
              value={(completedCount(status) / setupSteps.length) * 100}
            >
              <ProgressLabel>{`Step ${activeIndex + 1} of ${setupSteps.length}`}</ProgressLabel>
              <ProgressValue>
                {() => `${completedCount(status)} of ${setupSteps.length} complete`}
              </ProgressValue>
            </Progress>
            <div className="flex flex-col gap-1">
              {setupSteps.map((step, index) => {
                const complete = status[step.id]
                const unlocked = isStepUnlocked(status, step.id)
                return (
                  <Button
                    key={step.id}
                    variant={activeId === step.id ? "secondary" : "ghost"}
                    className="justify-start"
                    disabled={!unlocked}
                    onClick={() => setSelected(step.id)}
                  >
                    <span className="text-muted-foreground tabular-nums">
                      {index + 1}.
                    </span>
                    {step.title}
                    <Badge
                      className="ml-auto"
                      variant={
                        complete
                          ? "secondary"
                          : unlocked
                            ? "outline"
                            : "ghost"
                      }
                    >
                      {complete ? "Done" : unlocked ? "Required" : "Locked"}
                    </Badge>
                  </Button>
                )
              })}
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              {currentStep.title}
              <Badge variant={status[currentStep.id] ? "secondary" : "outline"}>
                {status[currentStep.id] ? "Done" : "Required"}
              </Badge>
            </CardTitle>
            <CardDescription>{currentStep.description}</CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-4">{activeStep()}</CardContent>
          <CardFooter>
            <ButtonGroup className="w-full">
              <Button
                variant="outline"
                className="flex-1"
                disabled={!previousStep}
                onClick={() => previousStep && setSelected(previousStep.id)}
              >
                Back
              </Button>
              <Button
                className="flex-1"
                disabled={!nextStep || !activeId || !status[activeId]}
                onClick={() => nextStep && setSelected(nextStep.id)}
              >
                Next
              </Button>
            </ButtonGroup>
          </CardFooter>
        </Card>
      </div>
    </div>
  )
}

export default SetupRoute
