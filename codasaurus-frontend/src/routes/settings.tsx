import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { ChevronsUpDownIcon, TriangleAlertIcon } from "lucide-react"
import { useState, type ReactNode } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/common/confirm-dialog"
import { ErrorState } from "@/components/common/error-state"
import { LoadingState } from "@/components/common/loading-state"
import { StatusBadge } from "@/components/common/status-badge"
import { PageHeader } from "@/components/layout/page-header"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from "@/components/ui/accordion"
import { Badge } from "@/components/ui/badge"
import { Button, buttonVariants } from "@/components/ui/button"
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command"
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { SecretInput } from "@/features/setup/secret-field"
import { SettingField } from "@/features/settings/setting-field"
import { ApiError } from "@/lib/api/errors"
import { apiFetch } from "@/lib/api/client"
import { withCurrent } from "@/lib/model-options"
import {
  githubSettingsQueryOptions,
  llmConfigQueryOptions,
  queryKeys,
  settingsQueryOptions,
} from "@/lib/api/queries"
import {
  integrationTestSchema,
  settingsActionResultSchema,
  type SettingsMap,
} from "@/lib/api/schemas"
import { useAuth } from "@/lib/auth/auth-context"
import { can } from "@/lib/auth/roles"

type TabId = "general" | "github" | "llm" | "oidc" | "tickets" | "advanced"

type TabMeta = {
  label: string
  description: string
  fields: Array<[key: string, label: string]>
}

// Mirrors ALLOWED_KEYS in the backend settings API; keys absent here are read
// only because PUT /api/settings/{key} rejects anything outside that whitelist.
const TABS: Record<TabId, TabMeta> = {
  general: {
    label: "General",
    description: "Instance identity and the offline kill-switch.",
    fields: [
      ["public_url", "Public URL"],
      ["offline_mode", "Mode"],
    ],
  },
  github: {
    label: "GitHub",
    description: "GitHub App registration and connection.",
    fields: [],
  },
  llm: {
    label: "LLM",
    description: "Gateway provider, credential and model aliases.",
    fields: [["llm_daily_budget_usd", "Daily budget (USD)"]],
  },
  oidc: {
    label: "OIDC",
    description: "Single sign-on through an OpenID Connect provider.",
    fields: [
      ["oidc_issuer", "Issuer URL"],
      ["oidc_client_id", "Client ID"],
      ["oidc_client_secret", "Client secret"],
      ["oidc_redirect_uri", "Redirect URI"],
      ["oidc_scopes", "Scopes"],
      ["oidc_allow_open_join", "Allow open join"],
      ["oidc_allow_unverified_email", "Allow unverified email"],
      ["oidc_allow_public_client", "Allow public client"],
    ],
  },
  tickets: {
    label: "Tickets",
    description: "Jira and Linear credentials for ticket workflows.",
    fields: [
      ["jira_base_url", "Jira base URL"],
      ["jira_email", "Jira email"],
      ["jira_api_token", "Jira API token"],
      ["linear_api_key", "Linear API key"],
    ],
  },
  advanced: {
    label: "Advanced",
    description: "Review policy, detector toggles and runtime limits.",
    fields: [
      ["default_severity", "Default severity"],
      ["review_strictness", "Review strictness"],
      ["max_blocking", "Max blocking findings"],
      ["max_warnings", "Max warnings"],
      ["forbidden_paths", "Forbidden paths"],
      ["request_reviewers", "Request reviewers"],
      ["create_check_run", "Create check run"],
      ["hallucinated_imports_enabled", "Hallucinated imports"],
      ["phantom_deps_enabled", "Phantom dependencies"],
      ["vulnerabilities_enabled", "Vulnerabilities"],
      ["secrets_enabled", "Secrets"],
      ["over_engineering_enabled", "Over-engineering"],
      ["boilerplate_enabled", "Boilerplate"],
      ["todo_leaks_enabled", "TODO leaks"],
      ["stale_api_enabled", "Stale API usage"],
      ["risky_patterns_enabled", "Risky patterns"],
      ["graph_enabled", "Repository graph"],
      ["guidelines_enabled", "Guidelines"],
      ["iac_enabled", "Infrastructure as code"],
      ["auto_labels_enabled", "Auto labels"],
      ["exclude_patterns", "Exclude patterns"],
      ["update_pr_description", "Update PR description"],
      ["custom_instructions", "Custom instructions"],
      ["allow_auto_fix", "Allow auto fix"],
      ["auto_approve", "Auto approve"],
      ["pr_title_fix", "PR title fix"],
      ["audit_retention_days", "Audit retention (days)"],
      ["queue_workers", "Queue workers"],
      ["max_concurrent_reviews", "Max concurrent reviews"],
      ["review_timeout_secs", "Review timeout (seconds)"],
      ["max_inline_comments", "Max inline comments"],
      ["max_reviewer_files", "Max reviewer files"],
      ["max_comment_bytes", "Max comment bytes"],
      ["max_llm_diff_chars", "Max LLM diff characters"],
      ["auto_improve_max_files", "Auto-improve max files"],
      ["auto_improve_max_diff", "Auto-improve max diff"],
      ["hsts", "HSTS"],
      ["metrics_token", "Metrics token"],
      ["secure_cookies", "Secure cookies"],
      ["insecure_cookies", "Insecure cookies"],
    ],
  },
}

