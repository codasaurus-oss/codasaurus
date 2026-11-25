import { useState } from "react"

import { ConfirmDialog } from "@/components/common/confirm-dialog"
import { StatusBadge } from "@/components/common/status-badge"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { Textarea } from "@/components/ui/textarea"
import type { Finding } from "@/lib/api/schemas"

type FindingsByFileProps = {
  findingsByFile: Record<string, Finding[]>
  canDismiss: boolean
  dismissing: boolean
  dismissed: ReadonlySet<string>
  onDismiss: (finding: Finding, message: string) => void
}

function severityTone(severity: string) {
  if (severity === "blocking") {
    return "destructive"
  }
  if (severity === "warning") {
    return "warning"
  }
  return "default"
}

function FindingRow({
  finding,
  canDismiss,
  dismissing,
  dismissed,
  onSelectDismiss,
}: {
  finding: Finding
  canDismiss: boolean
  dismissing: boolean
  dismissed: ReadonlySet<string>
  onSelectDismiss: (finding: Finding) => void
}) {
  const fingerprint = finding.fingerprint?.trim()
  const isDismissed = fingerprint !== undefined && dismissed.has(fingerprint)
  return (
    <TableRow>
      <TableCell className="align-top">
        <StatusBadge
          label={finding.severity}
          tone={severityTone(finding.severity)}
        />
      </TableCell>
      <TableCell className="align-top">
        <div className="font-medium">{finding.detector}</div>
        {finding.rule_id ? (
          <div className="font-mono text-xs text-muted-foreground">
            {finding.rule_id}
          </div>
        ) : null}
      </TableCell>
      <TableCell className="max-w-md align-top whitespace-normal">
        <p>{finding.message}</p>
        {finding.code_snippet ? (
          <ScrollArea className="mt-2 max-h-32 rounded-md bg-muted">
            <pre className="p-2 font-mono text-xs whitespace-pre-wrap">
              {finding.code_snippet}
            </pre>
          </ScrollArea>
        ) : null}
        {finding.suggested_fix ? (
          <p className="mt-2 text-xs">
            <span className="font-medium">Suggested fix: </span>
            <span className="font-mono">{finding.suggested_fix}</span>
          </p>
        ) : null}
        {finding.judge_rationale ? (
          <p className="mt-2 text-xs text-muted-foreground">
            <span className="font-medium">Rationale: </span>
            {finding.judge_rationale}
          </p>
        ) : null}
      </TableCell>
      <TableCell className="align-top tabular-nums">
        {finding.confidence ?? "—"}
      </TableCell>
      <TableCell className="align-top text-right">
        {canDismiss ? (
          <div className="flex items-center justify-end gap-2">
            {isDismissed ? <Badge variant="secondary">Dismissed</Badge> : null}
            <Button
              variant="outline"
              size="sm"
              disabled={!fingerprint || dismissing || isDismissed}
              title={
                fingerprint
                  ? isDismissed
                    ? "This finding was already dismissed in this session."
                    : undefined
                  : "This finding has no fingerprint, so it cannot be dismissed."
              }
              aria-label={`Dismiss ${finding.rule_id ?? finding.detector}`}
              onClick={() => onSelectDismiss(finding)}
            >
              Dismiss
            </Button>
          </div>
        ) : null}
      </TableCell>
    </TableRow>
  )
}

function FindingsByFile({
  findingsByFile,
  canDismiss,
  dismissing,
  dismissed,
  onDismiss,
}: FindingsByFileProps) {
  const [selected, setSelected] = useState<Finding | null>(null)
  const [message, setMessage] = useState("")

  function closeDialog() {
    setSelected(null)
    setMessage("")
  }

  return (
    <div className="flex flex-col gap-2">
      {Object.entries(findingsByFile).map(([file, fileFindings], index) => (
        <Collapsible
          key={file}
          defaultOpen={index === 0}
          className="rounded-xl ring-1 ring-foreground/10"
        >
          <CollapsibleTrigger className="flex w-full items-center justify-between gap-2 p-3 text-left">
            <span className="font-mono text-sm">{file}</span>
            <span className="text-xs text-muted-foreground">
              {fileFindings.length}{" "}
              {fileFindings.length === 1 ? "finding" : "findings"}
            </span>
          </CollapsibleTrigger>
          <CollapsibleContent>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Severity</TableHead>
                  <TableHead>Detector</TableHead>
                  <TableHead>Message</TableHead>
                  <TableHead>Confidence</TableHead>
                  <TableHead className="text-right">Action</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {fileFindings.map((finding) => (
                  <FindingRow
                    key={finding.id}
                    finding={finding}
                    canDismiss={canDismiss}
                    dismissing={dismissing}
                    dismissed={dismissed}
                    onSelectDismiss={setSelected}
                  />
                ))}
              </TableBody>
            </Table>
          </CollapsibleContent>
        </Collapsible>
      ))}

      <ConfirmDialog
        key={selected?.id ?? "none"}
        open={selected !== null}
        onOpenChange={(open) => {
          if (!open) {
            closeDialog()
          }
        }}
        title="Dismiss finding"
        description="Dismissing records this finding as a false positive in the learning store."
        onConfirm={() => {
          if (selected) {
            onDismiss(selected, message)
          }
        }}
      >
        <label className="flex flex-col gap-1 text-sm">
          Message (optional)
          <Textarea
            value={message}
            placeholder="Why is this a false positive?"
            onChange={(event) => setMessage(event.target.value)}
          />
        </label>
      </ConfirmDialog>
    </div>
  )
}

export { FindingsByFile }
