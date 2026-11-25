import { z } from "zod"

const timestamp = z.string()

const roleSchema = z.enum(["owner", "maintainer", "viewer"])

// Old rows can still carry the pre-rename "admin" role.
const normalizedRoleSchema = z
  .string()
  .transform((role) => (role === "admin" ? "owner" : role))
  .pipe(roleSchema)

const userInfoSchema = z
  .object({
    email: z.string(),
    role: normalizedRoleSchema,
    auth_provider: z.string().nullish(),
    is_bootstrap: z.boolean().default(false),
  })
  .passthrough()

const meResponseSchema = z
  .object({
    authenticated: z.boolean(),
    user: userInfoSchema.nullable(),
  })
  .passthrough()

const loginResponseSchema = z
  .object({
    user: userInfoSchema,
  })
  .passthrough()

const logoutResponseSchema = z
  .object({
    status: z.string(),
  })
  .passthrough()

const setupStatusSchema = z
  .object({
    database: z.boolean(),
    llm: z.boolean(),
    github: z.boolean(),
    admin: z.boolean(),
    complete: z.boolean(),
    github_install_url: z.string().nullish(),
    public_url: z.string(),
  })
  .passthrough()

const setupResultSchema = z
  .object({
    status: z.string(),
    message: z.string().nullish(),
    test_passed: z.boolean().nullish(),
  })
  .passthrough()

// Public liveness payload from GET /health; `network` is the egress map.
const healthResponseSchema = z
  .object({
    status: z.string(),
    db: z.boolean(),
    data_dir: z.boolean(),
    ready: z.boolean(),
    version: z.string(),
    egress_profile: z.string(),
    offline_mode: z.boolean(),
    network: z
      .object({
        llm: z.boolean(),
        registries: z.boolean(),
        osv: z.boolean(),
      })
      .passthrough(),
    note: z.string(),
  })
  .passthrough()

const databaseStatusSchema = z
  .object({
    connected: z.boolean(),
    provider: z.string(),
    host: z.string(),
    database: z.string(),
    server_version: z.string(),
    hint: z.string(),
  })
  .passthrough()

const gatewayCatalogModelSchema = z
  .object({ name: z.string(), mode: z.string() })
  .passthrough()

const modelCatalogSchema = z.object({
  models: z.array(gatewayCatalogModelSchema),
})

const llmConfigSchema = z
  .object({
    provider: z.string().nullable(),
    api_key: z.string().nullable(),
    model: z.string().nullable(),
    model_cheap: z.string().nullish(),
    embedding_model: z.string().nullish(),
    upstream_provider: z.string().nullish(),
    upstream_model: z.string().nullish(),
    upstream_model_cheap: z.string().nullish(),
    upstream_model_embedding: z.string().nullish(),
    models: z
      .array(z.object({ id: z.string(), name: z.string() }).passthrough())
      .default([]),
    providers: z
      .array(z.object({ id: z.string(), name: z.string() }).passthrough())
      .default([]),
    catalog: z.array(gatewayCatalogModelSchema).default([]),
  })
  .passthrough()

const publicUrlSchema = z
  .object({
    url: z.string(),
    source: z.string(),
    localhost: z.boolean(),
  })
  .passthrough()

const syncResultSchema = z
  .object({
    status: z.string(),
    synced: z.number(),
  })
  .passthrough()

const inviteInfoSchema = z
  .object({
    email: z.string().nullable(),
    role: normalizedRoleSchema,
    expires_at: timestamp,
    email_locked: z.boolean(),
  })
  .passthrough()

const oidcStatusSchema = z
  .object({
    enabled: z.boolean(),
  })
  .passthrough()

const repoSchema = z
  .object({
    id: z.number(),
    github_id: z.number().nullable(),
    full_name: z.string(),
    owner: z.string(),
    name: z.string(),
    default_branch: z.string().nullable(),
    installation_id: z.number(),
    private: z.boolean(),
    active: z.boolean(),
    config_json: z.string().nullable(),
    created_at: timestamp,
    updated_at: timestamp,
  })
  .passthrough()

const repoListSchema = z.array(repoSchema)