const ADVANCED_GROUPS = [
  {
    title: "Review policy",
    description: "Severity, strictness, limits, and reviewer behavior.",
    keys: [
      "default_severity",
      "review_strictness",
      "max_blocking",
      "max_warnings",
      "forbidden_paths",
      "request_reviewers",
      "create_check_run",
      "update_pr_description",
    ],
  },
  {
    title: "Detectors",
    description: "Choose which review detectors run.",
    keys: [
      "hallucinated_imports_enabled",
      "phantom_deps_enabled",
      "vulnerabilities_enabled",
      "secrets_enabled",
      "over_engineering_enabled",
      "boilerplate_enabled",
      "todo_leaks_enabled",
      "stale_api_enabled",
      "risky_patterns_enabled",
      "graph_enabled",
      "guidelines_enabled",
      "iac_enabled",
      "auto_labels_enabled",
    ],
  },
  {
    title: "Output behavior",
    description: "Custom instructions, fixes, and approvals.",
    keys: [
      "custom_instructions",
      "exclude_patterns",
      "allow_auto_fix",
      "auto_approve",
      "pr_title_fix",
    ],
  },
  {
    title: "Runtime limits",
    description: "Workers, timeouts, and review size limits.",
    keys: [
      "audit_retention_days",
      "queue_workers",
      "max_concurrent_reviews",
      "review_timeout_secs",
      "max_inline_comments",
      "max_reviewer_files",
      "max_comment_bytes",
      "max_llm_diff_chars",
      "auto_improve_max_files",
      "auto_improve_max_diff",
    ],
  },
  {
    title: "Security",
    description: "Cookies, HSTS, and metrics access.",
    keys: ["hsts", "metrics_token", "secure_cookies", "insecure_cookies"],
  },
] as const

const TAB_IDS = Object.keys(TABS) as TabId[]

type Integration = { path: string; label: string }

const INTEGRATIONS: Record<TabId, Integration[]> = {
  general: [],
  github: [{ path: "/api/settings/github/test", label: "GitHub App" }],
  llm: [],
  oidc: [{ path: "/api/settings/oidc/test", label: "OIDC discovery" }],
  tickets: [
    { path: "/api/settings/jira/test", label: "Jira" },
    { path: "/api/settings/linear/test", label: "Linear" },
  ],
  advanced: [],
}

type ClearTarget = {
  path: string
  button: string
  title: string
  description: string
}

const CLEARS: Record<TabId, ClearTarget[]> = {
  general: [],
  github: [
    {
      path: "/api/settings/github",
      button: "Clear GitHub config",
      title: "Clear GitHub App config",
      description:
        "Delete the stored GitHub App credentials and mark every repository inactive? The App stays installed on GitHub.",
    },
  ],
  llm: [],
  oidc: [
    {
      path: "/api/settings/oidc",
      button: "Clear SSO config",
      title: "Clear SSO configuration",
      description:
        "Delete the stored OIDC settings? Owners sign in with email and password afterwards.",
    },
  ],
  tickets: [
    {
      path: "/api/settings/tickets",
      button: "Clear ticket config",
      title: "Clear Jira and Linear",
      description: "Delete the stored Jira and Linear credentials?",
    },
  ],
  advanced: [],
}

type TestState = { status: "running" | "ok" | "error"; message: string }

const FORBIDDEN = new ApiError(
  403,
  "Only workspace owners can manage settings."
)

function DetailRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-2">
      <span className="text-muted-foreground">{label}</span>
      <span className="font-medium">{children}</span>
    </div>
  )
}

function OfflineCard({ values }: { values: SettingsMap }) {
  const effective = values.offline_mode_effective === true
  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle>Mode</CardTitle>
        <CardDescription>
          Review mode for this instance, independent of the LLM credential.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        <DetailRow label="Effective">
          <StatusBadge
            label={effective ? "On" : "Off"}
            tone={effective ? "warning" : "default"}
          />
        </DetailRow>
        <DetailRow label="Source">
          <span className="font-mono text-xs">
            {values.offline_mode_source ?? "unknown"}
          </span>
        </DetailRow>
        {values.offline_mode_hint ? (
          <p className="text-sm text-muted-foreground">
            {values.offline_mode_hint}
          </p>
        ) : null}
      </CardContent>
    </Card>
  )
}

function RegistrationCard() {
  const github = useQuery(githubSettingsQueryOptions)
  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle>App registration</CardTitle>
        <CardDescription>
          Written by first-run setup; these bind the GitHub App.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        {github.isPending ? (
          <Skeleton className="h-4 w-1/2" />
        ) : github.isError ? (
          <ErrorState
            error={github.error}
            onRetry={() => void github.refetch()}
          />
        ) : (
          <>
            <DetailRow label="App ID">{github.data.app_id ?? "—"}</DetailRow>
            <DetailRow label="Name">{github.data.app_name ?? "—"}</DetailRow>
            <DetailRow label="Slug">{github.data.slug ?? "—"}</DetailRow>
            <DetailRow label="Configured">
              <Badge variant={github.data.configured ? "default" : "outline"}>
                {github.data.configured ? "Yes" : "No"}
              </Badge>
            </DetailRow>
          </>
        )}
      </CardContent>
    </Card>
  )
}

const NONE = "__none__"

