use anyhow::Result;

pub async fn metadata_async(_package: &str) -> Result<Option<super::Metadata>> {
    Ok(None)
}

/// Check whether a Go module path exists using the public proxy.
///
/// proxy.golang.org returns 200 + JSON for known modules, 404/410 for unknown,
/// and 5xx for transient issues. We treat anything except 200/404/410 as a
/// soft failure so the detector does not flag packages during a proxy outage.
pub async fn check_async(package: &str) -> Result<Option<bool>> {
    let url = module_url(package);
    super::check_status(&url, reqwest::Method::GET, None, &[200], &[404, 410]).await
}

fn module_url(package: &str) -> String {
    format!(
        "https://proxy.golang.org/{}/@latest",
        crate::util::url_encode(package)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_url_encoding() {
        assert_eq!(
            module_url("github.com/gorilla/mux"),
            "https://proxy.golang.org/github.com%2Fgorilla%2Fmux/@latest"
        );
        assert_eq!(
            module_url("example.com/pkg"),
            "https://proxy.golang.org/example.com%2Fpkg/@latest"
        );
    }
}
