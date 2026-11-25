import { useQuery } from "@tanstack/react-query"
import { useEffect, useState } from "react"

import { EmptyState } from "@/components/common/empty-state"
import { ErrorState } from "@/components/common/error-state"
import { LoadingState } from "@/components/common/loading-state"
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
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { ApiError } from "@/lib/api/errors"
import { auditQueryOptions } from "@/lib/api/queries"
import type { AuditEntry } from "@/lib/api/schemas"
import { useAuth } from "@/lib/auth/auth-context"
import { can } from "@/lib/auth/roles"
import { formatWhen } from "@/lib/format"

const PAGE_SIZE = 50

const FORBIDDEN = new ApiError(
  403,
  "Only maintainers and owners can view the audit log."
)

// Mirrors the event types written by the backend audit logger.
const eventTypes = [
  "github.config_cleared",
  "installation.created",
  "installation.deleted",
  "mcp.server_removed",
  "mcp.server_updated",
  "mcp.tool_called",
  "oidc.config_cleared",
  "settings.updated",
  "tickets.config_cleared",
  "user.accept",
  "user.bootstrap_transfer",
  "user.invite",
  "user.invite_revoke",
  "user.login",
  "user.remove",
  "user.role_change",
]

const eventTypeItems = [
  { label: "All events", value: "" },
  ...eventTypes.map((eventType) => ({ label: eventType, value: eventType })),
]

const sensitiveMetadataKey = /(key|secret|token|password)/i

function redactMetadata(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(redactMetadata)
  }
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([key, nested]) => [
        key,
        sensitiveMetadataKey.test(key) ? "[redacted]" : redactMetadata(nested),
      ])
    )
  }
  return value
}

function metadataText(raw: string | null) {
  if (!raw) {
    return "No metadata"
  }
  try {
    return JSON.stringify(redactMetadata(JSON.parse(raw)), null, 2)
  } catch {
    return "Metadata cannot be displayed."
  }
}

function targetLabel(entry: AuditEntry) {
  if (!entry.target_type) {
    return "—"
  }
  return entry.target_id === null
    ? entry.target_type
    : `${entry.target_type}:${entry.target_id}`
}

function AuditConsole() {
  const [eventType, setEventType] = useState("")
  const [page, setPage] = useState(1)
  const [selected, setSelected] = useState<AuditEntry | null>(null)

  const audit = useQuery(
    auditQueryOptions({
      eventType: eventType || undefined,
      limit: PAGE_SIZE,
      offset: (page - 1) * PAGE_SIZE,
    })
  )

  // Retention purges shrink the result set; a page past the new last one would
  // otherwise keep requesting an offset the backend no longer has.
  useEffect(() => {
    if (audit.data && page > audit.data.total_pages) {
      setPage(Math.max(1, audit.data.total_pages))
    }
  }, [audit.data, page])

  let content
  if (audit.isPending) {
    content = <LoadingState rows={8} />
  } else if (audit.isError) {
    content = (
      <ErrorState error={audit.error} onRetry={() => void audit.refetch()} />
    )
  } else if (audit.data.entries.length === 0) {
    content = eventType ? (
      <EmptyState
        title="No matching events"
        description={`No audit entries match ${eventType}.`}
        action={
          <Button
            variant="outline"
            onClick={() => {
              setEventType("")
              setPage(1)
            }}
          >
            Clear filter
          </Button>
        }
      />
    ) : (
      <EmptyState
        title="No audit events"
        description="Audit entries appear here as users and integrations change this instance."
      />
    )
  } else {
    content = (
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Event</TableHead>
            <TableHead>Actor</TableHead>
            <TableHead>Target</TableHead>
            <TableHead>When</TableHead>
            <TableHead className="text-right">Actions</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {audit.data.entries.map((entry) => (
            <TableRow key={entry.id}>
              <TableCell className="font-mono text-xs">
                {entry.event_type}
              </TableCell>
              <TableCell className="text-muted-foreground">
                {entry.actor ?? "system"}
              </TableCell>
              <TableCell className="text-muted-foreground">
                {targetLabel(entry)}
              </TableCell>
              <TableCell className="text-muted-foreground">
                {formatWhen(entry.created_at)}
              </TableCell>
              <TableCell className="text-right">
                <Button
                  variant="outline"
                  size="sm"
                  aria-label={`View ${entry.event_type} metadata for ${
                    entry.actor ?? "system"
                  }`}
                  onClick={() => setSelected(entry)}
                >
                  Details
                </Button>
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    )
  }

  const totalPages = audit.data?.total_pages ?? 1

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Audit log"
        description="Security-relevant changes made on this instance."
      />

      <div className="flex flex-wrap items-center gap-2">
        <Select
          items={eventTypeItems}
          value={eventType}
          onValueChange={(value) => {
            setEventType(value ?? "")
            setPage(1)
          }}
        >
          <SelectTrigger aria-label="Filter by event type" className="w-64">
            <SelectValue placeholder="All events" />
          </SelectTrigger>
          <SelectContent className="w-64">
            {eventTypeItems.map((item) => (
              <SelectItem key={item.value} value={item.value}>
                {item.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {audit.data ? (
          <span className="text-sm text-muted-foreground">
            Entries older than {audit.data.retention_days} days are purged
            automatically.
          </span>
        ) : null}
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

      <Sheet
        open={selected !== null}
        onOpenChange={(open) => {
          if (!open) {
            setSelected(null)
          }
        }}
      >
        <SheetContent className="sm:max-w-lg">
          <SheetHeader>
            <SheetTitle>{selected?.event_type}</SheetTitle>
            <SheetDescription>
              Audit entry #{selected?.id} recorded{" "}
              {selected ? formatWhen(selected.created_at) : ""}.
            </SheetDescription>
          </SheetHeader>
          {selected ? (
            <div className="flex flex-col gap-3 overflow-y-auto px-4 text-sm">
              <div className="flex items-center justify-between gap-2">
                <span className="text-muted-foreground">Actor</span>
                <span className="font-medium">{selected.actor ?? "system"}</span>
              </div>
              <div className="flex items-center justify-between gap-2">
                <span className="text-muted-foreground">Target</span>
                <span className="font-medium">{targetLabel(selected)}</span>
              </div>
              <div className="flex flex-col gap-1">
                <span className="text-muted-foreground">Metadata</span>
                <pre className="max-h-96 overflow-auto rounded-lg bg-muted p-3 font-mono text-xs whitespace-pre-wrap break-words">
                  {metadataText(selected.metadata_json)}
                </pre>
              </div>
            </div>
          ) : null}
        </SheetContent>
      </Sheet>
    </div>
  )
}

function AuditRoute() {
  const { user, status } = useAuth()

  if (status === "loading") {
    return <LoadingState rows={6} />
  }
  if (!can(user?.role, "maintain")) {
    return <ErrorState error={FORBIDDEN} />
  }
  return <AuditConsole />
}

export default AuditRoute
