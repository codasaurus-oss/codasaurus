import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { RouterProvider } from "react-router/dom"

import { Toaster } from "@/components/ui/sonner"
import { AuthProvider } from "@/lib/auth/auth-context"
import { useTheme } from "@/lib/theme/theme-provider"
import { router } from "@/routes/router"

// Mounted inside ThemeProvider and outside the router so a toast survives the
// unmount of any route or auth state transition.
function Root({ client }: { client: QueryClient }) {
  const { theme } = useTheme()

  return (
    <>
      <QueryClientProvider client={client}>
        <AuthProvider>
          <RouterProvider router={router} />
        </AuthProvider>
      </QueryClientProvider>
      <Toaster theme={theme} position="bottom-right" />
    </>
  )
}

export { Root }
