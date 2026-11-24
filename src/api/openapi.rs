//! OpenAPI document and Swagger UI.
//!
//! Handlers are annotated with `#[utoipa::path]`; this module is the single
//! place that assembles them into the served document, so the spec and the
//! routes move together.

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Codasaurus API",
        version = "0.1.0",
        description = "Dashboard and setup API for a self-hosted Codasaurus instance.",
        license(name = "AGPL-3.0-or-later", url = "https://github.com/codasaurus-oss/codasaurus/blob/main/LICENSE")
    ),
    tags(
        (name = "setup", description = "First-run wizard"),
        (name = "auth", description = "Sessions, OIDC and invites"),
        (name = "repos", description = "Repositories"),
        (name = "reviews", description = "Reviews and findings"),
        (name = "stats", description = "Aggregate statistics"),
        (name = "settings", description = "Instance and integration settings"),
        (name = "learning", description = "Learned rules and dismissals"),
        (name = "users", description = "Users and invites"),
        (name = "github", description = "GitHub App installation links"),
        (name = "mcp", description = "MCP tool servers"),
        (name = "audit", description = "Audit log")
    ),
    paths(
        crate::api::setup::setup_status,
        crate::api::setup::get_database_status,
        crate::api::setup::setup_database,
        crate::api::setup::get_llm_config,
        crate::api::setup::setup_llm,
        crate::api::setup::github_manifest_page,
        crate::api::setup::github_manifest_url,
        crate::api::setup::setup_github,
        crate::api::setup::get_public_url,
        crate::api::setup::setup_public_url,
        crate::api::setup::github_callback_page,
        crate::api::setup::github_callback,
        crate::api::setup::setup_admin,
        crate::api::auth::login,
        crate::api::auth::me,
        crate::api::auth::logout,
        crate::api::auth::oidc_status,
        crate::api::auth::oidc_login,
        crate::api::auth::oidc_callback,
        crate::api::auth::invite_info,
        crate::api::auth::accept_invite,
        crate::api::stats::stats,
        crate::api::repos::list_repos,
        crate::api::repos::sync_repos,
        crate::api::repos::get_repo,
        crate::api::repos::update_repo,
        crate::api::repos::delete_repo,
        crate::api::reviews::list_reviews,
        crate::api::reviews::get_review,
        crate::api::reviews::get_review_findings,
        crate::api::reviews::get_review_sarif,
        crate::api::reviews::dismiss_finding,
        crate::api::settings::get_settings,
        crate::api::settings::get_github_settings,
        crate::api::settings::delete_github_settings,
        crate::api::settings::test_github_connection,
        crate::api::settings::delete_oidc_settings,
        crate::api::settings::test_oidc_connection,
        crate::api::settings::delete_ticket_settings,
        crate::api::settings::test_jira_connection,
        crate::api::settings::test_linear_connection,
        crate::api::settings::set_setting,
        crate::api::learning::list_rules,
        crate::api::learning::approve_rule,
        crate::api::learning::archive_rule,
        crate::api::learning::delete_rule,
        crate::api::users::list_users,
        crate::api::users::create_invite,
        crate::api::users::list_invites,
        crate::api::users::revoke_invite,
        crate::api::users::update_user,
        crate::api::users::remove_user,
        crate::api::users::transfer_bootstrap,
        crate::api::users::change_my_password,
        crate::api::github::install_url,
        crate::api::github::manage_url,
        crate::api::github::resolve_slug,
        crate::api::mcp::list_servers,
        crate::api::mcp::upsert_server,
        crate::api::mcp::delete_server,
        crate::api::mcp::test_server,
        crate::api::audit::list_audit
    )
)]
pub struct ApiDoc;

pub fn router() -> axum::Router<crate::api::AppState> {
    // SwaggerUi serves the document at the URL given to `.url()` as well as
    // the UI, so registering the JSON route separately would overlap.
    axum::Router::new().merge(
        utoipa_swagger_ui::SwaggerUi::new("/api/docs").url("/api/openapi.json", ApiDoc::openapi()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_documents_the_documented_surface() {
        let doc = ApiDoc::openapi();
        let value = serde_json::to_value(&doc).expect("spec serializes");
        let paths = value["paths"].as_object().expect("paths object");
        for path in [
            "/api/setup/status",
            "/api/setup/database",
            "/api/setup/llm",
            "/api/setup/github/callback",
            "/api/setup/admin",
            "/api/auth/login",
            "/api/auth/invite/{token}/accept",
            "/api/repos",
            "/api/repos/{id}",
            "/api/repos/sync",
            "/api/reviews/{id}/findings",
            "/api/reviews/dismiss",
            "/api/stats",
            "/api/settings",
            "/api/settings/{key}",
            "/api/users",
            "/api/users/invites",
            "/api/users/{id}/transfer-bootstrap",
            "/api/mcp/servers/{id}",
            "/api/github/resolve",
        ] {
            assert!(paths.contains_key(path), "missing {path}");
        }
        assert!(
            paths.len() >= 40,
            "expected >= 40 paths, got {}",
            paths.len()
        );
    }
}
