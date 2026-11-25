import { zodResolver } from "@hookform/resolvers/zod"
import { cn } from "cn"
import { CircleAlertIcon } from "lucide-react"
import { useState } from "react"
import { useForm } from "react-hook-form"
import { useNavigate } from "react-router"
import { z } from "zod"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
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
  FieldError,
  FieldGroup,
  FieldLabel,
  FieldSeparator,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { apiFetch } from "@/lib/api/client"
import { ApiError } from "@/lib/api/errors"
import { loginResponseSchema } from "@/lib/api/schemas"
import { useAuth } from "@/lib/auth/auth-context"

const loginSchema = z.object({
  email: z.email("Enter a valid email address"),
  password: z.string().min(1, "Password is required"),
})

type LoginValues = z.infer<typeof loginSchema>

function loginErrorMessage(error: unknown) {
  if (error instanceof ApiError) {
    return error.status === 429
      ? `${error.message} Please try again shortly.`
      : error.message
  }
  return error instanceof Error ? error.message : "Could not sign in."
}

type LoginFormProps = {
  from: string
  oidcEnabled: boolean
  className?: string
}

function LoginForm({ from, oidcEnabled, className }: LoginFormProps) {
  const { refresh } = useAuth()
  const navigate = useNavigate()
  const [error, setError] = useState<string | null>(null)
  const form = useForm<LoginValues>({
    resolver: zodResolver(loginSchema),
    defaultValues: { email: "", password: "" },
  })

  async function onSubmit(values: LoginValues) {
    setError(null)
    try {
      await apiFetch(
        "/api/auth/login",
        { method: "POST", body: JSON.stringify(values) },
        loginResponseSchema
      )
      await refresh()
      await navigate(from, { replace: true })
    } catch (caught) {
      setError(loginErrorMessage(caught))
    }
  }

  const errors = form.formState.errors

  return (
    <div className={cn("flex flex-col gap-6", className)}>
      <Card>
        <CardHeader>
          <CardTitle>Sign in to Codasaurus</CardTitle>
          <CardDescription>
            Enter your email and password to continue.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {error ? (
            <Alert variant="destructive" className="mb-4">
              <CircleAlertIcon />
              <AlertTitle>Sign-in failed</AlertTitle>
              <AlertDescription>{error}</AlertDescription>
            </Alert>
          ) : null}
          <form onSubmit={form.handleSubmit(onSubmit)} noValidate>
            <FieldGroup>
              <Field data-invalid={Boolean(errors.email)}>
                <FieldLabel htmlFor="login-email">Email</FieldLabel>
                <Input
                  id="login-email"
                  type="email"
                  autoComplete="email"
                  aria-invalid={Boolean(errors.email)}
                  aria-describedby={errors.email ? "login-email-error" : undefined}
                  {...form.register("email")}
                />
                <FieldError id="login-email-error" errors={[errors.email]} />
              </Field>
              <Field data-invalid={Boolean(errors.password)}>
                <FieldLabel htmlFor="login-password">Password</FieldLabel>
                <Input
                  id="login-password"
                  type="password"
                  autoComplete="current-password"
                  aria-invalid={Boolean(errors.password)}
                  aria-describedby={
                    errors.password ? "login-password-error" : undefined
                  }
                  {...form.register("password")}
                />
                <FieldError id="login-password-error" errors={[errors.password]} />
              </Field>
              <Field>
                <Button type="submit" disabled={form.formState.isSubmitting}>
                  {form.formState.isSubmitting ? "Signing in..." : "Sign in"}
                </Button>
                {oidcEnabled ? (
                  <>
                    <FieldSeparator>Or continue with</FieldSeparator>
                    <Button
                      variant="outline"
                      render={<a href="/api/auth/oidc/login" />}
                    >
                      Sign in with SSO
                    </Button>
                  </>
                ) : null}
              </Field>
            </FieldGroup>
          </form>
        </CardContent>
      </Card>
    </div>
  )
}

export { LoginForm }