// A review's summary_json is a JSON document stored as a string, so a null,
// a non-string, or a half-written value all have to degrade to null.
const summaryJsonSchema = z.preprocess((value) => {
  if (typeof value !== "string") {
    return null
  }
  try {
    return JSON.parse(value)
  } catch {
    return null
  }
}, z.unknown())

const reviewSchema = z
  .object({
    id: z.number(),
    repo_id: z.number(),
    pr_number: z.number(),
    pr_title: z.string().nullable(),
    pr_author: z.string().nullable(),
    pr_base_branch: z.string().nullable(),
    pr_head_branch: z.string().nullable(),
    pr_head_sha: z.string().nullable(),
    status: z.string(),
    summary_json: summaryJsonSchema,
    started_at: timestamp.nullable(),
    completed_at: timestamp.nullable(),
    created_at: timestamp,
    repo_name: z.string().nullish(),
    repo_full_name: z.string().nullish(),
    finding_count: z.number().nullish(),
    file_count: z.number().nullish(),
    by_severity: z.record(z.string(), z.number()).nullish(),
    by_detector: z.record(z.string(), z.number()).nullish(),
    duration_secs: z.number().nullish(),
  })
  .passthrough()

const findingSchema = z
  .object({
    id: z.number(),
    review_id: z.number(),
    fingerprint: z.string().nullable(),
    file_path: z.string(),
    line_start: z.number().nullable(),
    line_end: z.number().nullable(),
    column_start: z.number().nullable(),
    column_end: z.number().nullable(),
    severity: z.string(),
    detector: z.string(),
    rule_id: z.string().nullable(),
    message: z.string(),
    suggested_fix: z.string().nullable(),
    code_snippet: z.string().nullable(),
    context: z.string().nullable(),
    category: z.string().nullable(),
    confidence: z.number().nullable(),
    judge_rationale: z.string().nullable(),
    created_at: timestamp,
  })
  .passthrough()

const reviewsResponseSchema = z
  .object({
    reviews: z.array(reviewSchema),
    total: z.number(),
    total_pages: z.number(),
    limit: z.number(),
    offset: z.number(),
  })
  .passthrough()

const reviewSummarySchema = z
  .object({
    finding_count: z.number(),
    file_count: z.number(),
    by_severity: z.record(z.string(), z.number()),
    by_detector: z.record(z.string(), z.number()),
    duration_secs: z.number().nullable(),
  })
  .passthrough()

const reviewDetailSchema = z
  .object({
    review: reviewSchema,
    findings: z.array(findingSchema),
    summary: reviewSummarySchema,
  })
  .passthrough()

const reviewFindingsSchema = z
  .object({
    review_id: z.number(),
    findings_by_file: z.record(z.string(), z.array(findingSchema)),
    total_findings: z.number(),
  })
  .passthrough()

const userSchema = z
  .object({
    id: z.number(),
    email: z.string(),
    role: normalizedRoleSchema,
    auth_provider: z.string(),
    is_bootstrap: z.boolean(),
    created_at: timestamp,
  })
  .passthrough()

const usersResponseSchema = z
  .object({
    users: z.array(userSchema),
  })
  .passthrough()

const inviteSchema = z
  .object({
    id: z.number(),
    email: z.string().nullable(),
    role: normalizedRoleSchema,
    created_by: z.string().nullish(),
    expires_at: timestamp,
    created_at: timestamp,
  })
  .passthrough()

const invitesResponseSchema = z
  .object({
    invites: z.array(inviteSchema),
  })
  .passthrough()

const createdInviteSchema = z
  .object({
    id: z.number(),
    email: z.string().nullable(),
    role: normalizedRoleSchema,
    expires_at: timestamp,
    url: z.string(),
    token: z.string(),
  })
  .passthrough()

const auditEntrySchema = z
  .object({
    id: z.number(),
    event_type: z.string(),
    actor: z.string().nullable(),
    target_type: z.string().nullable(),
    target_id: z.number().nullable(),
    metadata_json: z.string().nullable(),
    created_at: timestamp,
  })
  .passthrough()

