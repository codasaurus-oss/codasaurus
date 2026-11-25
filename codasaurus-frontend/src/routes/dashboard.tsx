import { useQuery, useQueryClient } from "@tanstack/react-query"
import { GitPullRequestIcon } from "lucide-react"
import { useState, type ReactNode } from "react"
import { Link } from "react-router"
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  XAxis,
  YAxis,
} from "recharts"
import { toast } from "sonner"
import { z } from "zod"

import { ErrorState } from "@/components/common/error-state"
import { LoadingState } from "@/components/common/loading-state"
import { StatusBadge } from "@/components/common/status-badge"
import { PageHeader } from "@/components/layout/page-header"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
  type ChartConfig,
} from "@/components/ui/chart"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Skeleton } from "@/components/ui/skeleton"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { apiFetch } from "@/lib/api/client"
import {
  healthQueryOptions,
  queryKeys,
  setupStatusQueryOptions,
  statsQueryOptions,
} from "@/lib/api/queries"
import { statsSchema, syncResultSchema } from "@/lib/api/schemas"
import { useAuth } from "@/lib/auth/auth-context"
import { can } from "@/lib/auth/roles"
import { formatWhen } from "@/lib/format"

type Stats = z.infer<typeof statsSchema>

const activityConfig = {
  reviews: { label: "Reviews", color: "var(--chart-1)" },
  findings: { label: "Findings", color: "var(--chart-2)" },
} satisfies ChartConfig

const detectorConfig = {
  count: { label: "Findings", color: "var(--chart-1)" },
} satisfies ChartConfig

const outcomeConfig = {
  passed: { label: "Passed", color: "var(--chart-1)" },
  failed: { label: "Failed", color: "var(--chart-2)" },
  other: { label: "Other", color: "var(--chart-3)" },
} satisfies ChartConfig

function formatCount(value: number | null | undefined) {
  return typeof value === "number" ? value.toLocaleString() : "—"
}

function formatPercent(value: number | null | undefined) {
  return typeof value === "number" ? `${value.toFixed(1)}%` : "—"
}

function formatUsd(value: number | null | undefined) {
  return typeof value === "number" ? `$${value.toFixed(2)}` : "—"
}

function formatRatio(value: number | null | undefined) {
  return typeof value === "number" ? value.toFixed(2) : "—"
}

function ChartEmpty() {
  return (
    <p className="py-12 text-center text-sm text-muted-foreground">
      No data yet.
    </p>
  )
}

function Kpi({
  label,
  value,
  hint,
  accent,
}: {
  label: string
  value: string
  hint?: string
  accent?: string
}) {
  return (
    <Card size="sm">
      <CardHeader>
        <CardDescription>{label}</CardDescription>
        <CardTitle className={`text-2xl tabular-nums ${accent ?? ""}`}>
          {value}
        </CardTitle>
        {hint ? <p className="text-xs text-muted-foreground">{hint}</p> : null}
      </CardHeader>
    </Card>
  )
}

function StatRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-2">
      <span className="text-muted-foreground">{label}</span>
      <span className="font-medium tabular-nums">{children}</span>
    </div>
  )
}

function statusTone(status: string) {
  if (status === "passed") {
    return "success"
  }
  if (status === "failed" || status === "error") {
    return "destructive"
  }
  return "default"
}

