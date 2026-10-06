use super::*;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

struct Key {
    value: Zeroizing<String>,
    ready: Instant,
}
struct Pool {
    keys: Vec<Key>,
    preferred: usize,
}
impl Pool {
    fn parse(raw: &str) -> ApiResult<Self> {
        let mut keys = Vec::new();
        for line in raw.trim_start_matches('\u{feff}').lines() {
            let value = line.trim();
            if value.is_empty() {
                continue;
            }
            if keys.len() >= 16
                || value.len() > 1024
                || value.chars().any(char::is_whitespace)
                || reqwest::header::HeaderValue::from_str(value).is_err()
            {
                return Err(bad("Invalid server CurseForge credential configuration"));
            }
            if !keys.iter().any(|k: &Key| k.value.as_str() == value) {
                keys.push(Key {
                    value: Zeroizing::new(value.to_owned()),
                    ready: Instant::now(),
                });
            }
        }
        if keys.is_empty() {
            return Err(bad("CurseForge server API keys are not configured"));
        }
        Ok(Self { keys, preferred: 0 })
    }
    fn select(&self, tried: &[usize]) -> Option<usize> {
        (0..self.keys.len())
            .map(|n| (self.preferred + n) % self.keys.len())
            .find(|i| !tried.contains(i) && self.keys[*i].ready <= Instant::now())
    }
}
static POOL: OnceLock<Mutex<Pool>> = OnceLock::new();
fn pool() -> ApiResult<&'static Mutex<Pool>> {
    if let Some(pool) = POOL.get() {
        return Ok(pool);
    }
    let raw = credential("curseforge.keys")
        .or_else(|_| credential("curseforge.key"))
        .map_err(|_| bad("CurseForge needs server API keys. Ask the Owner to configure them."))?;
    let parsed = Pool::parse(&raw)?;
    let _ = POOL.set(Mutex::new(parsed));
    Ok(POOL.get().unwrap())
}
fn retry_delay(headers: &HeaderMap) -> Duration {
    let seconds = headers
        .get("retry-after")
        .and_then(|h| h.to_str().ok())
        .and_then(|v| {
            v.parse::<u64>().ok().or_else(|| {
                httpdate::parse_http_date(v).ok().map(|date| {
                    date.duration_since(std::time::SystemTime::now())
                        .unwrap_or_default()
                        .as_secs()
                })
            })
        })
        .unwrap_or(60)
        .clamp(1, 86400);
    Duration::from_secs(seconds)
}
async fn send(pool: &Mutex<Pool>, url: &str) -> ApiResult<reqwest::Response> {
    let mut tried = Vec::new();
    loop {
        let (index, key) = {
            let state = pool.lock().unwrap();
            let index = state.select(&tried).ok_or(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "CurseForge keys are cooling down. Please try again later.",
            ))?;
            let mut key = reqwest::header::HeaderValue::from_str(state.keys[index].value.as_str())
                .map_err(|_| bad("Invalid server CurseForge credential"))?;
            key.set_sensitive(true);
            (index, key)
        };
        tried.push(index);
        let response = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(90))
            .user_agent("Canna (https://cannamods.vip)")
            .build()
            .map_err(|_| bad("Provider connection failed"))?
            .get(url)
            .header("x-api-key", key)
            .send()
            .await
            .map_err(|_| bad("CurseForge unavailable; try again later"))?;
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            let delay = retry_delay(response.headers());
            let mut state = pool.lock().unwrap();
            state.keys[index].ready = Instant::now() + delay;
            state.preferred = (index + 1) % state.keys.len();
            continue;
        }
        // Author restrictions and invalid credentials are not quota errors.
        return Ok(response);
    }
}
pub async fn get(url: &str) -> ApiResult<reqwest::Response> {
    let parsed = reqwest::Url::parse(url).map_err(|_| bad("Invalid CurseForge address"))?;
    if parsed.scheme() != "https"
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || !matches!(
            parsed.host_str(),
            Some("api.curseforge.com" | "edge.forgecdn.net" | "mediafilez.forgecdn.net")
        )
    {
        return Err(bad("Untrusted CurseForge address"));
    }
    send(pool()?, url).await
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::get};
    #[tokio::test]
    async fn quota_failover_and_cooldown() {
        let counts = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
        let captured = counts.clone();
        let router = Router::new().route(
            "/",
            get(move |headers: HeaderMap| {
                let captured = captured.clone();
                async move {
                    let key = headers["x-api-key"].to_str().unwrap().to_owned();
                    captured.lock().unwrap().push(key.clone());
                    if key == "first" {
                        (
                            StatusCode::TOO_MANY_REQUESTS,
                            [("retry-after", "600")],
                            "limited",
                        )
                    } else {
                        (StatusCode::OK, [("retry-after", "600")], "ok")
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let state = Mutex::new(Pool::parse("first\nsecond\nfirst\n").unwrap());
        assert_eq!(send(&state, &url).await.unwrap().status(), StatusCode::OK);
        assert_eq!(send(&state, &url).await.unwrap().status(), StatusCode::OK);
        assert_eq!(*counts.lock().unwrap(), ["first", "second", "second"]);
        state.lock().unwrap().keys[1].ready = Instant::now() + Duration::from_secs(600);
        assert_eq!(
            send(&state, &url).await.err().unwrap().0,
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(counts.lock().unwrap().len(), 3);
        assert!(super::get("https://example.com").await.is_err());
        task.abort();
    }
    #[test]
    fn retry_windows_and_invalid_configuration() {
        let mut h = HeaderMap::new();
        assert_eq!(retry_delay(&h).as_secs(), 60);
        h.insert("retry-after", "120".parse().unwrap());
        assert_eq!(retry_delay(&h).as_secs(), 120);
        h.insert("retry-after", "9999999".parse().unwrap());
        assert_eq!(retry_delay(&h).as_secs(), 86400);
        assert!(Pool::parse("bad key").is_err());
        assert!(Pool::parse("").is_err());
    }
}
