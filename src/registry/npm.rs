const URL: &str = "https://registry.npmjs.org";
const ACCEPT: &str = "application/vnd.npm.install-v1+json";

pub async fn check_async(package: &str) -> anyhow::Result<Option<bool>> {
    super::check_status(
        &super::registry_url(URL, package),
        reqwest::Method::HEAD,
        Some(ACCEPT),
        &[200, 301, 302],
        &[404, 410],
    )
    .await
}

pub(super) fn extract_license(data: &serde_json::Value) -> Option<String> {
    data["license"]
        .as_str()
        .or_else(|| data["license"].get("type").and_then(|value| value.as_str()))
        .map(str::to_string)
}

pub async fn metadata_async(package: &str) -> anyhow::Result<Option<super::Metadata>> {
    super::fetch_metadata(
        &super::registry_url(URL, package),
        Some(ACCEPT),
        package,
        extract_license,
    )
    .await
}