const auditResponseSchema = z
  .object({
    entries: z.array(auditEntrySchema),
    total: z.number(),
    total_pages: z.number(),
    limit: z.number(),
    offset: z.number(),
    retention_days: z.number(),
  })
  .passthrough()

const learningRuleSchema = z
  .object({
    id: z.string(),
    detector: z.string(),
    file_pattern: z.string().nullable(),
    message_pattern: z.string().nullable(),
    action: z.string(),
    reason: z.string(),
    status: z.string(),
    source_count: z.number(),
    repo_full_name: z.string().nullable(),
    created_at: timestamp,
  })
  .passthrough()

const learningRulesResponseSchema = z
  .object({
    rules: z.array(learningRuleSchema),
  })
  .passthrough()

const mcpServerSchema = z
  .object({
    id: z.string(),
    name: z.string(),
    base_url: z.string(),
    docs_url: z.string().nullable(),
    enabled: z.boolean(),
    key_configured: z.boolean(),
    is_custom: z.boolean(),
  })
  .passthrough()

const mcpServersResponseSchema = z
  .object({
    servers: z.array(mcpServerSchema),
  })
  .passthrough()

const mcpTestResultSchema = z
  .object({
    status: z.string(),
    ok: z.boolean(),
    tool_count: z.number(),
  })
  .passthrough()

// GET /api/settings is a flat key → value map plus three synthetic offline-mode
// fields. Every stored value is a string and secrets arrive already masked.
const settingsMapSchema = z
  .object({
    offline_mode_effective: z.boolean().optional(),
    offline_mode_source: z.string().optional(),
    offline_mode_hint: z.string().optional(),
  })
  .catchall(z.string())

const settingSaveSchema = z
  .object({
    status: z.string(),
    key: z.string(),
    restart_required: z.boolean().optional(),
    skipped: z.string().optional(),
    cleared: z.boolean().optional(),
  })
  .passthrough()

const settingsActionResultSchema = z
  .object({
    status: z.string(),
    message: z.string().nullish(),
  })
  .passthrough()

const integrationTestSchema = z
  .object({
    status: z.string(),
    ok: z.boolean().nullish(),
    message: z.string().nullish(),
  })
  .passthrough()

const githubSettingsSchema = z
  .object({
    app_id: z.string().nullish(),
    app_name: z.string().nullish(),
    slug: z.string().nullish(),
    configured: z.boolean(),
  })
  .passthrough()

const githubUrlSchema = z
  .object({
    url: z.string().nullable(),
    error: z.string().optional(),
  })
  .passthrough()

const recentActivitySchema = z
  .object({
    id: z.number(),
    repo: z.string(),
    pr_number: z.number(),
    pr_title: z.string().nullable(),
    status: z.string(),
    created_at: timestamp,
  })
  .passthrough()

const reviewDaySchema = z
  .object({
    day: z.string(),
    reviews: z.number(),
    findings: z.number(),
  })
  .passthrough()

const detectorShareSchema = z
  .object({
    detector: z.string(),
    concern: z.string(),
    count: z.number(),
    share_pct: z.number(),
  })
  .passthrough()

const concernShareSchema = z
  .object({
    concern: z.string(),
    count: z.number(),
    share_pct: z.number(),
  })
  .passthrough()

const statsAnalyticsSchema = z
  .object({
    reviews_by_day: z.array(reviewDaySchema).optional(),
    findings_by_detector: z.array(detectorShareSchema).optional(),
    findings_by_concern: z.array(concernShareSchema).optional(),
    findings_last_7_days: z.number().optional(),
    findings_prev_7_days: z.number().optional(),
    reviews_prev_7_days: z.number().optional(),
    dismissals_prev_7_days: z.number().optional(),
    dismiss_rate_last_7_days: z.number().nullish(),
    pass_rate_7d: z.number().nullish(),
    pass_rate_prev_7d: z.number().nullish(),
    outcomes_7d: z
      .object({
        passed: z.number(),
        failed: z.number(),
        other: z.number(),
      })
      .passthrough()
      .optional(),
    weekly_digest: z
      .object({
        reviews: z.number(),
        findings: z.number(),
        dismissals: z.number(),
        note: z.string(),
      })
      .passthrough()
      .optional(),
  })
  .passthrough()