function LlmCard() {
  const queryClient = useQueryClient()
  const config = useQuery(llmConfigQueryOptions)
  const [draftProvider, setDraftProvider] = useState<string | null>(null)
  const [draftReview, setDraftReview] = useState<string | null>(null)
  const [draftCheap, setDraftCheap] = useState<string | null>(null)
  const [draftEmbedding, setDraftEmbedding] = useState<string | null>(null)
  const [apiKey, setApiKey] = useState("")
  const [openField, setOpenField] = useState<string | null>(null)

  const originalProvider = config.data?.upstream_provider ?? ""
  const originalReview =
    config.data?.upstream_model ?? config.data?.model ?? ""
  const originalCheap =
    config.data?.upstream_model_cheap ?? config.data?.model_cheap ?? ""
  const originalEmbedding =
    config.data?.upstream_model_embedding ?? config.data?.embedding_model ?? ""
  const selectedProvider = draftProvider ?? originalProvider
  const reviewValue = draftReview ?? originalReview
  const cheapValue = draftCheap ?? originalCheap
  const embeddingValue = draftEmbedding ?? originalEmbedding
  const disabledProvider = config.data?.provider === "disabled"
  const changed =
    (draftProvider !== null && draftProvider !== originalProvider) ||
    (draftReview !== null && draftReview !== originalReview) ||
    (draftCheap !== null && draftCheap !== originalCheap) ||
    (draftEmbedding !== null && draftEmbedding !== originalEmbedding)

  const save = useMutation({
    mutationFn: () => {
      const body: Record<string, string> = { provider: "litellm" }
      if (!changed) {
        return apiFetch("/api/setup/llm", {
          method: "POST",
          body: JSON.stringify(body),
        })
      }
      body.api_key = apiKey.trim()
      body.upstream_provider = selectedProvider
      body.upstream_model = reviewValue
      body.upstream_model_cheap = cheapValue
      body.upstream_model_embedding = embeddingValue
      return apiFetch("/api/setup/llm", {
        method: "POST",
        body: JSON.stringify(body),
      })
    },
    onSuccess: async () => {
      toast.success("Inference settings saved")
      setApiKey("")
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.llmConfig }),
        queryClient.invalidateQueries({ queryKey: queryKeys.settings }),
        queryClient.invalidateQueries({ queryKey: queryKeys.setupStatus }),
      ])
    },
  })

  const catalog = config.data?.catalog ?? []
  const textModels = catalog.filter((model) => model.mode === "text")
  const embeddingModels = catalog.filter((model) => model.mode === "embedding")

  function picker(
    id: string,
    label: string,
    value: string,
    options: Array<{ value: string; label: string }>,
    placeholder: string,
    onSelect: (value: string) => void
  ) {
    const open = openField === id
    const selectedLabel =
      options.find((option) => option.value === value)?.label ?? placeholder
    return (
      <Field>
        <FieldLabel htmlFor={`settings-llm-${id}`}>{label}</FieldLabel>
        <Popover open={open} onOpenChange={(next) => setOpenField(next ? id : null)}>
          <PopoverTrigger
            id={`settings-llm-${id}`}
            role="combobox"
            aria-label={label}
            aria-expanded={open}
            className={buttonVariants({
              variant: "outline",
              className: "w-full justify-start text-left",
            })}
          >
            {selectedLabel}
            <ChevronsUpDownIcon className="ml-auto opacity-50" />
          </PopoverTrigger>
          <PopoverContent align="start" className="w-(--anchor-width) p-0">
            <Command>
              <CommandInput placeholder={`Search ${label.toLowerCase()}`} />
              <CommandList>
                <CommandEmpty>No matches found.</CommandEmpty>
                <CommandGroup>
                  {options.map((option) => (
                    <CommandItem
                      key={option.value}
                      value={option.value}
                      onSelect={() => {
                        onSelect(option.value)
                        setOpenField(null)
                      }}
                    >
                      {option.label}
                    </CommandItem>
                  ))}
                </CommandGroup>
              </CommandList>
            </Command>
          </PopoverContent>
        </Popover>
      </Field>
    )
  }

  if (config.isPending) {
    return <LoadingState rows={6} />
  }
  if (config.isError) {
    return (
      <ErrorState error={config.error} onRetry={() => void config.refetch()} />
    )
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Inference</CardTitle>
        <CardDescription>
          Provider, credentials, and model aliases used for reviews.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {disabledProvider ? (
          <Alert>
            <AlertTitle>Inference is disabled</AlertTitle>
            <AlertDescription>
              Enable inference during setup before changing provider or models.
            </AlertDescription>
          </Alert>
        ) : null}
        <FieldGroup>
          {picker(
            "provider",
            "Provider",
            selectedProvider,
            withCurrent(
              (config.data?.providers ?? []).map((item) => ({
                value: item.id,
                label: item.name,
              })),
              selectedProvider
            ),
            "Select a provider",
            setDraftProvider
          )}
          <Field>
            <FieldLabel htmlFor="settings-llm-api-key">
              API key
              {config.data?.api_key ? (
                <Badge variant="secondary">Configured</Badge>
              ) : null}
            </FieldLabel>
            <SecretInput
              id="settings-llm-api-key"
              label="API key"
              placeholder="Leave blank to keep the stored key"
              value={apiKey}
              onChange={setApiKey}
            />
            <FieldDescription>
              {changed
                ? "Enter the provider API key to save model changes."
                : "Leave blank to keep the stored key."}
            </FieldDescription>
          </Field>
          {picker(
            "review",
            "Review model",
            reviewValue,
            withCurrent(
              textModels.map((model) => ({ value: model.name, label: model.name })),
              reviewValue
            ),
            "Select a model",
            setDraftReview
          )}
          {picker(
            "cheap",
            "Fallback / cheap model",
            cheapValue,
            withCurrent(
              [
                { value: NONE, label: "Use the review model" },
                ...textModels.map((model) => ({
                  value: model.name,
                  label: model.name,
                })),
              ],
              cheapValue
            ),
            "Use the review model",
            (value) => setDraftCheap(value === NONE ? "" : value)
          )}
          {picker(
            "embedding",
            "Embedding model",
            embeddingValue,
            withCurrent(
              [
                { value: NONE, label: "No embeddings" },
                ...embeddingModels.map((model) => ({
                  value: model.name,
                  label: model.name,
                })),
              ],
              embeddingValue
            ),
            "No embeddings",
            (value) => setDraftEmbedding(value === NONE ? "" : value)
          )}
        </FieldGroup>
        <Button
          onClick={() => save.mutate()}
          disabled={
            disabledProvider ||
            !selectedProvider ||
            !reviewValue ||
            (changed && !apiKey.trim()) ||
            save.isPending
          }
        >
          {save.isPending ? "Saving…" : "Save configuration"}
        </Button>
        {save.isError ? (
          <Alert variant="destructive">
            <AlertTitle>Could not save inference settings</AlertTitle>
            <AlertDescription>{save.error.message}</AlertDescription>
          </Alert>
        ) : null}
      </CardContent>
    </Card>
  )
}

