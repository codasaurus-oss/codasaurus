const URL: &str = "https://pypi.org/pypi";

pub async fn check_async(package: &str) -> anyhow::Result<Option<bool>> {
    let url = format!("{}/json", super::registry_url(URL, package));
    super::check_status(&url, reqwest::Method::GET, None, &[200], &[404]).await
}

pub(super) fn extract_license(data: &serde_json::Value) -> Option<String> {
    data["info"]["license"].as_str().map(str::to_string)
}

pub async fn metadata_async(package: &str) -> anyhow::Result<Option<super::Metadata>> {
    let url = format!("{}/json", super::registry_url(URL, package));
    super::fetch_metadata(&url, None, package, extract_license).await
}
