use super::*;
const DIRECTORY: &str = "/opt/canna/releases";
pub async fn latest() -> ApiResult<axum::Json<Value>> {
    let bytes = tokio::fs::read(format!("{DIRECTORY}/latest.json"))
        .await
        .map_err(|_| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "Application update unavailable",
            )
        })?;
    if bytes.len() > 1024 * 1024 {
        return Err(bad("Invalid application release"));
    }
    Ok(axum::Json(
        serde_json::from_slice(&bytes).map_err(|_| bad("Invalid application release"))?,
    ))
}
pub async fn binary(Path(version): Path<String>) -> ApiResult<Response> {
    let normalized = release_version(&version)?;
    serve_release(&normalized, false).await
}
fn release_version(version: &str) -> ApiResult<String> {
    let parts: Vec<_> = version
        .strip_prefix('v')
        .unwrap_or(version)
        .split('.')
        .collect();
    if parts.len() != 3
        || !parts
            .iter()
            .all(|s| !s.is_empty() && s.len() <= 8 && s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(bad("Invalid application version"));
    }
    Ok(parts.join("."))
}
async fn serve_release(version: &str, installer: bool) -> ApiResult<Response> {
    let suffix = if installer { "-setup" } else { "" };
    serve_artifact(
        version,
        suffix,
        if installer {
            "Canna-Setup.exe"
        } else {
            "Canna-Mod-Manager.exe"
        },
    )
    .await
}
async fn serve_artifact(version: &str, suffix: &str, name: &str) -> ApiResult<Response> {
    let file = tokio::fs::File::open(format!("{DIRECTORY}/v{version}{suffix}.exe"))
        .await
        .map_err(|_| ApiError(StatusCode::NOT_FOUND, "Application release unavailable"))?;
    let length = file
        .metadata()
        .await
        .map_err(|_| bad("Application release unavailable"))?
        .len();
    if length == 0 || length > 150 * 1024 * 1024 {
        return Err(bad("Invalid application release"));
    }
    Ok((
        [
            ("content-type", "application/octet-stream".to_owned()),
            ("content-length", length.to_string()),
            (
                "content-disposition",
                format!("attachment; filename=\"{name}\""),
            ),
            (
                "cache-control",
                "public, max-age=0, must-revalidate".to_owned(),
            ),
        ],
        axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file)),
    )
        .into_response())
}

// Stable links always resolve to the current public release, without member cookies.
pub async fn portable() -> ApiResult<Response> {
    let value = latest().await?.0;
    let version = value["tag_name"]
        .as_str()
        .ok_or(bad("Invalid application release"))?;
    serve_release(&release_version(version)?, false).await
}
pub async fn installer() -> ApiResult<Response> {
    let bytes = tokio::fs::read(format!("{DIRECTORY}/installer-latest.json"))
        .await
        .map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "Installer unavailable"))?;
    if bytes.len() > 8192 {
        return Err(bad("Invalid installer release"));
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| bad("Invalid installer release"))?;
    let version = value["version"]
        .as_str()
        .ok_or(bad("Invalid installer release"))?;
    serve_release(&release_version(version)?, true).await
}
pub async fn maintenance_latest() -> ApiResult<axum::Json<Value>> {
    let bytes = tokio::fs::read(format!("{DIRECTORY}/maintenance-latest.json"))
        .await
        .map_err(|_| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "Maintenance update unavailable",
            )
        })?;
    if bytes.len() > 1024 * 1024 {
        return Err(bad("Invalid maintenance release"));
    }
    Ok(axum::Json(
        serde_json::from_slice(&bytes).map_err(|_| bad("Invalid maintenance release"))?,
    ))
}
pub async fn maintenance_binary(Path(version): Path<String>) -> ApiResult<Response> {
    serve_artifact(
        &release_version(&version)?,
        "-maintenance",
        "Canna-Updater.exe",
    )
    .await
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_download_paths_cannot_escape_release_directory() {
        for value in [
            "../0.2.16",
            "v../../etc/passwd",
            "1.2.3.exe",
            "1.2/3.4",
            "1.2.-3",
            "",
        ] {
            assert!(release_version(value).is_err());
        }
        assert_eq!(release_version("v0.2.16").unwrap(), "0.2.16");
    }
}
