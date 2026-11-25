import { useState } from "react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { SecretInput, SecretTextarea } from "@/features/setup/secret-field"
import { type SetupStepProps } from "@/features/setup/steps"
import { apiFetch } from "@/lib/api/client"
import { setupResultSchema } from "@/lib/api/schemas"

type GithubStepProps = SetupStepProps & {
  installUrl?: string | null
  publicUrl: string
  onSaved: () => void
}

// Mirrors the server's is_local_url (src/api/setup.rs) so the manifest button
// is gated on the same rule that decides the callback URL GitHub receives.
function isLocalUrl(url: string) {
  const lower = url.toLowerCase()
  return (
    lower.startsWith("http://localhost") ||
    lower.startsWith("http://127.") ||
    lower.startsWith("http://[::1]") ||
    lower.includes("://localhost:")
  )
}

function GithubStep({
  complete,
  installUrl,
  publicUrl,
  onComplete,
  onSaved,
}: GithubStepProps) {
  const [appId, setAppId] = useState("")
  const [privateKey, setPrivateKey] = useState("")
  const [webhookSecret, setWebhookSecret] = useState("")
  const [slug, setSlug] = useState("")
  const [draftPublicUrl, setDraftPublicUrl] = useState(publicUrl)
  const [message, setMessage] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const localhost = isLocalUrl(publicUrl)

  async function savePublicUrl() {
    setBusy(true)
    setError(null)
    setMessage(null)
    try {
      const result = await apiFetch(
        "/api/setup/public-url",
        { method: "POST", body: JSON.stringify({ url: draftPublicUrl.trim() }) },
        setupResultSchema
      )
      setMessage(result.message ?? "Public URL saved")
      onSaved()
    } catch (caught) {
      setError(
        caught instanceof Error ? caught.message : "Could not save the public URL."
      )
    } finally {
      setBusy(false)
    }
  }

  async function saveManual() {
    setBusy(true)
    setError(null)
    setMessage(null)
    try {
      const body: Record<string, string> = {
        app_id: appId.trim(),
        private_key: privateKey.trim(),
        webhook_secret: webhookSecret.trim(),
      }
      if (slug.trim()) {
        body.slug = slug.trim()
      }
      const result = await apiFetch(
        "/api/setup/github",
        { method: "POST", body: JSON.stringify(body) },
        setupResultSchema
      )
      setMessage(result.message ?? "GitHub App credentials saved")
      // The parent holds this step open so the install link can render.
      onSaved()
    } catch (caught) {
      setError(
        caught instanceof Error
          ? caught.message
          : "Could not save the GitHub App credentials."
      )
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <Tabs defaultValue="manifest">
        <TabsList>
          <TabsTrigger value="manifest">Manifest</TabsTrigger>
          <TabsTrigger value="manual">Manual</TabsTrigger>
        </TabsList>
        <TabsContent value="manifest" className="flex flex-col gap-4 pt-2">
          <p className="text-sm text-muted-foreground">
            GitHub creates the app from a manifest and returns here when it is
            done. You will leave this page for a moment.
          </p>
          <Field>
            <FieldLabel htmlFor="setup-github-public-url">Public URL</FieldLabel>
            <Input
              id="setup-github-public-url"
              value={draftPublicUrl}
              onChange={(event) => setDraftPublicUrl(event.target.value)}
              placeholder="https://codasaurus.example.com"
            />
            <FieldDescription>
              Used for GitHub webhook and callback URLs.
            </FieldDescription>
          </Field>
          {localhost ? (
            <Alert>
              <AlertTitle>Localhost URL</AlertTitle>
              <AlertDescription>
                GitHub cannot reach a localhost URL. Save a public HTTPS URL
                here, or use the Manual tab.
              </AlertDescription>
            </Alert>
          ) : null}
          <Button
            variant="outline"
            disabled={busy || draftPublicUrl.trim() === ""}
            onClick={() => void savePublicUrl()}
          >
            {busy ? "Saving…" : "Save public URL"}
          </Button>
          <Button
            disabled={localhost}
            onClick={() => {
              window.location.href = "/api/setup/github/manifest-page"
            }}
          >
            Create app with GitHub
          </Button>
        </TabsContent>
        <TabsContent value="manual" className="flex flex-col gap-4 pt-2">
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="setup-github-app-id">App ID</FieldLabel>
              <Input
                id="setup-github-app-id"
                value={appId}
                onChange={(event) => setAppId(event.target.value)}
              />
            </Field>
            <Field>
              <SecretTextarea
                id="setup-github-private-key"
                label="Private key"
                className="field-sizing-fixed h-40 max-h-40 resize-y overflow-y-auto font-mono text-xs"
                rows={6}
                value={privateKey}
                onChange={setPrivateKey}
              />
              <FieldDescription>
                The PEM private key shown on the GitHub App settings page.
              </FieldDescription>
            </Field>
            <Field>
              <FieldLabel htmlFor="setup-github-webhook-secret">
                Webhook secret
              </FieldLabel>
              <SecretInput
                id="setup-github-webhook-secret"
                label="Webhook secret"
                value={webhookSecret}
                onChange={setWebhookSecret}
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="setup-github-slug">
                App slug (optional)
              </FieldLabel>
              <Input
                id="setup-github-slug"
                placeholder="codasaurus"
                value={slug}
                onChange={(event) => setSlug(event.target.value)}
              />
              <FieldDescription>
                Resolved from GitHub when omitted. It powers the install link.
              </FieldDescription>
            </Field>
          </FieldGroup>
          <Button onClick={() => void saveManual()} disabled={busy}>
            {busy ? "Verifying…" : "Save credentials"}
          </Button>
        </TabsContent>
      </Tabs>

      {installUrl || complete ? (
        <div className="flex flex-wrap gap-2">
          {installUrl ? (
            <Button
              variant="outline"
              render={
                <a href={installUrl} target="_blank" rel="noreferrer" />
              }
            >
              Install the app on GitHub
            </Button>
          ) : null}
          {complete ? <Button onClick={onComplete}>Continue</Button> : null}
        </div>
      ) : null}

      {message ? (
        <Alert>
          <AlertTitle>GitHub App</AlertTitle>
          <AlertDescription>{message}</AlertDescription>
        </Alert>
      ) : null}
      {error ? (
        <Alert variant="destructive">
          <AlertTitle>Could not save the GitHub App</AlertTitle>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      ) : null}
    </>
  )
}

export { GithubStep }
