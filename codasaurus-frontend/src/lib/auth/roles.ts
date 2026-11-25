import type { Role } from "@/lib/api/schemas"

// Mirrors the backend rank (src/api/rbac.rs): owner > maintainer > viewer.
const roleRank: Record<Role, number> = {
  viewer: 1,
  maintainer: 2,
  owner: 3,
}

const roleLabels: Record<Role, string> = {
  viewer: "Viewer",
  maintainer: "Maintainer",
  owner: "Owner",
}

const roleItems = (Object.keys(roleLabels) as Role[]).map((value) => ({
  value,
  label: roleLabels[value],
}))

function roleLabel(role: Role) {
  return roleLabels[role]
}

// Setup endpoints fall back to owner-only once the first owner exists.
const requiredRank = {
  review: roleRank.viewer,
  maintain: roleRank.maintainer,
  setup: roleRank.owner,
  owner: roleRank.owner,
} as const

type Action = keyof typeof requiredRank

function can(role: Role | null | undefined, action: Action): boolean {
  if (!role) {
    return false
  }
  return roleRank[role] >= requiredRank[action]
}

export { can, roleItems, roleLabel }
export type { Action }
