const URL: &str = "https://crates.io/api/v1/crates";

pub async fn check_async(package: &str) -> anyhow::Result<Option<bool>> {
    super::check_status(
        &super::registry_url(URL, package),
        reqwest::Method::GET,
        None,
        &[200],
        &[404],
    )
    .await
}

pub(super) fn extract_license(data: &serde_json::Value) -> Option<String> {
    data["crate"]["license"]
        .as_str()
        .map(str::to_string)
        .filter(|license| !license.is_empty())
}

pub async fn metadata_async(package: &str) -> anyhow::Result<Option<super::Metadata>> {
    super::fetch_metadata(
        &super::registry_url(URL, package),
        None,
        package,
        extract_license,
    )
    .await
}
