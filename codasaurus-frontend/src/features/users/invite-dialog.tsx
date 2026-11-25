import { useQueryClient } from "@tanstack/react-query"
import { CopyIcon } from "lucide-react"
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
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { apiFetch } from "@/lib/api/client"
import { queryKeys } from "@/lib/api/queries"
import {
  createdInviteSchema,
  type CreatedInvite,
  type Role,
} from "@/lib/api/schemas"
import { roleItems, roleLabel } from "@/lib/auth/roles"
import { formatWhen } from "@/lib/format"

type InviteDialogProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
}

function InviteDialog({ open, onOpenChange }: InviteDialogProps) {
  const queryClient = useQueryClient()
  const [email, setEmail] = useState("")
  const [role, setRole] = useState<Role>("viewer")
  const [created, setCreated] = useState<CreatedInvite | null>(null)
  const [error, setError] = useState<unknown>(null)
  const [creating, setCreating] = useState(false)

  function reset() {
    setEmail("")
    setRole("viewer")
    setCreated(null)
    setError(null)
  }

  function handleOpenChange(value: boolean) {
    if (!value) {
      reset()
    }
    onOpenChange(value)
  }

  async function create(event: FormEvent) {
    event.preventDefault()
    setError(null)
    setCreating(true)
    const address = email.trim()
    const body = address ? { email: address, role } : { role }
    try {
      const invite = await apiFetch(
        "/api/users/invites",
        { method: "POST", body: JSON.stringify(body) },
        createdInviteSchema
      )
      setCreated(invite)
      toast.success("Invite created")
      await queryClient.invalidateQueries({ queryKey: queryKeys.invites })
    } catch (caught) {
      setError(caught)
    } finally {
      setCreating(false)
    }
  }

  async function copyLink() {
    if (!created) {
      return
    }
    try {
      await navigator.clipboard.writeText(created.url)
      toast.success("Invite link copied")
    } catch {
      toast.error("Could not copy. Select the link and copy it manually.")
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
        {created ? (
          <div className="flex flex-col gap-4">
            <DialogHeader>
              <DialogTitle>Invite created</DialogTitle>
              <DialogDescription>
                Share this link. It expires {formatWhen(created.expires_at)}.
              </DialogDescription>
            </DialogHeader>
            <Field>
              <FieldLabel htmlFor="invite-url">Invite link</FieldLabel>
              <div className="flex items-center gap-2">
                <Input
                  id="invite-url"
                  aria-label="Invite URL"
                  readOnly
                  value={created.url}
                  onFocus={(event) => event.currentTarget.select()}
                />
                <Button
                  type="button"
                  variant="outline"
                  size="icon"
                  aria-label="Copy invite link"
                  onClick={() => void copyLink()}
                >
                  <CopyIcon />
                </Button>
              </div>
              <FieldDescription>
                Joins as {roleLabel(created.role)}. The link works once.
              </FieldDescription>
            </Field>
            <DialogFooter>
              <Button type="button" onClick={() => handleOpenChange(false)}>
                Done
              </Button>
            </DialogFooter>
          </div>
        ) : (
          <form
            className="flex flex-col gap-4"
            onSubmit={(event) => void create(event)}
          >
            <DialogHeader>
              <DialogTitle>Create invite</DialogTitle>
              <DialogDescription>
                Generate a single-use link that expires in 7 days.
              </DialogDescription>
            </DialogHeader>
            <FieldGroup>
              <Field>
                <FieldLabel htmlFor="invite-email">Email (optional)</FieldLabel>
                <Input
                  id="invite-email"
                  type="email"
                  autoComplete="off"
                  placeholder="any address"
                  value={email}
                  onChange={(event) => setEmail(event.target.value)}
                />
                <FieldDescription>
                  Lock the invite to one address, or leave it blank for anyone
                  with the link.
                </FieldDescription>
              </Field>
              <Field>
                <FieldLabel htmlFor="invite-role">Role</FieldLabel>
                <Select
                  items={roleItems}
                  value={role}
                  onValueChange={(value) => {
                    if (value) {
                      setRole(value as Role)
                    }
                  }}
                >
                  <SelectTrigger id="invite-role" aria-label="Invite role">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {roleItems.map((item) => (
                      <SelectItem key={item.value} value={item.value}>
                        {item.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
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
              <Button type="submit" disabled={creating}>
                {creating ? "Creating…" : "Create link"}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  )
}

export { InviteDialog }