type TabBodyProps = {
  tab: TabId
  values: SettingsMap
  tests: Record<string, TestState>
  clearErrors: Record<string, unknown>
  onSaved: (key: string, restartRequired: boolean) => void
  onTest: (integration: Integration) => void
  onClear: (target: ClearTarget) => void
}

function TabBody({
  tab,
  values,
  tests,
  clearErrors,
  onSaved,
  onTest,
  onClear,
}: TabBodyProps) {
  const meta = TABS[tab]
  const integrations = INTEGRATIONS[tab]
  const clears = CLEARS[tab]

  function fields(fieldsToRender: readonly [string, string][]) {
    return (
      <FieldGroup>
        {fieldsToRender.map(([key, label]) => (
          <SettingField
            key={`${key}:${values[key] ?? ""}`}
            settingKey={key}
            label={label}
            value={values[key]}
            onSaved={onSaved}
          />
        ))}
      </FieldGroup>
    )
  }

  return (
    <div className="flex flex-col gap-4">
      {tab === "general" ? <OfflineCard values={values} /> : null}
      {tab === "github" ? <RegistrationCard /> : null}
      {tab === "llm" ? <LlmCard /> : null}

      {tab === "advanced" ? (
        <Card>
          <CardHeader>
            <CardTitle>{meta.label}</CardTitle>
            <CardDescription>{meta.description}</CardDescription>
          </CardHeader>
          <CardContent>
            <Accordion defaultValue={ADVANCED_GROUPS.map((group) => group.title)}>
              {ADVANCED_GROUPS.map((group) => (
                <AccordionItem key={group.title} value={group.title}>
                  <AccordionTrigger>
                    <span className="flex flex-col items-start gap-0.5">
                      <span>{group.title}</span>
                      <span className="text-xs font-normal text-muted-foreground">
                        {group.description}
                      </span>
                    </span>
                  </AccordionTrigger>
                  <AccordionContent>
                    {fields(
                      group.keys.map((key) => {
                        const entry = TABS.advanced.fields.find(
                          ([fieldKey]) => fieldKey === key
                        )
                        return entry ?? [key, key]
                      })
                    )}
                  </AccordionContent>
                </AccordionItem>
              ))}
            </Accordion>
          </CardContent>
        </Card>
      ) : meta.fields.length > 0 ? (
        <Card>
          <CardHeader>
            <CardTitle>{meta.label}</CardTitle>
            <CardDescription>{meta.description}</CardDescription>
          </CardHeader>
          <CardContent>{fields(meta.fields)}</CardContent>
        </Card>
      ) : null}

      {integrations.length > 0 ? (
        <Card>
          <CardHeader>
            <CardTitle>Connection tests</CardTitle>
            <CardDescription>
              Saved credentials are used; unsaved edits are ignored.
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-3">
            {integrations.map((integration) => {
              const state = tests[integration.path]
              return (
                <div key={integration.path} className="flex flex-col gap-2">
                  <div className="flex items-center justify-between gap-2">
                    <span>{integration.label}</span>
                    <Button
                      size="sm"
                      variant="outline"
                      aria-label={`Test ${integration.label}`}
                      disabled={state?.status === "running"}
                      onClick={() => onTest(integration)}
                    >
                      {state?.status === "running"
                        ? "Testing…"
                        : "Test connection"}
                    </Button>
                  </div>
                  {state?.status === "ok" ? (
                    <Alert>
                      <AlertTitle>Connected</AlertTitle>
                      <AlertDescription>{state.message}</AlertDescription>
                    </Alert>
                  ) : null}
                  {state?.status === "error" ? (
                    <Alert variant="destructive">
                      <AlertTitle>Connection failed</AlertTitle>
                      <AlertDescription>{state.message}</AlertDescription>
                    </Alert>
                  ) : null}
                </div>
              )
            })}
          </CardContent>
        </Card>
      ) : null}

      {clears.length > 0 ? (
        <Card>
          <CardHeader>
            <CardTitle>Danger zone</CardTitle>
            <CardDescription>
              Removes stored credentials. This cannot be undone.
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-2">
            {clears.map((target) => (
              <div
                key={target.path}
                className="flex flex-col items-start gap-2"
              >
                <Button
                  variant="destructive"
                  onClick={() => onClear(target)}
                >
                  {target.button}
                </Button>
                {clearErrors[target.path] ? (
                  <ErrorState error={clearErrors[target.path]} />
                ) : null}
              </div>
            ))}
          </CardContent>
        </Card>
      ) : null}
    </div>
  )
}

