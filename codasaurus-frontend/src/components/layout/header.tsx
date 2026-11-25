import { useEffect, useState } from "react"
import {
  KeyRoundIcon,
  LogOutIcon,
  MonitorIcon,
  MoonIcon,
  SearchIcon,
  SunIcon,
  SunMoonIcon,
} from "lucide-react"
import { useLocation } from "react-router"
import { toast } from "sonner"

import { Avatar, AvatarFallback } from "@/components/ui/avatar"
import { navigation } from "@/components/layout/navigation"
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb"
import { Button } from "@/components/ui/button"
import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Separator } from "@/components/ui/separator"
import { SidebarTrigger } from "@/components/ui/sidebar"
import { ChangePasswordDialog } from "@/features/users/change-password-dialog"
import { apiFetch } from "@/lib/api/client"
import { useAuth } from "@/lib/auth/auth-context"
import { roleLabel } from "@/lib/auth/roles"
import { useTheme, type Theme } from "@/lib/theme/theme-provider"

function Header() {
  const [commandOpen, setCommandOpen] = useState(false)
  const [passwordOpen, setPasswordOpen] = useState(false)
  const { theme, setTheme } = useTheme()
  const { user, setUser } = useAuth()
  const location = useLocation()
  const segments = location.pathname.split("/").filter(Boolean)
  const section = navigation.find(
    (item) => item.href.slice(1) === `/${segments.slice(0, 2).join("/")}`
  )
  const currentPage =
    segments.length > 2
      ? section?.title === "Reviews"
        ? "Review detail"
        : `${section?.title ?? "Details"} detail`
      : (section?.title ?? "Not found")
  const initials = (user?.email ?? "?")
    .split("@")[0]
    .split(/[._-]/)
    .map((part) => part[0] ?? "")
    .join("")
    .slice(0, 2)
    .toUpperCase()
  const avatarTone = ["bg-muted", "bg-secondary", "bg-accent"][
    (user?.email ?? "").length % 3
  ]

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() === "k" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault()
        setCommandOpen((open) => !open)
      }
    }
    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [])

  async function logout() {
    try {
      await apiFetch("/api/auth/logout", { method: "POST" })
    } catch {
      toast.error("Could not sign out on the server. You may still be signed in here.")
    } finally {
      // The guard redirects to login once the local user is cleared.
      setUser(null)
    }
  }

  return (
    <header className="flex h-16 shrink-0 items-center gap-2 border-b">
      <div className="flex items-center gap-2 px-4">
        <SidebarTrigger className="-ml-1" />
        <Separator
          orientation="vertical"
          className="mr-2 data-[orientation=vertical]:h-4"
        />
        <Breadcrumb>
          <BreadcrumbList>
            <BreadcrumbItem className="hidden md:block">
              <BreadcrumbLink href="#/app/dashboard">Codasaurus</BreadcrumbLink>
            </BreadcrumbItem>
            <BreadcrumbSeparator className="hidden md:block" />
            <BreadcrumbItem>
              <BreadcrumbPage>{currentPage}</BreadcrumbPage>
            </BreadcrumbItem>
          </BreadcrumbList>
        </Breadcrumb>
      </div>
      <div className="ml-auto flex items-center gap-2 px-4">
        <Button
          variant="outline"
          size="sm"
          className="text-muted-foreground"
          aria-label="Search"
          onClick={() => setCommandOpen(true)}
        >
          <SearchIcon />
          <span className="hidden sm:inline">Search</span>
          <span className="hidden text-xs sm:inline">⌘K</span>
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button
                variant="ghost"
                size="icon"
                className="rounded-full"
                aria-label="Theme"
              />
            }
          >
            <SunMoonIcon />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-40">
            <DropdownMenuRadioGroup
              value={theme}
              onValueChange={(value) => setTheme(value as Theme)}
            >
              <DropdownMenuRadioItem value="light">
                <SunIcon />
                Light
              </DropdownMenuRadioItem>
              <DropdownMenuRadioItem value="dark">
                <MoonIcon />
                Dark
              </DropdownMenuRadioItem>
              <DropdownMenuRadioItem value="system">
                <MonitorIcon />
                System
              </DropdownMenuRadioItem>
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        {user ? (
          <DropdownMenu>
            <DropdownMenuTrigger
              render={
                <Button variant="ghost" size="sm" className="gap-2" aria-label="Account" />
              }
            >
              <Avatar size="sm" className={avatarTone}>
                <AvatarFallback>{initials || "?"}</AvatarFallback>
              </Avatar>
              <span className="hidden max-w-40 truncate sm:inline">
                {user.email}
              </span>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-56">
              <DropdownMenuGroup>
                <DropdownMenuLabel className="flex flex-col gap-0.5">
                  <span className="truncate text-sm font-medium text-foreground">
                    {user.email}
                  </span>
                  <span>{roleLabel(user.role)}</span>
                </DropdownMenuLabel>
              </DropdownMenuGroup>
              <DropdownMenuSeparator />
              {(user.auth_provider ?? "local") === "local" ? (
                <DropdownMenuItem onClick={() => setPasswordOpen(true)}>
                  <KeyRoundIcon />
                  Change password
                </DropdownMenuItem>
              ) : null}
              <DropdownMenuSeparator />
              <DropdownMenuItem onClick={() => void logout()}>
                <LogOutIcon />
                Log out
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        ) : null}
        {user ? (
          <ChangePasswordDialog
            open={passwordOpen}
            onOpenChange={setPasswordOpen}
            email={user.email}
          />
        ) : null}
      </div>
      <CommandDialog open={commandOpen} onOpenChange={setCommandOpen}>
        <Command>
          <CommandInput placeholder="Search pages..." />
          <CommandList>
            <CommandEmpty>No results found.</CommandEmpty>
            <CommandGroup heading="Navigation">
              {navigation.map((item) => (
                <CommandItem
                  key={item.href}
                  value={item.title}
                  onSelect={() => {
                    window.location.hash = item.href
                    setCommandOpen(false)
                  }}
                >
                  <item.icon />
                  {item.title}
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </CommandDialog>
    </header>
  )
}

export { Header }
