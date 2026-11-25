import { useQuery, useQueryClient } from "@tanstack/react-query"
import { FolderGit2Icon } from "lucide-react"
import { useMemo, useState } from "react"
import { toast } from "sonner"

import { ErrorState } from "@/components/common/error-state"
import { LoadingState } from "@/components/common/loading-state"
import { PageHeader } from "@/components/layout/page-header"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ButtonGroup } from "@/components/ui/button-group"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Input } from "@/components/ui/input"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { RepoConfigDialog } from "@/features/repos/repo-config-dialog"
import { apiFetch } from "@/lib/api/client"
import {
  queryKeys,
  reposQueryOptions,
  setupStatusQueryOptions,
} from "@/lib/api/queries"
import { syncResultSchema, type Repo } from "@/lib/api/schemas"
import { useAuth } from "@/lib/auth/auth-context"
import { can } from "@/lib/auth/roles"
import { formatWhen } from "@/lib/format"

function ReposRoute() {
  const { user } = useAuth()
  const queryClient = useQueryClient()
  const repos = useQuery(reposQueryOptions)
  const setup = useQuery(setupStatusQueryOptions)
  const [search, setSearch] = useState("")
  const [selectedId, setSelectedId] = useState<number | null>(null)
  const [syncing, setSyncing] = useState(false)
  const [actionError, setActionError] = useState<unknown>(null)

  const canMaintain = can(user?.role, "maintain")
  const installUrl = setup.data?.github_install_url

  const visibleRepos = useMemo(() => {
    const needle = search.trim().toLowerCase()
    if (!needle) {
      return repos.data ?? []
    }
    return (repos.data ?? []).filter((repo) =>
      repo.full_name.toLowerCase().includes(needle)
    )
  }, [repos.data, search])

  async function syncRepos() {
    setSyncing(true)
    setActionError(null)
    try {
      const result = await apiFetch(
        "/api/repos/sync",
        { method: "POST", timeoutMs: 120_000 },
        syncResultSchema
      )
      if (result.synced === 0) {
        toast.success(
          "No repositories were found. Install the GitHub App to connect one."
        )
      } else {
        toast.success(
          `Synced ${result.synced} ${
            result.synced === 1 ? "repository" : "repositories"
          }`
        )
      }
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.repos }),
        queryClient.invalidateQueries({ queryKey: queryKeys.stats }),
      ])
    } catch (caught) {
      setActionError(caught)
    } finally {
      setSyncing(false)
    }
  }

  async function setActive(repo: Repo, active: boolean) {
    setActionError(null)
    try {
      await apiFetch(`/api/repos/${repo.id}`, {
        method: "PUT",
        body: JSON.stringify({ config_json: repo.config_json ?? "", active }),
      })
      toast.success(active ? "Repository enabled" : "Repository paused")
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.repos }),
        queryClient.invalidateQueries({ queryKey: queryKeys.stats }),
      ])
    } catch (caught) {
      setActionError(caught)
    }
  }

  if (repos.isPending) {
    return <LoadingState rows={6} />
  }
  if (repos.isError) {
    return (
      <ErrorState error={repos.error} onRetry={() => void repos.refetch()} />
    )
  }

  const hasRepos = repos.data.length > 0

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Repositories"
        description="Repositories connected through the GitHub App."
        actions={
          hasRepos && canMaintain ? (
            <Button
              variant="outline"
              onClick={() => void syncRepos()}
              disabled={syncing}
            >
              {syncing ? "Syncing…" : "Sync repos"}
            </Button>
          ) : null
        }
      />

      {actionError ? (
        <ErrorState
          error={actionError}
          onRetry={() => void syncRepos()}
        />
      ) : null}

      {!hasRepos ? (
        <Empty className="border">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <FolderGit2Icon />
            </EmptyMedia>
            <EmptyTitle>No repositories yet</EmptyTitle>
            <EmptyDescription>
              Sync your GitHub App installations to import repositories, or
              install the app to connect one.
            </EmptyDescription>
          </EmptyHeader>
          {canMaintain || installUrl ? (
            <EmptyContent>
              <div className="flex flex-wrap items-center justify-center gap-2">
                {canMaintain ? (
                  <Button
                    variant="outline"
                    onClick={() => void syncRepos()}
                    disabled={syncing}
                  >
                    {syncing ? "Syncing…" : "Sync repos"}
                  </Button>
                ) : null}
                {installUrl ? (
                  <Button
                    variant="outline"
                    render={
                      <a href={installUrl} target="_blank" rel="noreferrer" />
                    }
                  >
                    Install GitHub App
                  </Button>
                ) : null}
              </div>
            </EmptyContent>
          ) : null}
        </Empty>
      ) : (
        <>
          <div className="flex items-center gap-2">
            <Input
              aria-label="Search repositories"
              placeholder="Search repositories…"
              className="max-w-xs"
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
          </div>

          {visibleRepos.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              No repositories match “{search.trim()}”.
            </p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Repository</TableHead>
                  <TableHead>Branch</TableHead>
                  <TableHead>Visibility</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Updated</TableHead>
                  <TableHead className="text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {visibleRepos.map((repo) => (
                  <TableRow key={repo.id}>
                    <TableCell className="font-medium">
                      {repo.full_name}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {repo.default_branch ?? "—"}
                    </TableCell>
                    <TableCell>
                      <Badge variant={repo.private ? "secondary" : "outline"}>
                        {repo.private ? "Private" : "Public"}
                      </Badge>
                    </TableCell>
                    <TableCell>
                      <Badge variant={repo.active ? "default" : "outline"}>
                        {repo.active ? "Active" : "Paused"}
                      </Badge>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatWhen(repo.updated_at)}
                    </TableCell>
                    <TableCell className="text-right">
                      <ButtonGroup className="justify-end">
                        {canMaintain && repo.active ? (
                          <Button
                            variant="outline"
                            size="sm"
                            aria-label={`Pause ${repo.full_name}`}
                            onClick={() => void setActive(repo, false)}
                          >
                            Pause
                          </Button>
                        ) : null}
                        {canMaintain && !repo.active ? (
                          <Button
                            variant="outline"
                            size="sm"
                            aria-label={`Enable ${repo.full_name}`}
                            onClick={() => void setActive(repo, true)}
                          >
                            Enable
                          </Button>
                        ) : null}
                        <Button
                          variant="ghost"
                          size="sm"
                          aria-label={`${canMaintain ? "Edit" : "View"} ${repo.full_name}`}
                          onClick={() => setSelectedId(repo.id)}
                        >
                          {canMaintain ? "Edit" : "View"}
                        </Button>
                      </ButtonGroup>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </>
      )}

      {selectedId !== null ? (
        <RepoConfigDialog
          repoId={selectedId}
          canEdit={canMaintain}
          onOpenChange={(open) => {
            if (!open) {
              setSelectedId(null)
            }
          }}
        />
      ) : null}
    </div>
  )
}

export default ReposRoute
