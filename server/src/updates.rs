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
    let parts: Vec<_> = version
        .strip_prefix('v')
        .unwrap_or(&version)
        .split('.')
        .collect();
    if parts.len() != 3
        || !parts
            .iter()
            .all(|s| !s.is_empty() && s.len() <= 8 && s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(bad("Invalid application version"));
    }
    let file = tokio::fs::File::open(format!("{DIRECTORY}/v{}.exe", parts.join(".")))
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
            ("cache-control", "public, max-age=86400".to_owned()),
        ],
        axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file)),
    )
        .into_response())
}
