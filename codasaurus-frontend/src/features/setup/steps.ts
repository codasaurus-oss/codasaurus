import type { SetupStatus } from "@/lib/api/schemas"

const llmStep = {
  id: "llm",
  title: "Inference",
  description: "Configure models for code reviews and repository search.",
} as const

const githubStep = {
  id: "github",
  title: "GitHub App",
  description: "Register the app that receives pull request webhooks.",
} as const

const adminStep = {
  id: "admin",
  title: "Admin",
  description: "Create the first owner account for this instance.",
} as const

const setupSteps = [llmStep, githubStep, adminStep] as const

type SetupStep = (typeof setupSteps)[number]
type SetupStepId = SetupStep["id"]
type SetupStepProps = { complete: boolean; onComplete: () => void }

function completedCount(status: SetupStatus) {
  return setupSteps.filter((step) => status[step.id]).length
}

function firstIncompleteStep(status: SetupStatus): SetupStepId | null {
  return setupSteps.find((step) => !status[step.id])?.id ?? null
}

// Steps unlock in order, so a later step opens only after every earlier one.
function isStepUnlocked(status: SetupStatus, id: SetupStepId) {
  const index = setupSteps.findIndex((step) => step.id === id)
  return setupSteps
    .slice(0, index)
    .every((step) => status[step.id])
}

export {
  adminStep,
  completedCount,
  firstIncompleteStep,
  githubStep,
  isStepUnlocked,
  llmStep,
  setupSteps,
}
export type { SetupStep, SetupStepId, SetupStepProps }
