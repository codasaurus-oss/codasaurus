const URL: &str = "https://registry.npmjs.org";
const ACCEPT: &str = "application/vnd.npm.install-v1+json";

pub async fn check_async(package: &str) -> anyhow::Result<Option<bool>> {
    match super::status_code(
        &format!("{URL}/{package}"),
        reqwest::Method::HEAD,
        Some(ACCEPT),
    )
    .await?
    {
        200 | 301 | 302 => Ok(Some(true)),
        404 | 410 => Ok(Some(false)),
        _ => Ok(None),
    }
}

pub async fn metadata_async(package: &str) -> anyhow::Result<Option<super::Metadata>> {
    let Some(data) = super::fetch_json(&format!("{URL}/{package}"), Some(ACCEPT)).await? else {
        return Ok(None);
    };
    let license = data["license"]
        .as_str()
        .or_else(|| data["license"].get("type").and_then(|v| v.as_str()))
        .map(str::to_string);
    Ok(Some(super::Metadata {
        name: package.to_string(),
        license,
    }))
}
