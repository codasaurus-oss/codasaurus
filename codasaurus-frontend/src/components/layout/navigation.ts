import {
  FolderGit2Icon,
  LayoutDashboardIcon,
  ScrollTextIcon,
  ServerIcon,
  SettingsIcon,
  ShieldCheckIcon,
  SparklesIcon,
  UsersIcon,
  type LucideIcon,
} from "lucide-react"

type NavigationItem = {
  title: string
  href: string
  icon: LucideIcon
}

const navigation: NavigationItem[] = [
  { title: "Dashboard", href: "#/app/dashboard", icon: LayoutDashboardIcon },
  { title: "Repositories", href: "#/app/repos", icon: FolderGit2Icon },
  { title: "Reviews", href: "#/app/reviews", icon: ShieldCheckIcon },
  { title: "Learning", href: "#/app/learning", icon: SparklesIcon },
  { title: "MCP servers", href: "#/app/mcp", icon: ServerIcon },
  { title: "Users", href: "#/app/users", icon: UsersIcon },
  { title: "Audit log", href: "#/app/audit", icon: ScrollTextIcon },
  { title: "Settings", href: "#/app/settings", icon: SettingsIcon },
]

export { navigation }
export type { NavigationItem }
