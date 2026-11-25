import { useState, type FormEvent } from "react"
import { toast } from "sonner"

import { ErrorState } from "@/components/common/error-state"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Field, FieldGroup, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { apiFetch } from "@/lib/api/client"

type ChangePasswordDialogProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
  email: string
}

function ChangePasswordDialog({
  open,
  onOpenChange,
  email,
}: ChangePasswordDialogProps) {
  const [current, setCurrent] = useState("")
  const [next, setNext] = useState("")
  const [error, setError] = useState<unknown>(null)
  const [saving, setSaving] = useState(false)

  function reset() {
    setCurrent("")
    setNext("")
    setError(null)
  }

  function handleOpenChange(value: boolean) {
    if (!value) {
      reset()
    }
    onOpenChange(value)
  }

  async function submit(event: FormEvent) {
    event.preventDefault()
    setError(null)
    if (next.length < 10) {
      setError("Password must be at least 10 characters")
      return
    }
    if (next.toLowerCase() === email.toLowerCase()) {
      setError("Password must not be the same as your email")
      return
    }
    setSaving(true)
    try {
      await apiFetch("/api/users/me/password", {
        method: "POST",
        body: JSON.stringify({
          current_password: current,
          new_password: next,
        }),
      })
      toast.success("Password changed")
      handleOpenChange(false)
    } catch (caught) {
      setError(caught)
    } finally {
      setSaving(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => void submit(event)}
        >
          <DialogHeader>
            <DialogTitle>Change password</DialogTitle>
            <DialogDescription>
              Use at least 10 characters, and not your email address.
            </DialogDescription>
          </DialogHeader>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="current-password">Current password</FieldLabel>
              <Input
                id="current-password"
                type="password"
                autoComplete="current-password"
                value={current}
                onChange={(event) => setCurrent(event.target.value)}
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="new-password">New password</FieldLabel>
              <Input
                id="new-password"
                type="password"
                autoComplete="new-password"
                value={next}
                onChange={(event) => setNext(event.target.value)}
              />
            </Field>
          </FieldGroup>
          {error ? <ErrorState error={error} /> : null}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => handleOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={saving}>
              {saving ? "Changing…" : "Change password"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}

export { ChangePasswordDialog }
