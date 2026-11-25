import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/common/confirm-dialog"
import { EmptyState } from "@/components/common/empty-state"
import { ErrorState } from "@/components/common/error-state"
import { LoadingState } from "@/components/common/loading-state"
import { PageHeader } from "@/components/layout/page-header"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { apiFetch } from "@/lib/api/client"
import { ApiError } from "@/lib/api/errors"
import { mcpServersQueryOptions, queryKeys } from "@/lib/api/queries"
import { mcpTestResultSchema, type McpServer } from "@/lib/api/schemas"
import { useAuth } from "@/lib/auth/auth-context"
import { can } from "@/lib/auth/roles"

const FORBIDDEN = new ApiError(
  403,
  "Only maintainers and owners can manage MCP servers."
)

type ServerDialogProps = {
  server: McpServer
  onOpenChange: (open: boolean) => void
}

function ServerDialog({ server, onOpenChange }: ServerDialogProps) {
  const queryClient = useQueryClient()
  const [enabled, setEnabled] = useState(server.enabled)
  const [apiKey, setApiKey] = useState("")
  const [baseUrl, setBaseUrl] = useState(server.base_url)
  const [name, setName] = useState(server.name)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<unknown>(null)

  const baseUrlMissing = server.is_custom && baseUrl.trim() === ""

  async function save() {
    setSaving(true)
    setError(null)
    try {
      const body: Record<string, unknown> = { enabled }
      if (apiKey.trim()) {
        body.api_key = apiKey.trim()
      }
      if (server.is_custom) {
        body.base_url = baseUrl.trim()
        if (name.trim()) {
          body.name = name.trim()
        }
      }
      await apiFetch(`/api/mcp/servers/${server.id}`, {
        method: "PUT",
        body: JSON.stringify(body),
      })
      toast.success(`${server.name} updated`)
      await queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers })
      onOpenChange(false)
    } catch (caught) {
      setError(caught)
    } finally {
      setSaving(false)
    }
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) {
          onOpenChange(false)
        }
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Edit {server.name}</DialogTitle>
          <DialogDescription>
            Changes apply to every review that uses this server.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3">
          <div className="flex items-center justify-between gap-2">
            <label htmlFor="mcp-enabled">Enabled</label>
            <Switch
              id="mcp-enabled"
              aria-label="Enabled"
              checked={enabled}
              onCheckedChange={setEnabled}
            />
          </div>

          <div className="flex flex-col gap-1">
            <label htmlFor="mcp-api-key" className="text-muted-foreground">
              API key
            </label>
            <Input
              id="mcp-api-key"
              aria-label="API key"
              type="password"
              autoComplete="off"
              placeholder={
                server.key_configured
                  ? "Leave blank to keep the stored key"
                  : "Optional API key"
              }
              value={apiKey}
              onChange={(event) => setApiKey(event.target.value)}
            />
          </div>

          {server.is_custom ? (
            <>
              <div className="flex flex-col gap-1">
                <label htmlFor="mcp-base-url" className="text-muted-foreground">
                  Base URL
                </label>
                <Input
                  id="mcp-base-url"
                  aria-label="Base URL"
                  value={baseUrl}
                  onChange={(event) => setBaseUrl(event.target.value)}
                />
              </div>
              <div className="flex flex-col gap-1">
                <label htmlFor="mcp-name" className="text-muted-foreground">
                  Name
                </label>
                <Input
                  id="mcp-name"
                  aria-label="Name"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                />
              </div>
            </>
          ) : null}

          {error ? <ErrorState error={error} /> : null}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            onClick={() => void save()}
            disabled={saving || baseUrlMissing}
          >
            {saving ? "Saving…" : "Save"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function McpConsole() {
  const { user } = useAuth()
  const queryClient = useQueryClient()
  const servers = useQuery(mcpServersQueryOptions)
  const [testing, setTesting] = useState<string | null>(null)
  const [editTarget, setEditTarget] = useState<McpServer | null>(null)
  const [deleteTarget, setDeleteTarget] = useState<McpServer | null>(null)
  const isOwner = can(user?.role, "owner")

  async function test(server: McpServer) {
    setTesting(server.id)
    try {
      const result = await apiFetch(
        `/api/mcp/${server.id}/test`,
        { method: "POST" },
        mcpTestResultSchema
      )
      toast.success(
        `${server.name}: ${result.tool_count} ${
          result.tool_count === 1 ? "tool" : "tools"
        } available`
      )
    } catch (caught) {
      toast.error(caught instanceof Error ? caught.message : "Test failed")
    } finally {
      setTesting(null)
    }
  }

  async function remove(server: McpServer) {
    try {
      await apiFetch(`/api/mcp/servers/${server.id}`, { method: "DELETE" })
      toast.success(`${server.name} removed`)
      await queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers })
    } catch (caught) {
      toast.error(caught instanceof Error ? caught.message : "Delete failed")
    }
  }

  if (servers.isPending) {
    return <LoadingState rows={6} />
  }
  if (servers.isError) {
    return (
      <ErrorState
        error={servers.error}
        onRetry={() => void servers.refetch()}
      />
    )
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="MCP servers"
        description="Model Context Protocol connectors available to reviews."
      />

      {servers.data.servers.length === 0 ? (
        <EmptyState
          title="No MCP servers"
          description="No catalog or custom MCP servers are configured."
        />
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Name</TableHead>
              <TableHead>Base URL</TableHead>
              <TableHead>Enabled</TableHead>
              <TableHead>Key</TableHead>
              <TableHead>Type</TableHead>
              <TableHead className="text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {servers.data.servers.map((server) => (
              <TableRow key={server.id}>
                <TableCell className="font-medium">{server.name}</TableCell>
                <TableCell className="text-muted-foreground">
                  {server.base_url}
                </TableCell>
                <TableCell>
                  <Badge variant={server.enabled ? "default" : "outline"}>
                    {server.enabled ? "Enabled" : "Disabled"}
                  </Badge>
                </TableCell>
                <TableCell>
                  <Badge variant={server.key_configured ? "default" : "outline"}>
                    {server.key_configured ? "Configured" : "Not set"}
                  </Badge>
                </TableCell>
                <TableCell>
                  <Badge variant="outline">
                    {server.is_custom ? "Custom" : "Catalog"}
                  </Badge>
                </TableCell>
                <TableCell className="text-right">
                  <div className="flex justify-end gap-1">
                    <Button
                      variant="outline"
                      size="sm"
                      aria-label={`Test ${server.name}`}
                      disabled={testing === server.id}
                      onClick={() => void test(server)}
                    >
                      {testing === server.id ? "Testing…" : "Test"}
                    </Button>
                    {isOwner ? (
                      <Button
                        variant="ghost"
                        size="sm"
                        aria-label={`Edit ${server.name}`}
                        onClick={() => setEditTarget(server)}
                      >
                        Edit
                      </Button>
                    ) : null}
                    {isOwner ? (
                      <Button
                        variant="ghost"
                        size="sm"
                        aria-label={`Delete ${server.name}`}
                        onClick={() => setDeleteTarget(server)}
                      >
                        Delete
                      </Button>
                    ) : null}
                  </div>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}

      {editTarget ? (
        <ServerDialog
          key={editTarget.id}
          server={editTarget}
          onOpenChange={(open) => {
            if (!open) {
              setEditTarget(null)
            }
          }}
        />
      ) : null}

      <ConfirmDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) {
            setDeleteTarget(null)
          }
        }}
        title="Delete server"
        description={
          deleteTarget
            ? `Remove ${deleteTarget.name} from this instance? Its stored key and configuration are deleted and reviews stop using it.`
            : ""
        }
        onConfirm={() => {
          if (deleteTarget) {
            void remove(deleteTarget)
          }
        }}
        destructive
      />
    </div>
  )
}

function McpRoute() {
  const { user, status } = useAuth()

  if (status === "loading") {
    return <LoadingState rows={6} />
  }
  if (!can(user?.role, "maintain")) {
    return <ErrorState error={FORBIDDEN} />
  }
  return <McpConsole />
}

export default McpRoute
