use super::*;

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_VERSIONS: usize = 5000;
static CACHE: tokio::sync::Mutex<Option<(i64, Vec<Version>)>> = tokio::sync::Mutex::const_new(None);

#[derive(Clone, Deserialize, serde::Serialize)]
struct Version {
    id: String,
    #[serde(rename = "type")]
    kind: String,
}
#[derive(Deserialize)]
struct Manifest {
    versions: Vec<Version>,
}
fn parse(bytes: &[u8]) -> ApiResult<Vec<Version>> {
    if bytes.len() > MAX_BYTES {
        return Err(bad("Minecraft version metadata exceeds its limit"));
    }
    let manifest: Manifest =
        serde_json::from_slice(bytes).map_err(|_| bad("Invalid Minecraft version metadata"))?;
    if manifest.versions.is_empty() || manifest.versions.len() > MAX_VERSIONS {
        return Err(bad("Invalid Minecraft version list length"));
    }
    let mut ids = std::collections::HashSet::new();
    // Mojang includes historical special IDs containing spaces. The installers
    // support the bounded ASCII subset; an unsupported row must not hide every
    // ordinary release. Deduplicate accepted IDs without inventing replacements.
    let versions: Vec<_> = manifest
        .versions
        .into_iter()
        .filter(|version| {
            !version.id.is_empty()
                && version.id.len() <= 40
                && version
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                && matches!(
                    version.kind.as_str(),
                    "release" | "snapshot" | "old_alpha" | "old_beta"
                )
                && ids.insert(version.id.clone())
        })
        .collect();
    if versions.is_empty() {
        return Err(bad("No supported Minecraft versions are available"));
    }
    Ok(versions)
}
async fn fetch() -> ApiResult<Vec<Version>> {
    let unavailable = || {
        ApiError(
            StatusCode::BAD_GATEWAY,
            "Minecraft versions are temporarily unavailable",
        )
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| unavailable())?;
    let response = client
        .get(MANIFEST_URL)
        .send()
        .await
        .map_err(|_| unavailable())?
        .error_for_status()
        .map_err(|_| unavailable())?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_BYTES as u64)
    {
        return Err(unavailable());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| unavailable())?;
        if bytes.len().saturating_add(chunk.len()) > MAX_BYTES {
            return Err(unavailable());
        }
        bytes.extend_from_slice(&chunk);
    }
    parse(&bytes).map_err(|_| unavailable())
}
pub async fn list(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    let (actor, _) = app.auth(&headers)?;
    app.limits
        .check(format!("minecraft-versions:{actor}"), 60)?;
    let mut cache = CACHE.lock().await;
    if let Some((at, versions)) = &*cache
        && now() - at < 3600
    {
        return Ok(axum::Json(json!({"versions":versions,"stale":false})));
    }
    match fetch().await {
        Ok(versions) => {
            let response = json!({"versions":versions,"stale":false});
            *cache = Some((now(), versions));
            Ok(axum::Json(response))
        }
        Err(error) => {
            if let Some((at, versions)) = &*cache
                && now() - at < 86400
            {
                return Ok(axum::Json(json!({"versions":versions,"stale":true})));
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_is_bounded_unique_and_official_types_only() {
        let good = br#"{"versions":[{"id":"1.21.11","type":"release"},{"id":"26w01a","type":"snapshot"}]}"#;
        assert_eq!(parse(good).unwrap().len(), 2);
        for entries in [
            json!([]),
            json!([{"id":"../x","type":"release"}]),
            json!([{"id":"1.2","type":"other"}]),
            json!([{"id":"x".repeat(41),"type":"release"}]),
            json!(
                (0..=MAX_VERSIONS)
                    .map(|i| json!({"id":i.to_string(),"type":"release"}))
                    .collect::<Vec<_>>()
            ),
        ] {
            assert!(parse(&serde_json::to_vec(&json!({"versions":entries})).unwrap()).is_err());
        }
        assert!(parse(&vec![b' '; MAX_BYTES + 1]).is_err());
        let mixed = json!({"versions":[
            {"id":"1.21.1","type":"release"},{"id":"3D Shareware v1.34","type":"snapshot"},
            {"id":"1.14.2 Pre-Release 4","type":"snapshot"},{"id":"../bad","type":"release"},
            {"id":"1.21.1","type":"release"},{"id":"26w01a","type":"snapshot"}
        ]});
        let accepted = parse(&serde_json::to_vec(&mixed).unwrap()).unwrap();
        assert_eq!(
            accepted.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
            vec!["1.21.1", "26w01a"]
        );
    }
    #[tokio::test]
    async fn metadata_endpoint_requires_membership_before_network() {
        let (_dir, app) = crate::tests::fixture();
        assert_eq!(
            crate::tests::call(
                app,
                "GET",
                "/api/v1/providers/minecraft-versions",
                Value::Null,
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }
}