function SettingsConsole() {
  const queryClient = useQueryClient()
  const settings = useQuery(settingsQueryOptions)
  const [tab, setTab] = useState<TabId>("general")
  const [restartKeys, setRestartKeys] = useState<string[]>([])
  const [tests, setTests] = useState<Record<string, TestState>>({})
  const [clearing, setClearing] = useState<ClearTarget | null>(null)
  const [clearErrors, setClearErrors] = useState<Record<string, unknown>>({})

  async function refresh() {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.settings }),
      queryClient.invalidateQueries({ queryKey: queryKeys.repos }),
      queryClient.invalidateQueries({ queryKey: queryKeys.oidcStatus }),
      queryClient.invalidateQueries({ queryKey: queryKeys.health }),
      queryClient.invalidateQueries({ queryKey: queryKeys.setupStatus }),
    ])
  }

  function handleSaved(key: string, restartRequired: boolean) {
    if (restartRequired) {
      setRestartKeys((previous) =>
        previous.includes(key) ? previous : [...previous, key]
      )
    }
    void refresh()
  }

  async function runTest(integration: Integration) {
    setTests((previous) => ({
      ...previous,
      [integration.path]: { status: "running", message: "" },
    }))
    try {
      const result = await apiFetch(
        integration.path,
        { method: "POST" },
        integrationTestSchema
      )
      setTests((previous) => ({
        ...previous,
        [integration.path]: {
          status: "ok",
          message: result.message ?? "Connected",
        },
      }))
    } catch (caught) {
      setTests((previous) => ({
        ...previous,
        [integration.path]: {
          status: "error",
          message:
            caught instanceof Error ? caught.message : "Connection failed",
        },
      }))
    }
  }

  async function runClear(target: ClearTarget) {
    setClearErrors((previous) => {
      const next = { ...previous }
      delete next[target.path]
      return next
    })
    try {
      const result = await apiFetch(
        target.path,
        { method: "DELETE" },
        settingsActionResultSchema
      )
      toast.success(result.message ?? `${target.button} done`)
      await refresh()
    } catch (caught) {
      setClearErrors((previous) => ({ ...previous, [target.path]: caught }))
    }
  }

  if (settings.isPending) {
    return <LoadingState rows={6} />
  }
  if (settings.isError) {
    return (
      <ErrorState
        error={settings.error}
        onRetry={() => void settings.refetch()}
      />
    )
  }

  const values = settings.data

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Settings"
        description="Owner-only instance configuration."
      />

      {restartKeys.length > 0 ? (
        <Alert>
          <TriangleAlertIcon />
          <AlertTitle>Restart required</AlertTitle>
          <AlertDescription>
            Saved {restartKeys.join(", ")}. The new value takes effect after the
            server restarts.
          </AlertDescription>
        </Alert>
      ) : null}

      <Tabs value={tab} onValueChange={(value) => setTab(value as TabId)}>
        <TabsList>
          {TAB_IDS.map((id) => (
            <TabsTrigger key={id} value={id}>
              {TABS[id].label}
            </TabsTrigger>
          ))}
        </TabsList>
        {TAB_IDS.map((id) => (
          <TabsContent key={id} value={id}>
            <TabBody
              tab={id}
              values={values}
              tests={tests}
              clearErrors={clearErrors}
              onSaved={handleSaved}
              onTest={(integration) => void runTest(integration)}
              onClear={setClearing}
            />
          </TabsContent>
        ))}
      </Tabs>

      <ConfirmDialog
        open={clearing !== null}
        onOpenChange={(open) => {
          if (!open) {
            setClearing(null)
          }
        }}
        title={clearing?.title ?? ""}
        description={clearing?.description ?? ""}
        onConfirm={() => {
          if (clearing) {
            void runClear(clearing)
          }
        }}
        destructive
      />
    </div>
  )
}

function SettingsRoute() {
  const { user, status } = useAuth()

  if (status === "loading") {
    return <LoadingState rows={6} />
  }
  if (!can(user?.role, "owner")) {
    return <ErrorState error={FORBIDDEN} />
  }
  return <SettingsConsole />
}

export default SettingsRoute
