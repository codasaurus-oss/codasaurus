import { zodResolver } from "@hookform/resolvers/zod"
import { useQuery } from "@tanstack/react-query"
import { CircleAlertIcon } from "lucide-react"
import { useState } from "react"
import { useForm } from "react-hook-form"
import { useNavigate, useParams } from "react-router"
import { z } from "zod"

import { EmptyState } from "@/components/common/empty-state"
import { ErrorState } from "@/components/common/error-state"
import { LoadingState } from "@/components/common/loading-state"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  Field,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { apiFetch } from "@/lib/api/client"
import { ApiError } from "@/lib/api/errors"
import { inviteQueryOptions } from "@/lib/api/queries"
import type { InviteInfo } from "@/lib/api/schemas"
import { useAuth } from "@/lib/auth/auth-context"
import { roleLabel } from "@/lib/auth/roles"

function acceptErrorMessage(error: unknown) {
  if (error instanceof ApiError) {
    return error.status === 429
      ? `${error.message} Please try again shortly.`
      : error.message
  }
  return error instanceof Error
    ? error.message
    : "Could not accept the invite."
}

function acceptSchema(invite: InviteInfo) {
  return z
    .object({
      email: invite.email_locked
        ? z.string()
        : z.email("Enter a valid email address"),
      password: z.string().min(10, "Password must be at least 10 characters"),
    })
    .refine(
      (values) =>
        values.password.toLowerCase() !==
        (invite.email ?? values.email).toLowerCase(),
      {
        message: "Password must not be the same as your email",
        path: ["password"],
      }
    )
}

type AcceptValues = z.infer<ReturnType<typeof acceptSchema>>

function InviteDetails({ invite }: { invite: InviteInfo }) {
  return (
    <div className="flex flex-col gap-3 rounded-lg border p-3 text-sm">
      <div className="flex items-center justify-between gap-4">
        <span className="text-muted-foreground">Email</span>
        <span className="flex items-center gap-2 font-medium">
          {invite.email ?? "Chosen when you accept"}
          <Badge variant="secondary">
            {invite.email_locked ? "Locked" : "Not locked"}
          </Badge>
        </span>
      </div>
      <div className="flex items-center justify-between gap-4">
        <span className="text-muted-foreground">Role</span>
        <Badge>{roleLabel(invite.role)}</Badge>
      </div>
      <div className="flex items-center justify-between gap-4">
        <span className="text-muted-foreground">Expires</span>
        <span className="font-medium">
          {new Date(invite.expires_at).toLocaleString()}
        </span>
      </div>
    </div>
  )
}

function AcceptInviteForm({
  token,
  invite,
}: {
  token: string
  invite: InviteInfo
}) {
  const { refresh } = useAuth()
  const navigate = useNavigate()
  const [error, setError] = useState<string | null>(null)
  const form = useForm<AcceptValues>({
    resolver: zodResolver(acceptSchema(invite)),
    defaultValues: { email: invite.email ?? "", password: "" },
  })

  async function onSubmit(values: AcceptValues) {
    setError(null)
    try {
      const body = invite.email_locked
        ? { password: values.password }
        : { email: values.email, password: values.password }
      await apiFetch(`/api/auth/invite/${token}/accept`, {
        method: "POST",
        body: JSON.stringify(body),
      })
      await refresh()
      await navigate("/app/dashboard", { replace: true })
    } catch (caught) {
      setError(acceptErrorMessage(caught))
    }
  }

  const errors = form.formState.errors

  return (
    <Card>
      <CardHeader>
        <CardTitle>Accept your invitation</CardTitle>
        <CardDescription>
          You have been invited to join Codasaurus.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <InviteDetails invite={invite} />
        {error ? (
          <Alert variant="destructive">
            <CircleAlertIcon />
            <AlertTitle>Could not accept the invite</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        ) : null}
        <form onSubmit={form.handleSubmit(onSubmit)} noValidate>
          <FieldGroup>
            <Field data-invalid={Boolean(errors.email)}>
              <FieldLabel htmlFor="invite-email">Email</FieldLabel>
              <Input
                id="invite-email"
                type="email"
                autoComplete="email"
                readOnly={invite.email_locked}
                aria-invalid={Boolean(errors.email)}
                aria-describedby={
                  errors.email
                    ? "invite-email-description invite-email-error"
                    : "invite-email-description"
                }
                {...form.register("email")}
              />
              <FieldDescription id="invite-email-description">
                {invite.email_locked
                  ? "This invite is locked to the email above."
                  : "Use the email address you want to sign in with."}
              </FieldDescription>
              <FieldError id="invite-email-error" errors={[errors.email]} />
            </Field>
            <Field data-invalid={Boolean(errors.password)}>
              <FieldLabel htmlFor="invite-password">Password</FieldLabel>
              <Input
                id="invite-password"
                type="password"
                autoComplete="new-password"
                aria-invalid={Boolean(errors.password)}
                aria-describedby={
                  errors.password
                    ? "invite-password-description invite-password-error"
                    : "invite-password-description"
                }
                {...form.register("password")}
              />
              <FieldDescription id="invite-password-description">
                At least 10 characters and different from your email.
              </FieldDescription>
              <FieldError id="invite-password-error" errors={[errors.password]} />
            </Field>
            <Field>
              <Button type="submit" disabled={form.formState.isSubmitting}>
                {form.formState.isSubmitting ? "Joining..." : "Accept invite"}
              </Button>
            </Field>
          </FieldGroup>
        </form>
      </CardContent>
    </Card>
  )
}

function InviteRoute() {
  const { token = "" } = useParams()
  const invite = useQuery(inviteQueryOptions(token))

  let content
  if (invite.isPending) {
    content = <LoadingState rows={4} />
  } else if (invite.isError) {
    // The backend answers 404 for expired, used and unknown tokens alike.
    content =
      invite.error instanceof ApiError && invite.error.status === 404 ? (
        <EmptyState
          title="Invite unavailable"
          description={invite.error.message}
          action={
            <Button render={<a href="#/login" />}>Back to sign in</Button>
          }
        />
      ) : (
        <ErrorState
          error={invite.error}
          onRetry={() => void invite.refetch()}
        />
      )
  } else {
    content = <AcceptInviteForm token={token} invite={invite.data} />
  }

  return (
    <div className="flex min-h-svh items-center justify-center p-6">
      <div className="w-full max-w-md">{content}</div>
    </div>
  )
}

export default InviteRoute