function RecentActivity({
  rows,
  actions,
}: {
  rows: NonNullable<Stats["recent_activity"]>
  actions: ReactNode
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Recent activity</CardTitle>
        <CardDescription>
          Latest reviews across connected repositories.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {rows.length === 0 ? (
          <Empty className="border">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <GitPullRequestIcon />
              </EmptyMedia>
              <EmptyTitle>No reviews yet</EmptyTitle>
              <EmptyDescription>
                Sync your repositories or install the GitHub App to start
                reviewing pull requests.
              </EmptyDescription>
            </EmptyHeader>
            {actions ? (
              <EmptyContent>
                <div className="flex flex-wrap items-center justify-center gap-2">
                  {actions}
                </div>
              </EmptyContent>
            ) : null}
          </Empty>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Repository</TableHead>
                <TableHead>Pull request</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>When</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((row) => (
                <TableRow key={row.id}>
                  <TableCell className="font-medium">
                    {row.repo || "Unknown repository"}
                  </TableCell>
                  <TableCell>
                    <Link
                      className="hover:underline hover:underline-offset-4"
                      to={`/app/reviews/${row.id}`}
                    >
                      #{row.pr_number}
                      {row.pr_title ? ` ${row.pr_title}` : ""}
                    </Link>
                  </TableCell>
                  <TableCell>
                    <StatusBadge
                      label={row.status}
                      tone={statusTone(row.status)}
                    />
                  </TableCell>
                  <TableCell className="text-muted-foreground">
                    {formatWhen(row.created_at)}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </CardContent>
    </Card>
  )
}

function DashboardRoute() {
  const { user } = useAuth()
  const queryClient = useQueryClient()
  const stats = useQuery(statsQueryOptions)
  const setup = useQuery(setupStatusQueryOptions)
  const health = useQuery(healthQueryOptions)
  const [syncing, setSyncing] = useState(false)

  async function syncRepos() {
    setSyncing(true)
    try {
      const result = await apiFetch(
        "/api/repos/sync",
        { method: "POST" },
        syncResultSchema
      )
      toast.success(
        `Synced ${result.synced} ${
          result.synced === 1 ? "repository" : "repositories"
        }`
      )
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.stats }),
        queryClient.invalidateQueries({ queryKey: queryKeys.repos }),
      ])
    } catch (caught) {
      toast.error(
        caught instanceof Error ? caught.message : "Could not sync repositories."
      )
    } finally {
      setSyncing(false)
    }
  }

  if (stats.isPending) {
    return <LoadingState rows={6} />
  }
  if (stats.isError) {
    return (
      <ErrorState
        error={stats.error}
        onRetry={() => void stats.refetch()}
      />
    )
  }

  const data = stats.data
  const analytics = data.analytics
  const days = analytics?.reviews_by_day ?? []
  const detectors = analytics?.findings_by_detector ?? []
  const outcomes = analytics?.outcomes_7d
  const outcomeData = outcomes
    ? [
        { name: "passed", value: outcomes.passed },
        { name: "failed", value: outcomes.failed },
        { name: "other", value: outcomes.other },
      ]
    : []
  const outcomeTotal = outcomeData.reduce((sum, row) => sum + row.value, 0)
  const hasDayData = days.some((day) => day.reviews > 0 || day.findings > 0)
  const installUrl = setup.data?.github_install_url
  const canSync = can(user?.role, "maintain")

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Dashboard"
        description="Review coverage, detector signal and engine health."
      />

      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <Kpi
          label="Active repositories"
          value={formatCount(data.total_repos)}
          accent="text-[var(--chart-1)]"
        />
        <Kpi
          label="Reviews today"
          value={formatCount(data.total_reviews_today)}
          accent="text-[var(--chart-2)]"
        />
        <Kpi
          label="Pass rate"
          value={formatPercent(data.pass_rate)}
          hint="last 30 days"
          accent="text-[var(--chart-3)]"
        />
        <Kpi
          label="Findings"
          value={formatCount(data.total_findings)}
          hint="last 30 days"
          accent="text-[var(--chart-5)]"
        />
      </div>

      <div className="grid gap-4 lg:grid-cols-3">
        <Card className="lg:col-span-2">
          <CardHeader>
            <CardTitle>Reviews and findings</CardTitle>
            <CardDescription>Daily totals for the last 14 days.</CardDescription>
          </CardHeader>
          <CardContent>
            {hasDayData ? (
              <ChartContainer
                config={activityConfig}
                className="aspect-auto h-[220px] w-full"
              >
                <AreaChart data={days}>
                  <CartesianGrid vertical={false} />
                  <XAxis
                    dataKey="day"
                    tickLine={false}
                    axisLine={false}
                    tickFormatter={(value: string) => value.slice(5)}
                  />
                  <YAxis tickLine={false} axisLine={false} allowDecimals={false} />
                  <ChartTooltip content={<ChartTooltipContent />} />
                  <Area
                    dataKey="reviews"
                    type="monotone"
                    fill="var(--color-reviews)"
                    fillOpacity={0.2}
                    stroke="var(--color-reviews)"
                  />
                  <Area
                    dataKey="findings"
                    type="monotone"
                    fill="var(--color-findings)"
                    fillOpacity={0.2}
                    stroke="var(--color-findings)"
                  />
                </AreaChart>
              </ChartContainer>
            ) : (
              <ChartEmpty />
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Outcomes</CardTitle>
            <CardDescription>Review results in the last 7 days.</CardDescription>
          </CardHeader>
          <CardContent>
            {outcomeTotal === 0 ? (
              <ChartEmpty />
            ) : (
              <ChartContainer
                config={outcomeConfig}
                className="aspect-auto h-[220px] w-full"
              >
                <BarChart data={outcomeData} layout="vertical">
                  <XAxis type="number" hide />
                  <YAxis
                    type="category"
                    dataKey="name"
                    tickLine={false}
                    axisLine={false}
                    width={60}
                  />
                  <ChartTooltip
                    cursor={false}
                    content={<ChartTooltipContent nameKey="name" hideLabel />}
                  />
                  <Bar dataKey="value" radius={4}>
                    {outcomeData.map((row) => (
                      <Cell
                        key={row.name}
                        fill={`var(--color-${row.name})`}
                      />
                    ))}
                  </Bar>
                </BarChart>
              </ChartContainer>
            )}
          </CardContent>
        </Card>
      </div>

      <div className="grid gap-4 lg:grid-cols-3">
        <Card className="lg:col-span-2">
          <CardHeader>
            <CardTitle>Findings by detector</CardTitle>
            <CardDescription>
              Detector hits in the last 30 days.
            </CardDescription>
          </CardHeader>
          <CardContent>
            {detectors.length === 0 ? (
              <ChartEmpty />
            ) : (
              <ChartContainer
                config={detectorConfig}
                className="aspect-auto h-[260px] w-full"
              >
                <BarChart data={detectors} layout="vertical">
                  <XAxis type="number" hide />
                  <YAxis
                    type="category"
                    dataKey="detector"
                    tickLine={false}
                    axisLine={false}
                    width={140}
                    tick={{ fontSize: 11 }}
                  />
                  <ChartTooltip
                    cursor={false}
                    content={<ChartTooltipContent />}
                  />
                  <Bar dataKey="count" radius={4}>
                    {detectors.map((row, index) => (
                      <Cell
                        key={row.detector}
                        fill={`var(--chart-${(index % 5) + 1})`}
                      />
                    ))}
                  </Bar>
                </BarChart>
              </ChartContainer>
            )}
          </CardContent>
        </Card>

        <div className="flex flex-col gap-4">
          <Card size="sm">
            <CardHeader>
              <CardTitle>Engine</CardTitle>
              <CardDescription>Which review tiers are available.</CardDescription>
            </CardHeader>
            <CardContent className="flex flex-col gap-2">
              {health.isError ? (
                <ErrorState
                  error={health.error}
                  onRetry={() => void health.refetch()}
                />
              ) : (
                <StatRow label="Mode">
                  {health.data ? (
                    <StatusBadge
                      label={
                        health.data.offline_mode
                          ? "Offline · Tier-1 only"
                          : "Online"
                      }
                      tone={health.data.offline_mode ? "warning" : "default"}
                    />
                  ) : (
                    <Skeleton className="h-4 w-20" />
                  )}
                </StatRow>
              )}
              <StatRow label="LLM setup">
                {setup.data ? (
                  <StatusBadge
                    label={setup.data.llm ? "Setup complete" : "Not configured"}
                  />
                ) : (
                  "—"
                )}
              </StatRow>
              <StatRow label="LLM requests">
                {formatCount(data.llm?.requests)}
              </StatRow>
              <StatRow label="Daily budget">
                {formatUsd(data.llm?.daily_budget_usd)}
              </StatRow>
              <StatRow label="Spend estimate">
                {formatUsd(data.llm?.spend_usd_estimate)}
              </StatRow>
            </CardContent>
          </Card>

          <Card size="sm">
            <CardHeader>
              <CardTitle>Signal quality</CardTitle>
              {data.trust?.note ? (
                <CardDescription>{data.trust.note}</CardDescription>
              ) : null}
            </CardHeader>
            <CardContent className="flex flex-col gap-2">
              <StatRow label="Accept rate">
                {formatPercent(data.trust?.accept_rate)}
              </StatRow>
              <StatRow label="False-positive proxy">
                {formatRatio(data.trust?.fp_proxy_ratio)}
              </StatRow>
              <StatRow label="Dismissals">
                {formatCount(data.trust?.dismissals)}
              </StatRow>
              <StatRow label="Tier-1 findings">
                {formatCount(data.trust?.tier1_findings)}
              </StatRow>
            </CardContent>
          </Card>
        </div>
      </div>

      <RecentActivity
        rows={data.recent_activity ?? []}
        actions={
          canSync || installUrl ? (
            <>
              {canSync ? (
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
            </>
          ) : null
        }
      />
    </div>
  )
}

export default DashboardRoute
