import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useState, type ReactNode } from "react"
import { Link, useParams } from "react-router"
import { toast } from "sonner"

import { EmptyState } from "@/components/common/empty-state"
import { ErrorState } from "@/components/common/error-state"
import { LoadingState } from "@/components/common/loading-state"
import {
  reviewStatusTone,
  StatusBadge,
} from "@/components/common/status-badge"
import { NotFound } from "@/components/layout/not-found"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { Skeleton } from "@/components/ui/skeleton"
import { FindingsByFile } from "@/features/reviews/findings-by-file"
import { apiFetch } from "@/lib/api/client"
import {
  queryKeys,
  reviewFindingsQueryOptions,
  reviewQueryOptions,
} from "@/lib/api/queries"
import type { Finding, ReviewDetail } from "@/lib/api/schemas"
import { useAuth } from "@/lib/auth/auth-context"
import { can } from "@/lib/auth/roles"

function parseReviewId(raw: string | undefined) {
  if (!raw) {
    return null
  }
  const id = Number(raw)
  return Number.isSafeInteger(id) && id > 0 ? id : null
}

function formatDuration(seconds: number | null) {
  return seconds === null ? "—" : `${seconds}s`
}

function SummaryCard({
  label,
  value,
  children,
}: {
  label: string
  value: string
  children?: ReactNode
}) {
  return (
    <Card size="sm">
      <CardHeader>
        <CardDescription>{label}</CardDescription>
        <CardTitle className="text-2xl tabular-nums">{value}</CardTitle>
        {children}
      </CardHeader>
    </Card>
  )
}

function Breakdown({ counts }: { counts: Record<string, number> }) {
  const entries = Object.entries(counts)
  if (entries.length === 0) {
    return <p className="text-xs text-muted-foreground">None</p>
  }
  return (
    <div className="flex flex-col gap-1">
      {entries.map(([label, count]) => (
        <div
          key={label}
          className="flex items-center justify-between gap-2 text-xs"
        >
          <span className="text-muted-foreground">{label}</span>
          <span className="tabular-nums">{count}</span>
        </div>
      ))}
    </div>
  )
}

function FindingsPanel({
  review,
  findingsByFile,
  canDismiss,
}: {
  review: ReviewDetail["review"]
  findingsByFile: Record<string, Finding[]>
  canDismiss: boolean
}) {
  const queryClient = useQueryClient()
  const [dismissing, setDismissing] = useState(false)
  const [dismissed, setDismissed] = useState<Set<string>>(() => new Set())

  async function dismissFinding(finding: Finding, message: string) {
    const fingerprint = finding.fingerprint?.trim()
    if (!fingerprint) {
      return
    }
    setDismissing(true)
    try {
      await apiFetch("/api/reviews/dismiss", {
        method: "POST",
        body: JSON.stringify({
          fingerprint,
          detector: finding.detector,
          file: finding.file_path,
          pr_number: review.pr_number,
          repo: review.repo_full_name || undefined,
          ...(message.trim() ? { message: message.trim() } : {}),
        }),
      })
      toast.success("Finding dismissed")
      setDismissed((current) => new Set(current).add(fingerprint))
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.review(review.id) }),
        queryClient.invalidateQueries({
          queryKey: queryKeys.reviewFindings(review.id),
        }),
        queryClient.invalidateQueries({ queryKey: queryKeys.reviews }),
        queryClient.invalidateQueries({ queryKey: queryKeys.stats }),
      ])
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Could not dismiss the finding."
      )
    } finally {
      setDismissing(false)
    }
  }

  return (
    <FindingsByFile
      findingsByFile={findingsByFile}
      canDismiss={canDismiss}
      dismissing={dismissing}
      dismissed={dismissed}
      onDismiss={(finding, message) => {
        void dismissFinding(finding, message)
      }}
    />
  )
}

function ReviewDetailRoute() {
  const { id: rawId } = useParams()
  const id = parseReviewId(rawId)
  const { user } = useAuth()
  const reviewQuery = useQuery({
    ...reviewQueryOptions(id ?? 0),
    enabled: id !== null,
  })
  const reviewStatus = reviewQuery.data?.review.status
  const active = reviewStatus === "pending" || reviewStatus === "running"
  const findingsQuery = useQuery({
    ...reviewFindingsQueryOptions(id ?? 0),
    enabled: id !== null && reviewStatus !== undefined && !active,
  })

  if (id === null) {
    return <NotFound />
  }
  if (reviewQuery.isPending) {
    return <LoadingState rows={8} />
  }
  if (reviewQuery.isError) {
    return (
      <ErrorState
        error={reviewQuery.error}
        onRetry={() => void reviewQuery.refetch()}
      />
    )
  }

  const { review, summary } = reviewQuery.data
  const repoName = review.repo_full_name || review.repo_name || "—"
  const canDismiss = can(user?.role, "maintain")

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-2">
        <Link
          to="/app/reviews"
          className="text-sm text-muted-foreground hover:underline"
        >
          ← Reviews
        </Link>
        <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
          <div className="flex flex-col gap-1">
            <h1 className="font-heading text-xl font-medium tracking-tight">
              #{review.pr_number} {review.pr_title ?? "Untitled pull request"}
            </h1>
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-muted-foreground">
              <span>{repoName}</span>
              <span>by {review.pr_author ?? "unknown"}</span>
              {review.pr_head_sha ? (
                <code className="font-mono text-xs">{review.pr_head_sha}</code>
              ) : null}
              <StatusBadge
                label={review.status}
                tone={reviewStatusTone(review.status)}
              />
            </div>
          </div>
          <Button
            variant="outline"
            render={
              <a
                href={`/api/reviews/${review.id}/sarif`}
                download={`review-${review.id}.sarif`}
              />
            }
          >
            Download SARIF
          </Button>
        </div>
      </div>

      <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
        <SummaryCard
          label="Findings"
          value={String(summary.finding_count)}
        >
          <Breakdown counts={summary.by_severity} />
        </SummaryCard>
        <SummaryCard label="Files" value={String(summary.file_count)} />
        <SummaryCard label="Detectors" value={String(Object.keys(summary.by_detector).length)}>
          <Breakdown counts={summary.by_detector} />
        </SummaryCard>
        <SummaryCard
          label="Duration"
          value={formatDuration(summary.duration_secs)}
        />
      </div>

      {active ? (
        <div className="flex items-center gap-2">
          <StatusBadge
            label={review.status === "pending" ? "Queued" : "Running"}
            tone="warning"
          />
          <span className="text-sm text-muted-foreground">
            Review in progress. This page refreshes every 10 seconds.
          </span>
        </div>
      ) : null}

      <section className="flex flex-col gap-2">
        <h2 className="font-heading text-base font-medium">Findings</h2>
        {active ? (
          <Skeleton className="h-32 w-full" />
        ) : findingsQuery.isPending ? (
          <LoadingState rows={3} />
        ) : findingsQuery.isError ? (
          <ErrorState
            error={findingsQuery.error}
            onRetry={() => void findingsQuery.refetch()}
          />
        ) : findingsQuery.data.total_findings === 0 ? (
          <EmptyState
            title="No findings"
            description="This review completed without any findings."
          />
        ) : Object.keys(findingsQuery.data.findings_by_file).length === 0 ? (
          <p className="text-sm text-muted-foreground">
            This review reports findings, but none are available to display.
          </p>
        ) : (
          <FindingsPanel
            key={review.id}
            review={review}
            findingsByFile={findingsQuery.data.findings_by_file}
            canDismiss={canDismiss}
          />
        )}
      </section>
    </div>
  )
}

export default ReviewDetailRoute
