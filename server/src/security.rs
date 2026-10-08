use super::*;
use axum::extract::ConnectInfo;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
};

#[derive(Default)]
pub struct Limits {
    buckets: Mutex<HashMap<String, (i64, u32)>>,
}
impl Limits {
    pub fn check(&self, key: String, limit: u32) -> ApiResult<()> {
        let mut buckets = self.buckets.lock().unwrap();
        let time = now();
        buckets.retain(|_, v| time.saturating_sub(v.0) < 60);
        if !buckets.contains_key(&key) && buckets.len() >= 10000 {
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "Service busy; try again shortly",
            ));
        }
        let bucket = buckets.entry(key).or_insert((time, 0));
        if bucket.1 >= limit {
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many requests; try again in one minute",
            ));
        }
        bucket.1 += 1;
        Ok(())
    }
}
pub(crate) fn client(request: &Request) -> String {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|v| v.0.ip());
    if peer.is_some_and(|p| p.is_loopback())
        && let Some(ip) = request
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .and_then(|v| v.trim().parse::<IpAddr>().ok())
    {
        return ip.to_string();
    }
    peer.map(|p| p.to_string())
        .unwrap_or_else(|| "local-fixture".into())
}
pub async fn protect(State(app): State<Shared>, request: Request, next: Next) -> Response {
    if !matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    ) {
        let path = request.uri().path();
        let authentication = matches!(
            path,
            "/api/v1/login"
                | "/api/v1/login/verify"
                | "/api/v1/register"
                | "/api/v1/verify-email"
                | "/api/v1/resend-verification"
                | "/api/v1/forgot-password"
                | "/api/v1/forgot-username"
                | "/api/v1/reset-password"
                | "/api/v1/desktop/approve"
        );
        let scope = if authentication { "auth" } else { "write" };
        let identity = if !authentication {
            app.auth(request.headers())
                .ok()
                .map(|v| format!("member:{}", v.0))
        } else {
            None
        }
        .unwrap_or_else(|| format!("client:{}", client(&request)));
        let result = app
            .limits
            .check(
                format!("{scope}:{identity}"),
                if authentication { 30 } else { 60 },
            )
            .and_then(|_| app.limits.check(format!("global:{scope}"), 1200));
        if let Err(error) = result {
            return error.into_response();
        }
    }
    let support = request.uri().path().starts_with("/api/v1/support/");
    let session = auth_token(request.headers()).map(str::to_owned);
    let authenticated = session.is_some() && app.auth(request.headers()).is_ok();
    let mut response = next.run(request).await;
    if support {
        response
            .headers_mut()
            .insert("cache-control", "no-store".parse().unwrap());
        response
            .headers_mut()
            .insert("x-robots-tag", "noindex".parse().unwrap());
    }
    if authenticated
        && let Some(raw) = session
        && !response.headers().contains_key("set-cookie")
    {
        response
            .headers_mut()
            .insert("cache-control", "no-store".parse().unwrap());
        response
            .headers_mut()
            .append("vary", "Cookie, Authorization".parse().unwrap());
        response.headers_mut().append(
            "set-cookie",
            format!(
                "canna_session={raw}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={}",
                devices::COOKIE_AGE
            )
            .parse()
            .unwrap(),
        );
    }
    response
}
pub fn quota(db: &Connection, user: i64, incoming: i64) -> ApiResult<()> {
    let (used, count): (i64, i64) = db.query_row(
        "SELECT COALESCE(SUM(size),0),COUNT(*) FROM mods WHERE user_id=?1",
        [user],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let role: String = db.query_row("SELECT role FROM users WHERE id=?1", [user], |r| r.get(0))?;
    let limit = match role.as_str() {
        "owner" => STORAGE_LIMIT,
        "admin" => 10 * 1024 * 1024 * 1024,
        "vip" => 5 * 1024 * 1024 * 1024,
        _ => 2 * 1024 * 1024 * 1024,
    };
    if used.saturating_add(incoming) > limit || count >= 500 {
        return Err(bad("Your mod storage quota is full; delete older uploads"));
    }
    Ok(())
}
pub fn content_quota(db: &Connection, user: i64) -> ApiResult<()> {
    let count: i64 = db.query_row("SELECT (SELECT COUNT(*) FROM posts WHERE user_id=?1)+(SELECT COUNT(*) FROM profile_comments WHERE author=?1)", [user], |r| r.get(0))?;
    if count >= 5000 {
        return Err(bad("Your posting quota is full; contact an administrator"));
    }
    Ok(())
}
pub fn approved(db: &Connection, id: &str) -> ApiResult<()> {
    let mut pending = vec![id.to_owned()];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if seen.len() > 128 {
            return Err(bad("Dependency graph exceeds 128 projects"));
        }
        let review:Option<bool>=db.query_row("SELECT NOT EXISTS(SELECT 1 FROM mod_reviews WHERE mod_id=m.id AND approved=0) FROM mods m WHERE id=?1",[&id],|r|r.get(0)).optional()?;
        if review != Some(true) {
            return Err(ApiError(
                StatusCode::FORBIDDEN,
                "This mod or an imported dependency is awaiting review or unavailable",
            ));
        }
        scans::require_review(db, &id)?;
        let data = external::details(db, &id)?;
        for dep in data["dependency_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            pending.push(dep.to_owned());
        }
    }
    Ok(())
}
pub async fn approve(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<Value>> {
    let (actor, admin) = app.auth(&headers)?;
    if !admin {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Administrator permission required",
        ));
    }
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let mut pending = vec![id.clone()];
    let mut seen = std::collections::BTreeSet::new();
    let mut approved = 0;
    while let Some(mod_id) = pending.pop() {
        if !seen.insert(mod_id.clone()) {
            continue;
        }
        if seen.len() > 128 {
            return Err(bad("Dependency review exceeds 128 projects"));
        }
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM mods WHERE id=?1)",
            [&mod_id],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(bad("Imported dependency is missing from the library"));
        }
        scans::require_review(&tx, &mod_id)?;
        let data = external::details(&tx, &mod_id)?;
        for dep in data["dependency_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            pending.push(dep.to_owned());
        }
        approved += tx.execute(
            "UPDATE mod_reviews SET approved=1 WHERE mod_id=?1 AND approved=0",
            [&mod_id],
        )?;
        notifications::accepted(&tx, &mod_id)?;
        tx.execute(
            "INSERT INTO audit(actor,action,target,created) VALUES(?1,'approve-mod',?2,?3)",
            params![actor, mod_id, now()],
        )?;
    }
    if approved == 0 {
        return Err(bad("No pending mod review found"));
    }
    tx.commit()?;
    Ok(axum::Json(json!({"ok":true})))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn client_limits_are_independent_and_bounded() {
        let limits = Limits::default();
        for _ in 0..30 {
            limits.check("auth:a".into(), 30).unwrap();
        }
        assert!(limits.check("auth:a".into(), 30).is_err());
        assert!(limits.check("auth:b".into(), 30).is_ok());
        limits.buckets.lock().unwrap().get_mut("auth:a").unwrap().0 = now() - 60;
        assert!(limits.check("auth:a".into(), 30).is_ok());
    }
    #[tokio::test]
    async fn exhausting_one_proxy_client_does_not_block_another() {
        use tower::ServiceExt;
        let (_dir, app) = crate::tests::fixture();
        for n in 0..31 {
            let mut request = Request::builder()
                .method("POST")
                .uri("/api/v1/login")
                .header("content-type", "application/json")
                .header("x-forwarded-for", "198.51.100.10")
                .body(Body::from(r#"{"username":"bad","password":"short"}"#))
                .unwrap();
            request
                .extensions_mut()
                .insert(ConnectInfo("127.0.0.1:2345".parse::<SocketAddr>().unwrap()));
            let status = router(app.clone()).oneshot(request).await.unwrap().status();
            assert_eq!(
                status,
                if n == 30 {
                    StatusCode::TOO_MANY_REQUESTS
                } else {
                    StatusCode::UNAUTHORIZED
                }
            );
        }
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/v1/login")
            .header("content-type", "application/json")
            .header("x-forwarded-for", "198.51.100.11")
            .body(Body::from(r#"{"username":"bad","password":"short"}"#))
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo("127.0.0.1:2345".parse::<SocketAddr>().unwrap()));
        assert_eq!(
            router(app).oneshot(request).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    #[test]
    fn storage_and_posting_quotas_are_enforced_by_account() {
        let (_dir, app) = crate::tests::fixture();
        crate::tests::account(&app, "quota", false);
        crate::tests::account(&app, "otherquota", false);
        let db = app.db.lock().unwrap();
        db.execute(
            "INSERT INTO mods VALUES('fixture',1,1686940,'Quota','1','','hash',?1)",
            [2_i64 * 1024 * 1024 * 1024],
        )
        .unwrap();
        assert!(quota(&db, 1, 1).is_err());
        assert!(quota(&db, 2, 128).is_ok());
        db.execute("UPDATE users SET role='vip' WHERE id=1", [])
            .unwrap();
        assert!(quota(&db, 1, 128).is_ok());
        db.execute("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<5000) INSERT INTO profile_comments SELECT CAST(x AS TEXT),2,1,'quota fixture',0 FROM n",[]).unwrap();
        assert!(content_quota(&db, 1).is_err());
        assert!(content_quota(&db, 2).is_ok());
    }
    #[test]
    fn only_loopback_proxy_can_supply_client_address() {
        let mut req = Request::builder()
            .header("x-forwarded-for", "198.51.100.7")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(
            "203.0.113.9:1234".parse::<SocketAddr>().unwrap(),
        ));
        assert_eq!(client(&req), "203.0.113.9");
        req.extensions_mut()
            .insert(ConnectInfo("127.0.0.1:1234".parse::<SocketAddr>().unwrap()));
        assert_eq!(client(&req), "198.51.100.7");
    }
}
