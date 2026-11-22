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
        crate::api::setup::get_public_url,
        crate::api::auth::login,
        crate::api::auth::me,
        crate::api::auth::logout,
        crate::api::stats::stats,
        crate::api::repos::list_repos,
        crate::api::repos::get_repo,
        crate::api::reviews::list_reviews,
        crate::api::reviews::get_review,
        crate::api::reviews::get_review_findings,
        crate::api::reviews::get_review_sarif,
        crate::api::settings::get_settings,
        crate::api::learning::list_rules,
        crate::api::users::list_users,
        crate::api::github::install_url,
        crate::api::github::manage_url,
        crate::api::mcp::list_servers,
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
            "/api/auth/login",
            "/api/repos",
            "/api/repos/{id}",
            "/api/reviews/{id}/findings",
            "/api/stats",
            "/api/settings",
            "/api/users",
        ] {
            assert!(paths.contains_key(path), "missing {path}");
        }
        assert!(
            paths.len() >= 19,
            "expected >= 19 paths, got {}",
            paths.len()
        );
    }
}
