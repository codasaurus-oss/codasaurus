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
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { apiFetch } from "@/lib/api/client"
import { ApiError } from "@/lib/api/errors"
import { learningQueryOptions, queryKeys } from "@/lib/api/queries"
import type { LearningRule } from "@/lib/api/schemas"
import { useAuth } from "@/lib/auth/auth-context"
import { can } from "@/lib/auth/roles"
import { formatWhen } from "@/lib/format"

const FORBIDDEN = new ApiError(
  403,
  "Only maintainers and owners can manage learning rules."
)

function statusVariant(status: string) {
  if (status === "approved") {
    return "default" as const
  }
  if (status === "suggested") {
    return "secondary" as const
  }
  return "outline" as const
}

function LearningConsole() {
  const queryClient = useQueryClient()
  const rules = useQuery(learningQueryOptions)
  const [deleteTarget, setDeleteTarget] = useState<LearningRule | null>(null)
  const [actionError, setActionError] = useState<unknown>(null)

  async function refresh() {
    await queryClient.invalidateQueries({ queryKey: queryKeys.learningRules })
  }

  async function decide(rule: LearningRule, action: "approve" | "archive") {
    setActionError(null)
    try {
      await apiFetch(`/api/learning/rules/${rule.id}/${action}`, {
        method: "POST",
      })
      toast.success(action === "approve" ? "Rule approved" : "Rule archived")
      await refresh()
    } catch (caught) {
      setActionError(caught)
    }
  }

  async function remove(rule: LearningRule) {
    setActionError(null)
    try {
      await apiFetch(`/api/learning/rules/${rule.id}`, { method: "DELETE" })
      toast.success("Rule deleted")
      await refresh()
    } catch (caught) {
      setActionError(caught)
    }
  }

  if (rules.isPending) {
    return <LoadingState rows={6} />
  }
  if (rules.isError) {
    return (
      <ErrorState error={rules.error} onRetry={() => void rules.refetch()} />
    )
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Learning rules"
        description="Rules learned from maintainer dismissals. The newest 200 are shown."
      />

      {actionError ? <ErrorState error={actionError} /> : null}

      {rules.data.rules.length === 0 ? (
        <EmptyState
          title="No learning rules"
          description="Rules appear here after maintainers dismiss findings and the learning pass proposes a pattern."
        />
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Detector</TableHead>
              <TableHead>File pattern</TableHead>
              <TableHead>Message pattern</TableHead>
              <TableHead>Action</TableHead>
              <TableHead>Status</TableHead>
              <TableHead>Sources</TableHead>
              <TableHead>Repository</TableHead>
              <TableHead>Created</TableHead>
              <TableHead className="text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rules.data.rules.map((rule) => (
              <TableRow key={rule.id}>
                <TableCell className="font-medium">{rule.detector}</TableCell>
                <TableCell className="text-muted-foreground">
                  {rule.file_pattern ?? "—"}
                </TableCell>
                <TableCell className="text-muted-foreground">
                  <span
                    className="block max-w-64 truncate"
                    title={rule.message_pattern ?? undefined}
                  >
                    {rule.message_pattern ?? "—"}
                  </span>
                </TableCell>
                <TableCell className="font-mono text-xs">
                  {rule.action}
                </TableCell>
                <TableCell>
                  <Badge variant={statusVariant(rule.status)}>
                    {rule.status}
                  </Badge>
                </TableCell>
                <TableCell className="text-muted-foreground">
                  {rule.source_count}
                </TableCell>
                <TableCell className="text-muted-foreground">
                  {rule.repo_full_name ?? "Global"}
                </TableCell>
                <TableCell className="text-muted-foreground">
                  {formatWhen(rule.created_at)}
                </TableCell>
                <TableCell className="text-right">
                  <div className="flex justify-end gap-1">
                    {rule.status === "suggested" ? (
                      <Button
                        variant="outline"
                        size="sm"
                        aria-label={`Approve rule ${rule.id}`}
                        onClick={() => void decide(rule, "approve")}
                      >
                        Approve
                      </Button>
                    ) : null}
                    {rule.status !== "archived" ? (
                      <Button
                        variant="ghost"
                        size="sm"
                        aria-label={`Archive rule ${rule.id}`}
                        onClick={() => void decide(rule, "archive")}
                      >
                        Archive
                      </Button>
                    ) : null}
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label={`Delete rule ${rule.id}`}
                      onClick={() => setDeleteTarget(rule)}
                    >
                      Delete
                    </Button>
                  </div>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}

      <ConfirmDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) {
            setDeleteTarget(null)
          }
        }}
        title="Delete rule"
        description={
          deleteTarget
            ? `Delete the ${deleteTarget.detector} rule for ${
                deleteTarget.repo_full_name ?? "all repositories"
              }? This cannot be undone.`
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

function LearningRoute() {
  const { user, status } = useAuth()

  if (status === "loading") {
    return <LoadingState rows={6} />
  }
  if (!can(user?.role, "maintain")) {
    return <ErrorState error={FORBIDDEN} />
  }
  return <LearningConsole />
}

export default LearningRoute
