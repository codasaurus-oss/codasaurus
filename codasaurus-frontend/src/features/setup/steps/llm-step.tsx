import { useMutation, useQuery } from "@tanstack/react-query"
import { Fragment, useEffect, useRef, useState } from "react"
import { ChevronsUpDownIcon } from "lucide-react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button, buttonVariants } from "@/components/ui/button"
import { ButtonGroup } from "@/components/ui/button-group"
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
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import { SecretInput } from "@/features/setup/secret-field"
import { type SetupStepProps } from "@/features/setup/steps"
import { Spinner } from "@/components/ui/spinner"
import { apiFetch } from "@/lib/api/client"
import { llmConfigSchema, modelCatalogSchema, setupResultSchema } from "@/lib/api/schemas"
import { withCurrent, type PickerOption } from "@/lib/model-options"

const REVIEW_ALIAS = "codasaurus-review"
const CHEAP_ALIAS = "codasaurus-cheap"
const EMBEDDING_ALIAS = "codasaurus-embedding"
const NONE = "__none__"

function LlmStep({ onComplete }: SetupStepProps) {
  const [draftProvider, setDraftProvider] = useState<string | null>(null)
  const config = useQuery({
    queryKey: ["setup", "llm", draftProvider],
    queryFn: () =>
      apiFetch(
        draftProvider
          ? `/api/setup/llm?provider=${encodeURIComponent(draftProvider)}`
          : "/api/setup/llm",
        undefined,
        llmConfigSchema
      ),
  })
  const selectedProvider = draftProvider ?? config.data?.upstream_provider ?? ""
  const [apiKey, setApiKey] = useState("")
  const [discoveredCatalog, setDiscoveredCatalog] = useState<
    Array<{ name: string; mode: string }> | null
  >(null)
  const discoveryRequest = useRef(0)
  const discover = useMutation({
    mutationFn: () =>
      apiFetch(
        "/api/setup/llm/discover",
        {
          method: "POST",
          body: JSON.stringify({
            provider: selectedProvider,
            api_key: apiKey.trim(),
          }),
        },
        modelCatalogSchema
      ),
  })
  const { mutate: loadModels, reset: resetDiscovery } = discover
  const [draftReview, setDraftReview] = useState<string | null>(null)
  const [draftCheap, setDraftCheap] = useState<string | null>(null)
  const [draftEmbedding, setDraftEmbedding] = useState<string | null>(null)
  const [openField, setOpenField] = useState<string | null>(null)
  const [message, setMessage] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState<"litellm" | "disabled" | null>(null)

  useEffect(() => {
    const request = ++discoveryRequest.current
    if (!selectedProvider || apiKey.trim() === "") {
      resetDiscovery()
      return
    }
    const timer = window.setTimeout(() => {
      loadModels(undefined, {
        onSuccess: (result) => {
          if (request === discoveryRequest.current) {
            setDiscoveredCatalog(result.models)
          }
        },
      })
    }, 400)
    return () => window.clearTimeout(timer)
  }, [selectedProvider, apiKey, loadModels, resetDiscovery])

  const reviewValue =
    draftReview ?? config.data?.upstream_model ?? config.data?.model ?? ""
  const cheapValue =
    draftCheap ??
    config.data?.upstream_model_cheap ??
    (config.data?.model_cheap ? config.data.model_cheap : "")
  const embeddingValue =
    draftEmbedding ??
    config.data?.upstream_model_embedding ??
    (config.data?.embedding_model ? config.data.embedding_model : "")
  const providers = config.data?.providers ?? []
  const catalog = discoveredCatalog ?? config.data?.catalog ?? []
  const modelsReady =
    Boolean(selectedProvider) &&
    apiKey.trim() !== "" &&
    discoveredCatalog !== null &&
    !discover.isPending
  const textModels = catalog.filter((model) => model.mode === "text")
  const embeddingModels = catalog.filter((model) => model.mode === "embedding")
  const canSave =
    selectedProvider !== "" &&
    reviewValue !== "" &&
    apiKey.trim() !== "" &&
    busy === null

  const pickerFields: Array<{
    id: string
    label: string
    value: string
    options: PickerOption[]
    placeholder: string
    searchPlaceholder: string
    disabled: boolean
    description: string
    onSelect: (value: string) => void
  }> = [
    {
      id: "provider",
      label: "Provider",
      value: selectedProvider,
      options: withCurrent(
        providers.map((provider) => ({
          value: provider.id,
          label: provider.name,
        })),
        selectedProvider
      ),
      placeholder: "Select a provider",
      searchPlaceholder: "Search providers",
      disabled: config.isPending,
      description: "Select the service that supplies your models.",
      onSelect: (value) => {
        setDraftProvider(value)
        setDiscoveredCatalog(null)
        resetDiscovery()
        setDraftReview("")
        setDraftCheap("")
        setDraftEmbedding("")
      },
    },
    {
      id: "review-model",
      label: "Review model",
      value: reviewValue,
      options: withCurrent(
        textModels.map((model) => ({ value: model.name, label: model.name })),
        reviewValue
      ),
      placeholder: "Select a model",
      searchPlaceholder: "Search models",
      disabled: !modelsReady || config.isPending,
      description:
        selectedProvider && textModels.length === 0
          ? "No text models are available for this provider."
          : "Used for detailed code review.",
      onSelect: setDraftReview,
    },
    {
      id: "cheap-model",
      label: "Fallback / cheap model",
      value: cheapValue,
      options: [
        { value: NONE, label: "Use the review model" },
        ...withCurrent(
          textModels.map((model) => ({ value: model.name, label: model.name })),
          cheapValue
        ),
      ],
      placeholder: "Use the review model",
      searchPlaceholder: "Search models",
      disabled: !modelsReady || config.isPending,
      description: "Optional. Used for lighter review tasks.",
      onSelect: (value) => setDraftCheap(value === NONE ? "" : value),
    },
    {
      id: "embedding-model",
      label: "Embedding model",
      value: embeddingValue,
      options: [
        { value: NONE, label: "No embeddings" },
        ...withCurrent(
          embeddingModels.map((model) => ({
            value: model.name,
            label: model.name,
          })),
          embeddingValue
        ),
      ],
      placeholder: "No embeddings",
      searchPlaceholder: "Search embedding models",
      disabled: !modelsReady || config.isPending,
      description:
        selectedProvider && embeddingModels.length === 0
          ? "No embedding models are available for this provider."
          : "Optional. Enables semantic code search.",
      onSelect: (value) => setDraftEmbedding(value === NONE ? "" : value),
    },
  ]

  async function save(provider: "litellm" | "disabled") {
    setBusy(provider)
    setError(null)
    setMessage(null)
    try {
      const body: Record<string, string> = { provider }
      if (provider === "litellm") {
        Object.assign(body, {
          api_key: apiKey.trim(),
          upstream_provider: selectedProvider,
          upstream_model: reviewValue,
          model: config.data?.model ?? REVIEW_ALIAS,
        })
        if (cheapValue) {
          Object.assign(body, {
            upstream_model_cheap: cheapValue,
            model_cheap: config.data?.model_cheap ?? CHEAP_ALIAS,
          })
        }
        if (embeddingValue) {
          Object.assign(body, {
            upstream_model_embedding: embeddingValue,
            embedding_model: config.data?.embedding_model ?? EMBEDDING_ALIAS,
          })
        }
      }
      const result = await apiFetch(
        "/api/setup/llm?test=true",
        { method: "POST", body: JSON.stringify(body) },
        setupResultSchema
      )
      setMessage(result.message ?? "Configuration saved")
      onComplete()
    } catch (caught) {
      setError(
        caught instanceof Error ? caught.message : "Could not save the configuration."
      )
    } finally {
      setBusy(null)
    }
  }

  return (
    <>
      {config.isError ? (
        <Alert variant="destructive">
          <AlertTitle>Could not load providers</AlertTitle>
          <AlertDescription>{config.error.message}</AlertDescription>
        </Alert>
      ) : null}

      <p className="text-sm text-muted-foreground">
        Choose a provider and models for code reviews. You can skip this and
        configure inference later in Settings.
      </p>

      <FieldGroup>
        {pickerFields.map((field) => {
          const open = openField === field.id
          const selectedLabel =
            field.options.find((option) => option.value === field.value)?.label ??
            field.placeholder
          return (
            <Fragment key={field.id}>
              <Field>
                <FieldLabel htmlFor={`setup-${field.id}`}>{field.label}</FieldLabel>
                <Popover
                  open={open}
                  onOpenChange={(next) => setOpenField(next ? field.id : null)}
                >
                  <PopoverTrigger
                    id={`setup-${field.id}`}
                    role="combobox"
                    aria-expanded={open}
                    aria-label={field.label}
                    className={buttonVariants({
                      variant: "outline",
                      className: "w-full justify-start text-left",
                    })}
                    disabled={field.disabled}
                  >
                    {selectedLabel}
                    <ChevronsUpDownIcon className="ml-auto opacity-50" />
                  </PopoverTrigger>
                  <PopoverContent
                    align="start"
                    className="w-(--anchor-width) p-0"
                  >
                    <Command>
                      <CommandInput
                        placeholder={field.searchPlaceholder}
                        aria-label={field.searchPlaceholder}
                      />
                      <CommandList>
                        <CommandEmpty>No matches found.</CommandEmpty>
                        <CommandGroup>
                          {field.options.map((option) => (
                            <CommandItem
                              key={option.value}
                              value={option.value}
                              keywords={[option.label]}
                              onSelect={() => {
                                field.onSelect(option.value)
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
                <FieldDescription>{field.description}</FieldDescription>
              </Field>
              {field.id === "provider" ? (
                <Field>
                  <FieldLabel htmlFor="setup-api-key">API key</FieldLabel>
                  <SecretInput
                    id="setup-api-key"
                    label="API key"
                    placeholder="Enter your API key"
                    disabled={!selectedProvider}
                    value={apiKey}
                    onChange={(value) => {
                      setApiKey(value)
                      setDiscoveredCatalog(null)
                    }}
                  />
                  <FieldDescription className="flex items-center gap-2">
                    {discover.isPending ? (
                      <>
                        <Spinner />
                        <span>Loading models…</span>
                      </>
                    ) : (
                      "Models load automatically after you enter your API key."
                    )}
                  </FieldDescription>
                  {discover.isError ? (
                    <FieldDescription className="text-destructive">
                      {discover.error.message}
                    </FieldDescription>
                  ) : null}
                </Field>
              ) : null}
            </Fragment>
          )
        })}
      </FieldGroup>

      <ButtonGroup>
        <Button onClick={() => void save("litellm")} disabled={!canSave}>
          {busy === "litellm" ? "Testing…" : "Save and test"}
        </Button>
        <Button
          variant="outline"
          onClick={() => void save("disabled")}
          disabled={busy !== null}
        >
          {busy === "disabled" ? "Saving…" : "Continue without AI"}
        </Button>
      </ButtonGroup>

      {message ? (
        <Alert>
          <AlertTitle>Inference</AlertTitle>
          <AlertDescription>{message}</AlertDescription>
        </Alert>
      ) : null}
      {error ? (
        <Alert variant="destructive">
          <AlertTitle>Could not save the configuration</AlertTitle>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      ) : null}
    </>
  )
}

export { LlmStep }
