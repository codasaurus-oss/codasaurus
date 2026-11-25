import { useQuery } from "@tanstack/react-query"
import type { ReactNode } from "react"

import { Header } from "@/components/layout/header"
import { navigation } from "@/components/layout/navigation"
import { Badge } from "@/components/ui/badge"
import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
} from "@/components/ui/sidebar"
import { TooltipProvider } from "@/components/ui/tooltip"
import { healthQueryOptions, setupStatusQueryOptions } from "@/lib/api/queries"

function AppSidebar() {
  return (
    <Sidebar collapsible="icon">
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton
              size="lg"
              render={<a href="#/app/dashboard" />}
            >
              <img
                src="/codasaurus-logo.svg"
                alt=""
                className="size-8 rounded-lg"
              />
              <div className="grid flex-1 text-left text-sm leading-tight">
                <span className="truncate font-medium">Codasaurus</span>
                <span className="truncate text-xs text-muted-foreground">
                  Security review
                </span>
              </div>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>
      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupLabel>Platform</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {navigation.map((item) => (
                <SidebarMenuItem key={item.href}>
                  <SidebarMenuButton
                    tooltip={item.title}
                    render={<a href={item.href} />}
                  >
                    <item.icon />
                    <span>{item.title}</span>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>
    </Sidebar>
  )
}

function HealthStrip() {
  const setup = useQuery(setupStatusQueryOptions)
  const health = useQuery(healthQueryOptions)

  return (
    <footer
      data-slot="health-strip"
      className="flex flex-wrap items-center gap-x-4 gap-y-1 border-t px-4 py-2 text-xs text-muted-foreground"
    >
      {setup.isError ? (
        <span>Status unavailable</span>
      ) : setup.data ? (
        <>
          <span className="flex items-center gap-1.5">
            Database
            <Badge
              variant="outline"
              className={
                setup.data.database
                  ? "border-success/30 bg-success/10 text-success"
                  : "border-warning/30 bg-warning/10 text-warning"
              }
            >
              {setup.data.database ? "Connected" : "Unavailable"}
            </Badge>
          </span>
          <span className="flex items-center gap-1.5">
            GitHub App
            <Badge
              variant="outline"
              className={
                setup.data.github
                  ? "border-success/30 bg-success/10 text-success"
                  : "border-warning/30 bg-warning/10 text-warning"
              }
            >
              {setup.data.github ? "Configured" : "Not configured"}
            </Badge>
          </span>
          <span className="flex items-center gap-1.5">
            LLM
            <Badge variant="outline">
              {setup.data.llm ? "Setup complete" : "Not configured"}
            </Badge>
          </span>
          {health.data ? (
            <span className="flex items-center gap-1.5">
              Mode
              <Badge
                variant="outline"
                className={
                  health.data.offline_mode
                    ? "border-warning/30 bg-warning/10 text-warning"
                    : undefined
                }
              >
                {health.data.offline_mode ? "Offline · Tier-1 only" : "Online"}
              </Badge>
            </span>
          ) : null}
        </>
      ) : null}
    </footer>
  )
}

function AppShell({ children }: { children: ReactNode }) {
  return (
    <TooltipProvider>
      <SidebarProvider defaultOpen={false}>
        <AppSidebar />
        <SidebarInset>
          <Header />
          <div className="flex flex-1 flex-col gap-4 p-4 md:p-6">
            {children}
          </div>
          <HealthStrip />
        </SidebarInset>
      </SidebarProvider>
    </TooltipProvider>
  )
}

export { AppShell }
