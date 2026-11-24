const URL: &str = "https://pypi.org/pypi";

pub async fn check_async(package: &str) -> anyhow::Result<Option<bool>> {
    match super::status_code(&format!("{URL}/{package}/json"), reqwest::Method::GET, None).await? {
        200 => Ok(Some(true)),
        404 => Ok(Some(false)),
        _ => Ok(None),
    }
}

pub async fn metadata_async(package: &str) -> anyhow::Result<Option<super::Metadata>> {
    let Some(data) = super::fetch_json(&format!("{URL}/{package}/json"), None).await? else {
        return Ok(None);
    };
    let license = data["info"]["license"].as_str().map(str::to_string);
    Ok(Some(super::Metadata {
        name: package.to_string(),
        license,
    }))
}
