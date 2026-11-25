import { useQuery, useQueryClient } from "@tanstack/react-query"
import { LinkIcon, UserPlusIcon } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/common/confirm-dialog"
import { ErrorState } from "@/components/common/error-state"
import { LoadingState } from "@/components/common/loading-state"
import { PageHeader } from "@/components/layout/page-header"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { InviteDialog } from "@/features/users/invite-dialog"
import { apiFetch } from "@/lib/api/client"
import {
  invitesQueryOptions,
  queryKeys,
  usersQueryOptions,
} from "@/lib/api/queries"
import type { Invite, Role, User } from "@/lib/api/schemas"
import { useAuth } from "@/lib/auth/auth-context"
import { can, roleItems, roleLabel } from "@/lib/auth/roles"
import { formatWhen } from "@/lib/format"

type TabId = "users" | "invites"

function UsersRoute() {
  const { user, refresh } = useAuth()
  const queryClient = useQueryClient()
  const isOwner = can(user?.role, "owner")
  const users = useQuery(usersQueryOptions)
  const invites = useQuery({ ...invitesQueryOptions, enabled: isOwner })
  const [tab, setTab] = useState<TabId>("users")
  const [roleChange, setRoleChange] = useState<{
    user: User
    role: Role
  } | null>(null)
  const [deleteTarget, setDeleteTarget] = useState<User | null>(null)
  const [transferTarget, setTransferTarget] = useState<User | null>(null)
  const [revokeTarget, setRevokeTarget] = useState<Invite | null>(null)
  const [inviteOpen, setInviteOpen] = useState(false)
  const [actionError, setActionError] = useState<unknown>(null)

  async function refreshRoster() {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.users }),
      queryClient.invalidateQueries({ queryKey: queryKeys.invites }),
    ])
    await refresh(true)
  }

  async function changeRole(target: User, role: Role) {
    setActionError(null)
    try {
      await apiFetch(`/api/users/${target.id}`, {
        method: "PATCH",
        body: JSON.stringify({ role }),
      })
      toast.success(`${target.email} is now ${roleLabel(role)}`)
      await refreshRoster()
    } catch (caught) {
      setActionError(caught)
    }
  }

  async function removeUser(target: User) {
    setActionError(null)
    try {
      await apiFetch(`/api/users/${target.id}`, { method: "DELETE" })
      toast.success(`${target.email} removed`)
      await refreshRoster()
    } catch (caught) {
      setActionError(caught)
    }
  }

  async function transferBootstrap(target: User) {
    setActionError(null)
    try {
      await apiFetch(`/api/users/${target.id}/transfer-bootstrap`, {
        method: "POST",
      })
      toast.success(`Bootstrap owner transferred to ${target.email}`)
      await refreshRoster()
    } catch (caught) {
      setActionError(caught)
    }
  }

  async function revokeInvite(target: Invite) {
    setActionError(null)
    try {
      await apiFetch(`/api/users/invites/${target.id}`, { method: "DELETE" })
      toast.success("Invite revoked")
      await queryClient.invalidateQueries({ queryKey: queryKeys.invites })
    } catch (caught) {
      setActionError(caught)
    }
  }

  if (users.isPending) {
    return <LoadingState rows={6} />
  }
  if (users.isError) {
    return (
      <ErrorState error={users.error} onRetry={() => void users.refetch()} />
    )
  }

  const roster = users.data.users
  const ownerCount = roster.filter((member) => member.role === "owner").length

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Users"
        description="Team members and their access to this instance."
      />

      {actionError ? <ErrorState error={actionError} /> : null}

      <Tabs value={tab} onValueChange={(value) => setTab(value as TabId)}>
        <TabsList>
          <TabsTrigger value="users">Users</TabsTrigger>
          {isOwner ? <TabsTrigger value="invites">Invites</TabsTrigger> : null}
        </TabsList>

        <TabsContent value="users">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Email</TableHead>
                <TableHead>Role</TableHead>
                <TableHead>Provider</TableHead>
                <TableHead>Bootstrap</TableHead>
                <TableHead>Created</TableHead>
                {isOwner ? (
                  <TableHead className="text-right">Actions</TableHead>
                ) : null}
              </TableRow>
            </TableHeader>
            <TableBody>
              {roster.map((member) => {
                const isSelf = member.email === user?.email
                const isLastOwner =
                  member.role === "owner" && ownerCount <= 1
                const canTransferBootstrap =
                  isOwner &&
                  Boolean(user?.is_bootstrap) &&
                  !member.is_bootstrap &&
                  member.role === "owner"
                const canRemove =
                  !member.is_bootstrap && !isSelf && !isLastOwner
                return (
                  <TableRow key={member.id}>
                    <TableCell className="font-medium">
                      {member.email}
                    </TableCell>
                    <TableCell>
                      {isOwner ? (
                        <Select
                          items={roleItems}
                          value={member.role}
                          disabled={isLastOwner}
                          onValueChange={(value) => {
                            if (value && value !== member.role) {
                              setRoleChange({
                                user: member,
                                role: value as Role,
                              })
                            }
                          }}
                        >
                          <SelectTrigger
                            aria-label={`Role for ${member.email}`}
                            className="w-32"
                          >
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
                      ) : (
                        <Badge variant="outline">
                          {roleLabel(member.role)}
                        </Badge>
                      )}
                    </TableCell>
                    <TableCell className="text-muted-foreground capitalize">
                      {member.auth_provider}
                    </TableCell>
                    <TableCell>
                      {member.is_bootstrap ? (
                        <Badge>Bootstrap</Badge>
                      ) : (
                        <span className="text-muted-foreground">—</span>
                      )}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatWhen(member.created_at)}
                    </TableCell>
                    {isOwner ? (
                      <TableCell className="text-right">
                        <div className="flex justify-end gap-1">
                          {canTransferBootstrap ? (
                            <Button
                              variant="outline"
                              size="sm"
                              aria-label={`Transfer bootstrap to ${member.email}`}
                              onClick={() => setTransferTarget(member)}
                            >
                              Transfer bootstrap
                            </Button>
                          ) : null}
                          {canRemove ? (
                            <Button
                              variant="ghost"
                              size="sm"
                              aria-label={`Remove ${member.email}`}
                              onClick={() => setDeleteTarget(member)}
                            >
                              Remove
                            </Button>
                          ) : null}
                        </div>
                      </TableCell>
                    ) : null}
                  </TableRow>
                )
              })}
            </TableBody>
          </Table>
        </TabsContent>

        {isOwner ? (
          <TabsContent value="invites">
            <div className="flex flex-col gap-4">
              <div className="flex justify-end">
                <Button onClick={() => setInviteOpen(true)}>
                  <UserPlusIcon />
                  Create invite
                </Button>
              </div>

              {invites.isPending ? (
                <LoadingState rows={3} />
              ) : invites.isError ? (
                <ErrorState
                  error={invites.error}
                  onRetry={() => void invites.refetch()}
                />
              ) : invites.data.invites.length === 0 ? (
                <Empty className="border">
                  <EmptyHeader>
                    <EmptyMedia variant="icon">
                      <LinkIcon />
                    </EmptyMedia>
                    <EmptyTitle>No pending invites</EmptyTitle>
                    <EmptyDescription>
                      Create a link to invite a teammate to this instance.
                    </EmptyDescription>
                  </EmptyHeader>
                </Empty>
              ) : (
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Email</TableHead>
                      <TableHead>Role</TableHead>
                      <TableHead>Created by</TableHead>
                      <TableHead>Expires</TableHead>
                      <TableHead className="text-right">Actions</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {invites.data.invites.map((invite) => (
                      <TableRow key={invite.id}>
                        <TableCell className="font-medium">
                          {invite.email ?? "Any email"}
                        </TableCell>
                        <TableCell>
                          <Badge variant="outline">
                            {roleLabel(invite.role)}
                          </Badge>
                        </TableCell>
                        <TableCell className="text-muted-foreground">
                          {invite.created_by ?? "—"}
                        </TableCell>
                        <TableCell className="text-muted-foreground">
                          {formatWhen(invite.expires_at)}
                        </TableCell>
                        <TableCell className="text-right">
                          <Button
                            variant="ghost"
                            size="sm"
                            aria-label={`Revoke invite for ${invite.email ?? "any email"}`}
                            onClick={() => setRevokeTarget(invite)}
                          >
                            Revoke
                          </Button>
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              )}
            </div>
          </TabsContent>
        ) : null}
      </Tabs>

      <ConfirmDialog
        open={roleChange !== null}
        onOpenChange={(open) => {
          if (!open) {
            setRoleChange(null)
          }
        }}
        title="Change role"
        description={
          roleChange
            ? `Change ${roleChange.user.email} from ${roleLabel(roleChange.user.role)} to ${roleLabel(roleChange.role)}?`
            : ""
        }
        onConfirm={() => {
          if (roleChange) {
            void changeRole(roleChange.user, roleChange.role)
          }
        }}
      />

      <ConfirmDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) {
            setDeleteTarget(null)
          }
        }}
        title="Remove user"
        description={
          deleteTarget
            ? `Remove ${deleteTarget.email} from this instance? They lose access immediately and this cannot be undone.`
            : ""
        }
        onConfirm={() => {
          if (deleteTarget) {
            void removeUser(deleteTarget)
          }
        }}
        destructive
      />

      <ConfirmDialog
        open={transferTarget !== null}
        onOpenChange={(open) => {
          if (!open) {
            setTransferTarget(null)
          }
        }}
        title="Transfer bootstrap owner"
        description={
          transferTarget
            ? `Make ${transferTarget.email} the bootstrap owner of this instance? Your account loses the flag and can never transfer it back.`
            : ""
        }
        onConfirm={() => {
          if (transferTarget) {
            void transferBootstrap(transferTarget)
          }
        }}
        destructive
        confirmLabel="Transfer bootstrap"
        confirmationText={transferTarget?.email}
      />

      <ConfirmDialog
        open={revokeTarget !== null}
        onOpenChange={(open) => {
          if (!open) {
            setRevokeTarget(null)
          }
        }}
        title="Revoke invite"
        description={
          revokeTarget
            ? `Revoke the invite for ${revokeTarget.email ?? "any email"}? The link stops working immediately.`
            : ""
        }
        onConfirm={() => {
          if (revokeTarget) {
            void revokeInvite(revokeTarget)
          }
        }}
        destructive
        confirmLabel="Revoke"
      />

      <InviteDialog open={inviteOpen} onOpenChange={setInviteOpen} />
    </div>
  )
}

export default UsersRoute
