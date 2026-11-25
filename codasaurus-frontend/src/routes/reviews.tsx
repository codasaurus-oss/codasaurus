import { useQuery } from "@tanstack/react-query"
import { useState } from "react"
import { Link } from "react-router"

import { EmptyState } from "@/components/common/empty-state"
import { ErrorState } from "@/components/common/error-state"
import { LoadingState } from "@/components/common/loading-state"
import {
  reviewStatusTone,
  StatusBadge,
} from "@/components/common/status-badge"
import { PageHeader } from "@/components/layout/page-header"
import { Button } from "@/components/ui/button"
import {
  Pagination,
  PaginationContent,
  PaginationItem,
  PaginationNext,
  PaginationPrevious,
} from "@/components/ui/pagination"
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
import { reposQueryOptions, reviewsQueryOptions } from "@/lib/api/queries"
import { formatWhen } from "@/lib/format"

const PAGE_SIZE = 20
const statuses = ["pending", "running", "passed", "failed", "error"]

const statusItems = [
  { label: "All statuses", value: "" },
  ...statuses.map((status) => ({ label: status, value: status })),
]

function formatTimestamp(value: string | null) {
  return value ? formatWhen(value) : "—"
}

function ReviewsRoute() {
  const [repoId, setRepoId] = useState<number | undefined>()
  const [status, setStatus] = useState("")
  const [page, setPage] = useState(1)

  const repos = useQuery(reposQueryOptions)
  const reviews = useQuery(
    reviewsQueryOptions({
      repoId,
      status: status || undefined,
      limit: PAGE_SIZE,
      offset: (page - 1) * PAGE_SIZE,
    })
  )

  const repoItems = [
    { label: "All repositories", value: "" },
    ...(repos.data ?? []).map((repo) => ({
      label: repo.full_name,
      value: String(repo.id),
    })),
  ]
  const hasFilters = repoId !== undefined || status !== ""

  function clearFilters() {
    setRepoId(undefined)
    setStatus("")
    setPage(1)
  }

  let content
  if (reviews.isPending) {
    content = <LoadingState rows={6} />
  } else if (reviews.isError) {
    content = (
      <ErrorState
        error={reviews.error}
        onRetry={() => void reviews.refetch()}
      />
    )
  } else if (reviews.data.reviews.length === 0) {
    content = hasFilters ? (
      <EmptyState
        title="No matching reviews"
        description="No reviews match the selected repository and status filters."
        action={
          <Button variant="outline" onClick={clearFilters}>
            Clear filters
          </Button>
        }
      />
    ) : (
      <EmptyState
        title="No reviews"
        description="Reviews appear here once a pull request is opened on a connected repository."
      />
    )
  } else {
    content = (
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>PR</TableHead>
            <TableHead>Repository</TableHead>
            <TableHead>Author</TableHead>
            <TableHead>Status</TableHead>
            <TableHead>Started</TableHead>
            <TableHead>Completed</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {reviews.data.reviews.map((review) => (
            <TableRow key={review.id}>
              <TableCell>
                <Link
                  className="font-medium underline-offset-4 hover:underline"
                  to={`/app/reviews/${review.id}`}
                >
                  #{review.pr_number}
                </Link>
              </TableCell>
              <TableCell className="text-muted-foreground">
                {review.repo_full_name || review.repo_name || "—"}
              </TableCell>
              <TableCell className="text-muted-foreground">
                {review.pr_author ?? "—"}
              </TableCell>
              <TableCell>
                <StatusBadge
                  label={review.status}
                  tone={reviewStatusTone(review.status)}
                />
              </TableCell>
              <TableCell className="text-muted-foreground">
                {formatTimestamp(review.started_at)}
              </TableCell>
              <TableCell className="text-muted-foreground">
                {formatTimestamp(review.completed_at)}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    )
  }

  const totalPages = reviews.data?.total_pages ?? 1

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Reviews"
        description="Pull request reviews and their findings."
      />

      <div className="flex flex-wrap items-center gap-2">
        <Select
          items={repoItems}
          value={repoId === undefined ? "" : String(repoId)}
          onValueChange={(value) => {
            setRepoId(value ? Number(value) : undefined)
            setPage(1)
          }}
        >
          <SelectTrigger aria-label="Filter by repository" className="w-56">
            <SelectValue placeholder="All repositories" />
          </SelectTrigger>
          <SelectContent className="w-56">
            {repoItems.map((item) => (
              <SelectItem key={item.value} value={item.value}>
                {item.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        <Select
          items={statusItems}
          value={status}
          onValueChange={(value) => {
            setStatus(value ?? "")
            setPage(1)
          }}
        >
          <SelectTrigger aria-label="Filter by status" className="w-48">
            <SelectValue placeholder="All statuses" />
          </SelectTrigger>
          <SelectContent className="w-48">
            {statusItems.map((item) => (
              <SelectItem key={item.value} value={item.value}>
                {item.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {content}

      {totalPages > 1 ? (
        <Pagination>
          <PaginationContent>
            <PaginationItem>
              <PaginationPrevious
                href="#"
                aria-disabled={page <= 1}
                onClick={(event) => {
                  event.preventDefault()
                  if (page > 1) {
                    setPage(page - 1)
                  }
                }}
              />
            </PaginationItem>
            <PaginationItem>
              <span className="px-2 text-sm text-muted-foreground">
                Page {page} of {totalPages}
              </span>
            </PaginationItem>
            <PaginationItem>
              <PaginationNext
                href="#"
                aria-disabled={page >= totalPages}
                onClick={(event) => {
                  event.preventDefault()
                  if (page < totalPages) {
                    setPage(page + 1)
                  }
                }}
              />
            </PaginationItem>
          </PaginationContent>
        </Pagination>
      ) : null}
    </div>
  )
}

export default ReviewsRoute
