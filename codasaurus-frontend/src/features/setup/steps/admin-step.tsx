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
import { SecretInput } from "@/features/setup/secret-field"
import { type SetupStepProps } from "@/features/setup/steps"
import { apiFetch } from "@/lib/api/client"
import { setupResultSchema } from "@/lib/api/schemas"

type AdminStepProps = SetupStepProps & { firstOwner: boolean }

function validate(email: string, password: string) {
  if (!email.includes("@")) {
    return "Enter a valid email address"
  }
  if (password.length < 10) {
    return "Password must be at least 10 characters"
  }
  if (password.toLowerCase() === email.toLowerCase()) {
    return "Password must not be the same as your email"
  }
  return null
}

function AdminStep({ firstOwner, onComplete }: AdminStepProps) {
  const [email, setEmail] = useState("")
  const [password, setPassword] = useState("")
  const [bootstrapSecret, setBootstrapSecret] = useState("")
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function createOwner() {
    const invalid = validate(email.trim(), password)
    if (invalid) {
      setError(invalid)
      return
    }
    setBusy(true)
    setError(null)
    try {
      await apiFetch(
        "/api/setup/admin",
        {
          method: "POST",
          headers: firstOwner
            ? { "x-bootstrap-secret": bootstrapSecret }
            : undefined,
          body: JSON.stringify({ email: email.trim(), password }),
        },
        setupResultSchema
      )
      onComplete()
    } catch (caught) {
      setError(
        caught instanceof Error
          ? caught.message
          : "Could not create the owner account."
      )
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <FieldGroup>
        <Field>
          <FieldLabel htmlFor="setup-admin-email">Email</FieldLabel>
          <Input
            id="setup-admin-email"
            type="email"
            autoComplete="email"
            value={email}
            onChange={(event) => setEmail(event.target.value)}
          />
        </Field>
        <Field>
          <FieldLabel htmlFor="setup-admin-password">Password</FieldLabel>
          <SecretInput
            id="setup-admin-password"
            label="Password"
            autoComplete="new-password"
            value={password}
            onChange={setPassword}
          />
          <FieldDescription>
            At least 10 characters and different from your email.
          </FieldDescription>
        </Field>
        {firstOwner ? (
          <Field>
            <FieldLabel htmlFor="setup-admin-secret">Setup secret</FieldLabel>
            <SecretInput
              id="setup-admin-secret"
              label="Setup secret"
              value={bootstrapSecret}
              onChange={setBootstrapSecret}
            />
            <FieldDescription>
              The one-time key from your server settings. It is needed only to
              create the first owner account.
            </FieldDescription>
          </Field>
        ) : null}
      </FieldGroup>
      <Button onClick={() => void createOwner()} disabled={busy}>
        {busy ? "Creating…" : "Create owner"}
      </Button>

      {error ? (
        <Alert variant="destructive">
          <AlertTitle>Could not create the owner</AlertTitle>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      ) : null}
    </>
  )
}

export { AdminStep }