const statsSchema = z
  .object({
    total_repos: z.number(),
    total_reviews_today: z.number(),
    pass_rate: z.number().nullable(),
    total_findings: z.number(),
    reviews_last_7_days: z.number(),
    dismissals_last_7_days: z.number(),
    recent_activity: z.array(recentActivitySchema).optional(),
    trust: z
      .object({
        dismissals: z.number(),
        tier1_findings: z.number(),
        fp_proxy_ratio: z.number(),
        accept_rate: z.number().nullable(),
        note: z.string(),
      })
      .passthrough()
      .optional(),
    llm: z
      .object({
        requests: z.number(),
        prompt_chars: z.number(),
        spend_usd_estimate: z.number(),
        spend_usd_last_day: z.number(),
        daily_budget_usd: z.number(),
        note: z.string(),
      })
      .passthrough()
      .optional(),
    analytics: statsAnalyticsSchema.optional(),
  })
  .passthrough()

type Role = z.infer<typeof roleSchema>
type AuditEntry = z.infer<typeof auditEntrySchema>
type LearningRule = z.infer<typeof learningRuleSchema>
type McpServer = z.infer<typeof mcpServerSchema>
type InviteInfo = z.infer<typeof inviteInfoSchema>
type UserInfo = z.infer<typeof userInfoSchema>
type MeResponse = z.infer<typeof meResponseSchema>
type SetupStatus = z.infer<typeof setupStatusSchema>
type SetupResult = z.infer<typeof setupResultSchema>
type DatabaseStatus = z.infer<typeof databaseStatusSchema>
type LlmConfig = z.infer<typeof llmConfigSchema>
type PublicUrl = z.infer<typeof publicUrlSchema>
type SyncResult = z.infer<typeof syncResultSchema>
type Repo = z.infer<typeof repoSchema>
type Review = z.infer<typeof reviewSchema>
type ReviewsResponse = z.infer<typeof reviewsResponseSchema>
type ReviewDetail = z.infer<typeof reviewDetailSchema>
type Finding = z.infer<typeof findingSchema>
type User = z.infer<typeof userSchema>
type UsersResponse = z.infer<typeof usersResponseSchema>
type Invite = z.infer<typeof inviteSchema>
type CreatedInvite = z.infer<typeof createdInviteSchema>
type SettingsMap = z.infer<typeof settingsMapSchema>

export {
  auditEntrySchema,
  auditResponseSchema,
  createdInviteSchema,
  databaseStatusSchema,
  findingSchema,
  githubSettingsSchema,
  githubUrlSchema,
  healthResponseSchema,
  integrationTestSchema,
  inviteInfoSchema,
  inviteSchema,
  invitesResponseSchema,
  learningRuleSchema,
  learningRulesResponseSchema,
  llmConfigSchema,
  modelCatalogSchema,
  loginResponseSchema,
  logoutResponseSchema,
  mcpServerSchema,
  mcpServersResponseSchema,
  mcpTestResultSchema,
  meResponseSchema,
  oidcStatusSchema,
  publicUrlSchema,
  repoListSchema,
  repoSchema,
  reviewDetailSchema,
  reviewFindingsSchema,
  reviewsResponseSchema,
  reviewSchema,
  roleSchema,
  settingSaveSchema,
  settingsActionResultSchema,
  settingsMapSchema,
  setupResultSchema,
  setupStatusSchema,
  statsSchema,
  summaryJsonSchema,
  syncResultSchema,
  userInfoSchema,
  userSchema,
  usersResponseSchema,
}
export type {
  AuditEntry,
  CreatedInvite,
  DatabaseStatus,
  Finding,
  Invite,
  InviteInfo,
  LearningRule,
  LlmConfig,
  McpServer,
  MeResponse,
  PublicUrl,
  Repo,
  Review,
  ReviewDetail,
  ReviewsResponse,
  Role,
  SettingsMap,
  SetupResult,
  SetupStatus,
  SyncResult,
  User,
  UserInfo,
  UsersResponse,
}
