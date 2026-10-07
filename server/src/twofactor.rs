use super::*;
const TRUST: &str = "__Host-canna_trust";
const BINDING: &str = "__Host-canna_login";
fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| {
            let (key, value) = part.trim().split_once('=')?;
            (key == name && value.len() == 64).then_some(value)
        })
}
fn set_cookie(name: &str, value: &str, seconds: i64) -> axum::http::HeaderValue {
    format!("{name}={value}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={seconds}")
        .parse()
        .unwrap()
}
pub async fn start(app: Shared, id: i64, headers: &HeaderMap) -> ApiResult<Response> {
    if let Some(raw) = cookie(headers, TRUST) {
        let valid:bool=app.db.lock().unwrap().query_row("SELECT EXISTS(SELECT 1 FROM trusted_devices WHERE hash=?1 AND user_id=?2 AND expires>?3)",params![digest(raw),id,now()],|r|r.get(0))?;
        if valid {
            return Ok(session_response(app.session(id, headers)?));
        }
    }
    let address: String = app.db.lock().unwrap().query_row(
        "SELECT email FROM users WHERE id=?1 AND verified=1 AND banned=0",
        [id],
        |r| r.get(0),
    )?;
    let (challenge, code) = email::challenge();
    let binding = token();
    app.mail.send(&address, &code, "login").await?;
    {
        let db = app.db.lock().unwrap();
        db.execute("DELETE FROM login_codes WHERE expires<=?1", [now()])?;
        db.execute("DELETE FROM trusted_devices WHERE expires<=?1", [now()])?;
        // A new login attempt from this browser replaces its older challenge.
        if let Some(old) = cookie(headers, BINDING) {
            db.execute("DELETE FROM login_codes WHERE binding=?1", [digest(old)])?;
        }
        let count: i64 = db.query_row(
            "SELECT count(*) FROM login_codes WHERE user_id=?1",
            [id],
            |r| r.get(0),
        )?;
        if count >= 10 {
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many pending codes; wait ten minutes",
            ));
        }
        db.execute("INSERT INTO login_codes(challenge,user_id,hash,binding,expires) VALUES(?1,?2,?3,?4,?5)",params![challenge,id,email::code_hash(&challenge,&code),digest(&binding),now()+600])?;
    }
    let mut response =
        axum::Json(json!({"two_factor_required":true,"challenge":challenge})).into_response();
    response
        .headers_mut()
        .append("set-cookie", set_cookie(BINDING, &binding, 600));
    Ok(response)
}
#[derive(Deserialize)]
pub struct LoginCode {
    challenge: String,
    code: String,
    #[serde(default)]
    trust_device: bool,
}
pub async fn verify(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<LoginCode>,
) -> ApiResult<Response> {
    if input.challenge.len() > 64
        || input.code.len() != 6
        || !input.code.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(bad("Invalid or expired sign-in code"));
    }
    let binding = cookie(&headers, BINDING).ok_or_else(|| bad("Sign in again in this browser"))?;
    let trusted = if input.trust_device {
        Some(token())
    } else {
        None
    };
    let id = {
        let mut db = app.db.lock().unwrap();
        let tx = db.transaction()?;
        let entry:Option<(i64,String,u32)>=tx.query_row("SELECT c.user_id,c.hash,c.attempts FROM login_codes c JOIN users u ON u.id=c.user_id WHERE c.challenge=?1 AND c.binding=?2 AND c.expires>?3 AND u.verified=1 AND u.banned=0",params![input.challenge,digest(binding),now()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let (id, expected, attempts) =
            entry.ok_or_else(|| bad("Invalid or expired sign-in code"))?;
        if attempts >= 5 {
            return Err(bad("Too many attempts; sign in again for a new code"));
        }
        if expected != email::code_hash(&input.challenge, &input.code) {
            tx.execute(
                "UPDATE login_codes SET attempts=attempts+1 WHERE challenge=?1",
                [&input.challenge],
            )?;
            tx.commit()?;
            return Err(bad("Invalid or expired sign-in code"));
        }
        tx.execute(
            "DELETE FROM login_codes WHERE challenge=?1",
            [&input.challenge],
        )?;
        if let Some(raw) = &trusted {
            if let Some(old) = cookie(&headers, TRUST) {
                tx.execute(
                    "DELETE FROM trusted_devices WHERE hash=?1 AND user_id=?2",
                    params![digest(old), id],
                )?;
            }
            tx.execute("DELETE FROM trusted_devices WHERE user_id=?1 AND hash IN (SELECT hash FROM trusted_devices WHERE user_id=?1 ORDER BY expires DESC LIMIT -1 OFFSET 19)",[id])?;
            tx.execute(
                "INSERT INTO trusted_devices VALUES(?1,?2,?3)",
                params![digest(raw), id, now() + 30 * 86400],
            )?;
        }
        tx.commit()?;
        id
    };
    let session = app.session(id, &headers)?;
    if let Some(raw) = &trusted {
        app.db.lock().unwrap().execute(
            "UPDATE session_devices SET trust_hash=?1 WHERE hash=?2",
            params![digest(raw), digest(session.0["token"].as_str().unwrap())],
        )?;
    }
    let mut response = session_response(session);
    response
        .headers_mut()
        .append("set-cookie", set_cookie(BINDING, "", 0));
    if let Some(raw) = trusted {
        response
            .headers_mut()
            .append("set-cookie", set_cookie(TRUST, &raw, 30 * 86400));
    }
    Ok(response)
}
pub async fn revoke(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<Response> {
    let (id, _) = app.auth(&headers)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    tx.execute("DELETE FROM trusted_devices WHERE user_id=?1", [id])?;
    tx.execute("DELETE FROM login_codes WHERE user_id=?1", [id])?;
    tx.execute("DELETE FROM sessions WHERE user_id=?1", [id])?;
    tx.commit()?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .append("set-cookie", set_cookie(TRUST, "", 0));
    response
        .headers_mut()
        .append("set-cookie", set_cookie(BINDING, "", 0));
    response
        .headers_mut()
        .append("set-cookie", set_cookie("canna_session", "", 0));
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, fixture, value};
    use tower::ServiceExt;
    fn cookies(response: &Response, name: &str) -> String {
        response
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with(&format!("{name}=")))
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned()
    }
    async fn confirm(
        app: Shared,
        binding: &str,
        challenge: &str,
        code: &str,
        trust: bool,
    ) -> Response {
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/api/v1/login/verify")
            .header("content-type", "application/json")
            .header("cookie", binding)
            .header("origin", "https://cannamods.vip")
            .body(Body::from(
                json!({"challenge":challenge,"code":code,"trust_device":trust}).to_string(),
            ))
            .unwrap();
        router(app).oneshot(request).await.unwrap()
    }
    fn last_code(app: &App) -> String {
        match &app.mail {
            email::Mailer::Test(outbox) => outbox.lock().unwrap().last().unwrap().1.clone(),
            _ => panic!("Fixture mail required"),
        }
    }
    #[tokio::test]
    async fn desktop_verification_creates_a_named_device_and_logout_is_scoped() {
        use crate::tests::call;
        let (_dir, app) = fixture();
        let other_device = account(&app, "desktop", false);
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE users SET email='desktop@example.com'", [])
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "user-agent",
            "CannaDesktop/0.2.32 (Windows)".parse().unwrap(),
        );
        let response = start(app.clone(), 1, &headers).await.unwrap();
        let binding = cookies(&response, BINDING);
        let started = value(response).await;
        assert_eq!(started["two_factor_required"], true);
        assert!(started.get("token").is_none());
        let challenge = started["challenge"].as_str().unwrap();
        let code = last_code(&app);
        assert_eq!(
            confirm(app.clone(), "", challenge, &code, false)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/api/v1/login/verify")
            .header("content-type", "application/json")
            .header("cookie", &binding)
            .header("origin", "https://cannamods.vip")
            .header("user-agent", "CannaDesktop/0.2.32 (Windows)")
            .body(Body::from(
                json!({"challenge":challenge,"code":code,"trust_device":false}).to_string(),
            ))
            .unwrap();
        let response = router(app.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            !response
                .headers()
                .get_all("set-cookie")
                .iter()
                .any(|h| h.to_str().unwrap().starts_with(&format!("{TRUST}=")))
        );
        let signed_in = value(response).await;
        let token = signed_in["token"].as_str().unwrap();
        let devices = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/devices",
                Value::Null,
                Some(token),
            )
            .await,
        )
        .await;
        assert!(
            devices
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["current"] == true
                    && d["kind"] == "desktop"
                    && d["name"] == "Canna desktop on Windows")
        );
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/profiles/1",
                Value::Null,
                Some(token)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            confirm(app.clone(), &binding, challenge, &code, false)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/logout",
                Value::Null,
                Some(token)
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            call(app.clone(), "GET", "/api/v1/me", Value::Null, Some(token))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(app, "GET", "/api/v1/me", Value::Null, Some(&other_device))
                .await
                .status(),
            StatusCode::OK
        );
    }
    #[tokio::test]
    async fn login_requires_email_and_trust_is_scoped_expiring_and_revocable() {
        let (_dir, app) = fixture();
        let session = account(&app, "family", false);
        account(&app, "other", false);
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE users SET email=username || '@example.com'", [])
            .unwrap();
        let response = start(app.clone(), 1, &HeaderMap::new()).await.unwrap();
        assert!(
            response
                .headers()
                .get_all("set-cookie")
                .iter()
                .all(|h| !h.to_str().unwrap().starts_with("canna_session="))
        );
        let binding = cookies(&response, BINDING);
        let challenge = value(response).await["challenge"]
            .as_str()
            .unwrap()
            .to_owned();
        let code = last_code(&app);
        assert_eq!(
            confirm(app.clone(), "", &challenge, &code, true)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        let response = confirm(app.clone(), &binding, &challenge, &code, true).await;
        assert_eq!(response.status(), StatusCode::OK);
        let trust = cookies(&response, TRUST);
        assert!(
            response
                .headers()
                .get_all("set-cookie")
                .iter()
                .any(|h| h.to_str().unwrap().contains("Max-Age=2592000"))
        );
        assert_eq!(
            confirm(app.clone(), &binding, &challenge, &code, true)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        let mut headers = HeaderMap::new();
        headers.insert("cookie", trust.parse().unwrap());
        let trusted_response = start(app.clone(), 1, &headers).await.unwrap();
        let body = value(trusted_response).await;
        assert!(body["token"].is_string());
        assert_eq!(
            value(start(app.clone(), 2, &headers).await.unwrap()).await["two_factor_required"],
            true
        );
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE trusted_devices SET expires=?1", [now() - 1])
            .unwrap();
        assert_eq!(
            value(start(app.clone(), 1, &headers).await.unwrap()).await["two_factor_required"],
            true
        );
        headers.insert(
            "authorization",
            format!("Bearer {session}").parse().unwrap(),
        );
        // Revocation must remove active trust too, not just expired records.
        app.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO trusted_devices VALUES('active-device',1,?1)",
                [now() + 86400],
            )
            .unwrap();
        assert_eq!(
            revoke(State(app.clone()), headers).await.unwrap().status(),
            StatusCode::NO_CONTENT
        );
        let db = app.db.lock().unwrap();
        for table in ["trusted_devices", "login_codes", "sessions"] {
            assert_eq!(
                db.query_row(
                    &format!("SELECT count(*) FROM {table} WHERE user_id=1"),
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
        }
    }
    #[tokio::test]
    async fn login_codes_expire_and_lock_after_five_guesses() {
        let (_dir, app) = fixture();
        account(&app, "family", false);
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE users SET email='family@example.com'", [])
            .unwrap();
        let response = start(app.clone(), 1, &HeaderMap::new()).await.unwrap();
        let binding = cookies(&response, BINDING);
        let challenge = value(response).await["challenge"]
            .as_str()
            .unwrap()
            .to_owned();
        let code = last_code(&app);
        let wrong = if code == "000000" { "111111" } else { "000000" };
        for _ in 0..5 {
            assert_eq!(
                confirm(app.clone(), &binding, &challenge, wrong, false)
                    .await
                    .status(),
                StatusCode::BAD_REQUEST
            );
        }
        assert_eq!(
            confirm(app.clone(), &binding, &challenge, &code, false)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        let response = start(app.clone(), 1, &HeaderMap::new()).await.unwrap();
        let binding = cookies(&response, BINDING);
        let challenge = value(response).await["challenge"]
            .as_str()
            .unwrap()
            .to_owned();
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE login_codes SET expires=?1", [now() - 1])
            .unwrap();
        assert_eq!(
            confirm(app, &binding, &challenge, &code, false)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
}
