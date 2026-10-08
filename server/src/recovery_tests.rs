use crate::tests::{call, fixture, value};
use crate::*;
use tower::ServiceExt;

fn add_user(app: &App, name: &str, address: &str, verified: bool, banned: bool) -> i64 {
    let db = app.db.lock().unwrap();
    db.execute(
        "INSERT INTO users(username,password,email,verified,banned) VALUES(?1,'original-fixture-hash',?2,?3,?4)",
        params![name, address, verified, banned],
    )
    .unwrap();
    db.last_insert_rowid()
}
fn outbox(app: &App) -> Vec<(String, String)> {
    if let email::Mailer::Test(outbox) = &app.mail {
        outbox.lock().unwrap().clone()
    } else {
        panic!("Test mailer required");
    }
}
async fn deliver_pending() {
    // Test delivery has no I/O; allow spawned tasks to run without wall sleeps.
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn username_recovery_is_generic_and_only_emails_verified_unbanned_accounts() {
    let (_dir, app) = fixture();
    add_user(
        &app,
        "VisibleOnlyInEmail",
        "member@example.com",
        true,
        false,
    );
    add_user(
        &app,
        "UnverifiedName",
        "unverified@example.com",
        false,
        false,
    );
    add_user(&app, "BannedName", "banned@example.com", true, true);
    let mut responses = Vec::new();
    for address in [
        "unknown@example.com",
        "unverified@example.com",
        "banned@example.com",
        "  MEMBER@EXAMPLE.COM  ",
    ] {
        let response = call(
            app.clone(),
            "POST",
            "/api/v1/forgot-username",
            json!({"email":address}),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!response.headers().contains_key("set-cookie"));
        responses.push(value(response).await);
    }
    assert!(responses.windows(2).all(|pair| pair[0] == pair[1]));
    let text = responses[0].to_string();
    for secret in [
        "VisibleOnlyInEmail",
        "UnverifiedName",
        "BannedName",
        "member@example.com",
    ] {
        assert!(!text.contains(secret));
    }
    assert!(responses[0].get("challenge").is_none());
    deliver_pending().await;
    assert_eq!(
        outbox(&app),
        vec![("member@example.com".into(), "VisibleOnlyInEmail".into())]
    );
    let db = app.db.lock().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM codes WHERE kind='username'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM sessions", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn username_cooldown_suppresses_concurrent_mail_and_allows_later_reminder() {
    let (_dir, app) = fixture();
    let id = add_user(&app, "ConcurrentName", "member@example.com", true, false);
    let mut jobs = Vec::new();
    for _ in 0..12 {
        let app = app.clone();
        jobs.push(tokio::spawn(async move {
            let response = call(
                app,
                "POST",
                "/api/v1/forgot-username",
                json!({"email":"member@example.com"}),
                None,
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            value(response).await
        }));
    }
    let mut responses = Vec::new();
    for job in jobs {
        responses.push(job.await.unwrap());
    }
    assert!(responses.windows(2).all(|pair| pair[0] == pair[1]));
    deliver_pending().await;
    assert_eq!(outbox(&app).len(), 1);
    {
        let db = app.db.lock().unwrap();
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM codes WHERE user_id=?1 AND kind='username'",
                [id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        // Marker age > one minute, while its ten-minute retention is active.
        db.execute(
            "UPDATE codes SET expires=?1 WHERE user_id=?2 AND kind='username'",
            params![now() + 500, id],
        )
        .unwrap();
    }
    let later = value(
        call(
            app.clone(),
            "POST",
            "/api/v1/forgot-username",
            json!({"email":"member@example.com"}),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(later, responses[0]);
    deliver_pending().await;
    assert_eq!(outbox(&app).len(), 2);
    assert_eq!(
        app.db
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM codes WHERE user_id=?1 AND kind='username'",
                [id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn username_reminders_preserve_credentials_sessions_and_other_recovery_codes() {
    let (_dir, app) = fixture();
    let id = add_user(&app, "PreservedName", "member@example.com", true, false);
    {
        let db = app.db.lock().unwrap();
        db.execute(
            "INSERT INTO sessions VALUES('existing-session',?1,?2)",
            params![id, now() + 3600],
        )
        .unwrap();
        db.execute(
            "INSERT INTO trusted_devices VALUES('existing-device',?1,?2)",
            params![id, now() + 86400],
        )
        .unwrap();
        db.execute(
            "INSERT INTO login_codes VALUES('existing-login',?1,'login-hash','binding',?2,0)",
            params![id, now() + 600],
        )
        .unwrap();
        for (challenge, kind) in [("existing-reset", "reset"), ("existing-verify", "verify")] {
            db.execute("INSERT INTO codes(challenge,user_id,hash,kind,expires,attempts) VALUES(?1,?2,'existing-hash',?3,?4,2)",params![challenge,id,kind,now()+600]).unwrap();
        }
    }
    let response = call(
        app.clone(),
        "POST",
        "/api/v1/forgot-username",
        json!({"email":"member@example.com"}),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    deliver_pending().await;
    let db = app.db.lock().unwrap();
    let fields: (String, i64) = db
        .query_row(
            "SELECT password,verified FROM users WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(fields, ("original-fixture-hash".into(), 1));
    for table in ["sessions", "trusted_devices", "login_codes"] {
        assert_eq!(
            db.query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE user_id=?1"),
                [id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    for challenge in ["existing-reset", "existing-verify"] {
        let code: (String, u32) = db
            .query_row(
                "SELECT hash,attempts FROM codes WHERE challenge=?1",
                [challenge],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(code, ("existing-hash".into(), 2));
    }
}

#[tokio::test]
async fn username_cooldown_markers_cannot_redeem_as_password_email_or_login_codes() {
    let (_dir, app) = fixture();
    let id = add_user(&app, "MarkerName", "member@example.com", true, false);
    app.db.lock().unwrap().execute("INSERT INTO codes(challenge,user_id,hash,kind,expires) VALUES('known-marker',?1,?2,'username',?3)",params![id,email::code_hash("known-marker","123456"),now()+600]).unwrap();
    for endpoint in ["verify-email", "reset-password", "login/verify"] {
        let payload = json!({"challenge":"known-marker","code":"123456","password":"new-testing-password-123","trust_device":false});
        let response = if endpoint == "login/verify" {
            // Test the isolated login-code table rather than a missing-cookie guard.
            let request = axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/login/verify")
                .header("content-type", "application/json")
                .header("origin", "https://cannamods.vip")
                .header("cookie", format!("__Host-canna_login={}", "b".repeat(64)))
                .body(Body::from(payload.to_string()))
                .unwrap();
            router(app.clone()).oneshot(request).await.unwrap()
        } else {
            call(
                app.clone(),
                "POST",
                &format!("/api/v1/{endpoint}"),
                payload,
                None,
            )
            .await
        };
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{endpoint}");
        assert!(!response.headers().contains_key("set-cookie"));
    }
    let db = app.db.lock().unwrap();
    assert_eq!(
        db.query_row("SELECT password FROM users WHERE id=?1", [id], |row| row
            .get::<_, String>(
            0
        ))
        .unwrap(),
        "original-fixture-hash"
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM sessions", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT attempts FROM codes WHERE challenge='known-marker'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn disabled_delivery_and_unknown_account_have_same_public_response() {
    let dir = tempfile::tempdir().unwrap();
    let app = Arc::new(
        App::open(
            &dir.path().join("db.sqlite"),
            dir.path().join("files"),
            &"23".repeat(32),
            [23; 32],
            email::Mailer::Disabled,
        )
        .unwrap(),
    );
    add_user(&app, "DisabledName", "member@example.com", true, false);
    let mut responses = Vec::new();
    for address in ["member@example.com", "unknown@example.com"] {
        let response = call(
            app.clone(),
            "POST",
            "/api/v1/forgot-username",
            json!({"email":address}),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        responses.push(value(response).await);
    }
    assert_eq!(responses[0], responses[1]);
    assert_eq!(
        app.db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM codes", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn username_reminder_obeys_browser_origin_and_authentication_rate_limit() {
    let (_dir, app) = fixture();
    add_user(&app, "OriginName", "member@example.com", true, false);
    for (origin, expected) in [
        ("https://evil.example", StatusCode::FORBIDDEN),
        ("https://cannamods.vip", StatusCode::OK),
    ] {
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/api/v1/forgot-username")
            .header("content-type", "application/json")
            .header("origin", origin)
            .body(Body::from(
                json!({"email":"member@example.com"}).to_string(),
            ))
            .unwrap();
        assert_eq!(
            router(app.clone()).oneshot(request).await.unwrap().status(),
            expected
        );
    }
    deliver_pending().await;
    assert_eq!(outbox(&app).len(), 1);
    let (_rate_dir, rate_app) = fixture();
    for _ in 0..30 {
        assert_eq!(
            call(
                rate_app.clone(),
                "POST",
                "/api/v1/forgot-username",
                json!({"email":"unknown@example.com"}),
                None
            )
            .await
            .status(),
            StatusCode::OK
        );
    }
    assert_eq!(
        call(
            rate_app,
            "POST",
            "/api/v1/forgot-username",
            json!({"email":"unknown@example.com"}),
            None
        )
        .await
        .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}
