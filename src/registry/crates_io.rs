const URL: &str = "https://crates.io/api/v1/crates";

pub async fn check_async(package: &str) -> anyhow::Result<Option<bool>> {
    match super::status_code(&format!("{URL}/{package}"), reqwest::Method::GET, None).await? {
        200 => Ok(Some(true)),
        404 => Ok(Some(false)),
        _ => Ok(None),
    }
}

pub async fn metadata_async(package: &str) -> anyhow::Result<Option<super::Metadata>> {
    let Some(data) = super::fetch_json(&format!("{URL}/{package}"), None).await? else {
        return Ok(None);
    };
    let license = data["crate"]["license"]
        .as_str()
        .map(str::to_string)
        .filter(|s| !s.is_empty());
    Ok(Some(super::Metadata {
        name: package.to_string(),
        license,
    }))
}
