import { keepPreviousData, queryOptions } from "@tanstack/react-query"

import { apiFetch } from "@/lib/api/client"
import {
  auditResponseSchema,
  githubSettingsSchema,
  healthResponseSchema,
  inviteInfoSchema,
  invitesResponseSchema,
  learningRulesResponseSchema,
  llmConfigSchema,
  mcpServersResponseSchema,
  oidcStatusSchema,
  repoListSchema,
  repoSchema,
  reviewDetailSchema,
  reviewFindingsSchema,
  reviewsResponseSchema,
  settingsMapSchema,
  setupStatusSchema,
  statsSchema,
  usersResponseSchema,
} from "@/lib/api/schemas"

type ReviewFilters = {
  repoId?: number
  status?: string
  limit?: number
  offset?: number
}

const queryKeys = {
  health: ["health"] as const,
  oidcStatus: ["auth", "oidc-status"] as const,
  invite: (token: string) => ["invites", "detail", token] as const,
  setupStatus: ["setup", "status"] as const,
  stats: ["stats"] as const,
  repos: ["repos"] as const,
  repo: (id: number) => ["repos", "detail", id] as const,
  reviews: ["reviews"] as const,
  review: (id: number) => ["reviews", "detail", id] as const,
  reviewFindings: (id: number) => ["reviews", "findings", id] as const,
  reviewList: (filters: ReviewFilters) => ["reviews", "list", filters] as const,
  users: ["users"] as const,
  invites: ["invites"] as const,
  audit: ["audit"] as const,
  auditList: (filters: { eventType?: string; limit?: number; offset?: number }) =>
    ["audit", "list", filters] as const,
  learningRules: ["learning", "rules"] as const,
  mcpServers: ["mcp", "servers"] as const,
  settings: ["settings"] as const,
  llmConfig: ["setup", "llm"] as const,
  githubSettings: ["settings", "github"] as const,
}

const oidcStatusQueryOptions = queryOptions({
  queryKey: queryKeys.oidcStatus,
  queryFn: () => apiFetch("/api/auth/oidc/status", undefined, oidcStatusSchema),
  staleTime: 300_000,
})

const inviteQueryOptions = (token: string) =>
  queryOptions({
    queryKey: queryKeys.invite(token),
    queryFn: () =>
      apiFetch(`/api/auth/invite/${token}`, undefined, inviteInfoSchema),
    enabled: Boolean(token),
    retry: false,
  })

const llmConfigQueryOptions = queryOptions({
  queryKey: queryKeys.llmConfig,
  queryFn: () => apiFetch("/api/setup/llm", undefined, llmConfigSchema),
  staleTime: 30_000,
})

const setupStatusQueryOptions = queryOptions({
  queryKey: queryKeys.setupStatus,
  queryFn: () => apiFetch("/api/setup/status", undefined, setupStatusSchema),
  staleTime: 30_000,
})

const statsQueryOptions = queryOptions({
  queryKey: queryKeys.stats,
  queryFn: () => apiFetch("/api/stats", undefined, statsSchema),
  staleTime: 30_000,
  refetchInterval: 30_000,
})

const reposQueryOptions = queryOptions({
  queryKey: queryKeys.repos,
  queryFn: () => apiFetch("/api/repos", undefined, repoListSchema),
  staleTime: 30_000,
})

const repoQueryOptions = (id: number) =>
  queryOptions({
    queryKey: queryKeys.repo(id),
    queryFn: () => apiFetch(`/api/repos/${id}`, undefined, repoSchema),
  })

const reviewsQueryOptions = (filters: ReviewFilters) =>
  queryOptions({
    queryKey: queryKeys.reviewList(filters),
    queryFn: () => {
      const params = new URLSearchParams()
      params.set("limit", String(filters.limit ?? 20))
      params.set("offset", String(filters.offset ?? 0))
      if (filters.repoId !== undefined) {
        params.set("repo_id", String(filters.repoId))
      }
      if (filters.status) {
        params.set("status", filters.status)
      }
      return apiFetch(
        `/api/reviews?${params.toString()}`,
        undefined,
        reviewsResponseSchema
      )
    },
    placeholderData: keepPreviousData,
  })

const reviewQueryOptions = (id: number) =>
  queryOptions({
    queryKey: queryKeys.review(id),
    queryFn: () => apiFetch(`/api/reviews/${id}`, undefined, reviewDetailSchema),
    refetchInterval: (query) => {
      const status = query.state.data?.review.status
      return status === "pending" || status === "running" ? 10_000 : false
    },
  })

const reviewFindingsQueryOptions = (id: number) =>
  queryOptions({
    queryKey: queryKeys.reviewFindings(id),
    queryFn: () =>
      apiFetch(`/api/reviews/${id}/findings`, undefined, reviewFindingsSchema),
  })

const healthQueryOptions = queryOptions({
  queryKey: queryKeys.health,
  queryFn: () => apiFetch("/health", undefined, healthResponseSchema),
  staleTime: 30_000,
  refetchInterval: 30_000,
})

const usersQueryOptions = queryOptions({
  queryKey: queryKeys.users,
  queryFn: () => apiFetch("/api/users", undefined, usersResponseSchema),
  staleTime: 30_000,
})

const invitesQueryOptions = queryOptions({
  queryKey: queryKeys.invites,
  queryFn: () => apiFetch("/api/users/invites", undefined, invitesResponseSchema),
  staleTime: 30_000,
})

const auditQueryOptions = (filters: {
  eventType?: string
  limit?: number
  offset?: number
}) =>
  queryOptions({
    queryKey: queryKeys.auditList(filters),
    queryFn: () => {
      const params = new URLSearchParams()
      params.set("limit", String(filters.limit ?? 50))
      params.set("offset", String(filters.offset ?? 0))
      if (filters.eventType) {
        params.set("event_type", filters.eventType)
      }
      return apiFetch(
        `/api/audit?${params.toString()}`,
        undefined,
        auditResponseSchema
      )
    },
    placeholderData: keepPreviousData,
  })

const learningQueryOptions = queryOptions({
  queryKey: queryKeys.learningRules,
  queryFn: () =>
    apiFetch("/api/learning/rules", undefined, learningRulesResponseSchema),
  staleTime: 30_000,
})

const mcpServersQueryOptions = queryOptions({
  queryKey: queryKeys.mcpServers,
  queryFn: () =>
    apiFetch("/api/mcp/servers", undefined, mcpServersResponseSchema),
  staleTime: 30_000,
})

const settingsQueryOptions = queryOptions({
  queryKey: queryKeys.settings,
  queryFn: () => apiFetch("/api/settings", undefined, settingsMapSchema),
  staleTime: 30_000,
})

const githubSettingsQueryOptions = queryOptions({
  queryKey: queryKeys.githubSettings,
  queryFn: () =>
    apiFetch("/api/settings/github", undefined, githubSettingsSchema),
  staleTime: 30_000,
})

export {
  auditQueryOptions,
  githubSettingsQueryOptions,
  healthQueryOptions,
  inviteQueryOptions,
  invitesQueryOptions,
  learningQueryOptions,
  llmConfigQueryOptions,
  mcpServersQueryOptions,
  oidcStatusQueryOptions,
  queryKeys,
  repoQueryOptions,
  reposQueryOptions,
  reviewFindingsQueryOptions,
  reviewQueryOptions,
  reviewsQueryOptions,
  settingsQueryOptions,
  setupStatusQueryOptions,
  statsQueryOptions,
  usersQueryOptions,
}
export type { ReviewFilters }
