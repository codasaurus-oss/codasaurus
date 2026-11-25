import { useState } from "react"
import { toast } from "sonner"

import { ErrorState } from "@/components/common/error-state"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Field,
  FieldDescription,
  FieldTitle,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import { SecretInput } from "@/features/setup/secret-field"
import { apiFetch } from "@/lib/api/client"
import { settingSaveSchema } from "@/lib/api/schemas"

type Control = "text" | "url" | "password" | "number" | "switch" | "select" | "textarea"

type SettingFieldProps = {
  settingKey: string
  label: string
  value?: string
  onSaved: (key: string, restartRequired: boolean) => void
}

// Mirrors the backend SENSITIVE_KEYS masking: these arrive as "••••••••".
const MASKED_KEYS = new Set([
  "llm_api_key",
  "github_private_key",
  "github_webhook_secret",
  "oidc_client_secret",
  "metrics_token",
  "jira_api_token",
  "linear_api_key",
])

const BOOLEAN_KEYS = new Set([
  "hsts",
  "insecure_cookies",
  "secure_cookies",
  "offline_mode",
  "oidc_allow_open_join",
  "oidc_allow_unverified_email",
  "oidc_allow_public_client",
  "request_reviewers",
  "create_check_run",
  "update_pr_description",
  "allow_auto_fix",
  "auto_approve",
])

const SELECT_OPTIONS: Record<string, readonly string[]> = {
  llm_provider: ["litellm", "disabled"],
  default_severity: ["blocking", "warning", "info"],
  review_strictness: ["lenient", "balanced", "strict", "nitpick"],
  pr_title_fix: ["off", "suggest", "auto"],
}

const URL_KEYS = new Set([
  "public_url",
  "oidc_issuer",
  "oidc_redirect_uri",
  "jira_base_url",
])

const NUMBER_KEYS = new Set([
  "llm_daily_budget_usd",
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
  "max_warnings",
  "max_blocking",
])

const TEXTAREA_KEYS = new Set([
  "forbidden_paths",
  "exclude_patterns",
  "custom_instructions",
])

function controlFor(key: string): Control {
  if (MASKED_KEYS.has(key)) {
    return "password"
  }
  if (BOOLEAN_KEYS.has(key) || key.endsWith("_enabled")) {
    return "switch"
  }
  if (key in SELECT_OPTIONS) {
    return "select"
  }
  if (URL_KEYS.has(key)) {
    return "url"
  }
  if (NUMBER_KEYS.has(key)) {
    return "number"
  }
  if (TEXTAREA_KEYS.has(key)) {
    return "textarea"
  }
  return "text"
}

function isTrue(value: string | undefined) {
  return ["1", "true", "yes", "on"].includes((value ?? "").trim().toLowerCase())
}

// The backend skips a masked or "*" secret, so it must never be submitted as a change.
function isMaskPlaceholder(value: string | undefined) {
  if (!value) {
    return false
  }
  return value.includes("•") || value.includes("*")
}

function inputType(control: Control) {
  if (control === "number" || control === "url" || control === "password") {
    return control
  }
  return "text"
}

function SettingField({ settingKey, label, value, onSaved }: SettingFieldProps) {
  const control = controlFor(settingKey)
  const id = `setting-${settingKey}`
  const masked = control === "password" && isMaskPlaceholder(value)
  const [draft, setDraft] = useState(control === "password" ? "" : (value ?? ""))
  const [checked, setChecked] = useState(isTrue(value))
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<unknown>(null)
  const [notice, setNotice] = useState<string | null>(null)

  async function save(next: string): Promise<boolean> {
    if (control === "password" && (next.includes("•") || next.includes("*"))) {
      setNotice(
        "Masked and starred values are not submitted. Enter the secret itself."
      )
      return false
    }
    setNotice(null)
    setSaving(true)
    setError(null)
    try {
      const result = await apiFetch(
        `/api/settings/${settingKey}`,
        { method: "PUT", body: JSON.stringify({ value: next }) },
        settingSaveSchema
      )
      if (result.skipped === "unchanged") {
        toast.message(`${label} unchanged`)
      } else {
        toast.success(`${label} saved`)
      }
      if (control === "password") {
        setDraft("")
      }
      onSaved(settingKey, Boolean(result.restart_required))
      return true
    } catch (caught) {
      setError(caught)
      return false
    } finally {
      setSaving(false)
    }
  }

  if (control === "switch") {
    return (
      <Field orientation="horizontal">
        <FieldTitle>{label}</FieldTitle>
        <Switch
          id={id}
          aria-label={label}
          checked={checked}
          disabled={saving}
          onCheckedChange={(next) => {
            const previous = checked
            setChecked(next)
            void save(next ? "true" : "false").then((saved) => {
              if (!saved) {
                setChecked(previous)
              }
            })
          }}
        />
        {error ? <ErrorState error={error} /> : null}
      </Field>
    )
  }

  const unchanged = control === "password" ? draft === "" : draft === (value ?? "")
  const options = SELECT_OPTIONS[settingKey]

  return (
    <Field>
      <FieldTitle>
        {label}
        {masked ? <Badge variant="secondary">Configured</Badge> : null}
      </FieldTitle>
      {control === "select" && options ? (
        <Select
          value={draft === "" ? null : draft}
          onValueChange={(next) => setDraft(next === null ? "" : String(next))}
        >
          <SelectTrigger id={id} aria-label={label} className="w-full sm:w-64">
            <SelectValue placeholder="Not set" />
          </SelectTrigger>
          <SelectContent className="w-full sm:w-64">
            {options.map((option) => (
              <SelectItem key={option} value={option}>
                {option}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      ) : control === "textarea" ? (
        <Textarea
          id={id}
          aria-label={label}
          rows={4}
          className="font-mono text-xs"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
        />
      ) : control === "password" ? (
        <SecretInput
          id={id}
          label={label}
          placeholder={masked ? "••••••••" : "Not configured"}
          value={draft}
          onChange={(next) => {
            setDraft(next)
            setNotice(null)
          }}
        />
      ) : (
        <Input
          id={id}
          aria-label={label}
          type={inputType(control)}
          step={control === "number" ? "any" : undefined}
          value={draft}
          onChange={(event) => {
            setDraft(event.target.value)
            setNotice(null)
          }}
        />
      )}
      {control === "password" ? (
        <FieldDescription>
          Leave blank to keep the stored value; it is never shown again.
        </FieldDescription>
      ) : null}
      {notice ? (
        <p role="status" className="text-xs text-warning">
          {notice}
        </p>
      ) : null}
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          aria-label={`${masked ? "Replace" : "Save"} ${label}`}
          disabled={saving || unchanged}
          onClick={() => void save(draft)}
        >
          {saving ? "Saving…" : masked ? "Replace" : "Save"}
        </Button>
        {settingKey === "metrics_token" && masked ? (
          <Button
            size="sm"
            variant="outline"
            aria-label={`Clear ${label}`}
            disabled={saving}
            onClick={() => void save("")}
          >
            Clear token
          </Button>
        ) : null}
      </div>
      {error ? <ErrorState error={error} /> : null}
    </Field>
  )
}

export { SettingField }
