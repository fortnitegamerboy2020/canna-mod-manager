use super::*;
use rand::Rng;
pub enum Mailer {
    Disabled,
    Resend {
        client: reqwest::Client,
        key: Zeroizing<String>,
        from: String,
    },
    #[cfg(test)]
    Test(Mutex<Vec<(String, String)>>),
}
impl Mailer {
    pub fn configured() -> anyhow::Result<Self> {
        let key = super::credential("mail.key").unwrap_or_default();
        if key.trim().is_empty() {
            return Ok(Self::Disabled);
        }
        Ok(Self::Resend {
            client: reqwest::Client::builder()
                .https_only(true)
                .timeout(std::time::Duration::from_secs(15))
                .build()?,
            key,
            from: std::env::var("CANNA_MAIL_FROM")
                .unwrap_or_else(|_| "Canna <accounts@cannamods.vip>".into()),
        })
    }
    pub fn ready(&self) -> bool {
        !matches!(self, Self::Disabled)
    }
    pub(super) async fn send(&self, address: &str, code: &str, kind: &str) -> ApiResult<()> {
        match self {
            Self::Disabled => Err(ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "Email delivery is not configured yet; registration is paused",
            )),
            Self::Resend { client, key, from } => {
                let subject = match kind {
                    "verify" => "Verify your Canna account",
                    "login" => "Your Canna sign-in code",
                    _ => "Reset your Canna password",
                };
                let response=client.post("https://api.resend.com/emails").bearer_auth(&**key).json(&json!({"from":from,"to":[address],"subject":subject,"text":format!("Your Canna code is {code}. It expires in 10 minutes and works once. If you did not request this, ignore this email.")})).send().await.map_err(|_|ApiError(StatusCode::SERVICE_UNAVAILABLE,"Email delivery failed; please try again later"))?;
                if !response.status().is_success() {
                    return Err(ApiError(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "Email delivery failed; please try again later",
                    ));
                }
                Ok(())
            }
            #[cfg(test)]
            Self::Test(outbox) => {
                outbox.lock().unwrap().push((address.into(), code.into()));
                Ok(())
            }
        }
    }
}
fn address(value: &str) -> ApiResult<String> {
    let value = value.trim().to_ascii_lowercase();
    let parts: Vec<_> = value.split('@').collect();
    if value.len() > 254
        || !value.is_ascii()
        || parts.len() != 2
        || parts[0].is_empty()
        || parts[1].is_empty()
        || !parts[1].contains('.')
        || value.bytes().any(|b| b.is_ascii_whitespace() || b < 32)
    {
        return Err(bad("Enter a valid email address"));
    }
    Ok(value)
}
pub(super) fn challenge() -> (String, String) {
    (
        Uuid::new_v4().to_string(),
        format!("{:06}", OsRng.gen_range(0..1_000_000)),
    )
}
pub(super) fn code_hash(challenge: &str, code: &str) -> String {
    digest(&format!("{challenge}:{code}"))
}
pub async fn hash_password(app: &App, password: String) -> ApiResult<String> {
    let permit = app
        .auth_gate
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError(StatusCode::TOO_MANY_REQUESTS, "Please try again shortly"))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let password = Zeroizing::new(password);
        Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(65536, 3, 1, None).unwrap(),
        )
        .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
        .map(|hash| hash.to_string())
    })
    .await
    .map_err(|_| bad("Password hashing failed"))?
    .map_err(|_| bad("Password hashing failed"))
}
pub async fn register(
    State(app): State<Shared>,
    axum::Json(input): axum::Json<Credentials>,
) -> ApiResult<axum::Json<Value>> {
    if !valid_credentials(&input.username, &input.password) {
        return Err(bad(
            "Username must contain 3–32 letters/numbers; password must contain 12–256 bytes",
        ));
    }
    let email = address(&input.email)?;
    if !app.mail.ready() {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Email delivery is not configured yet; registration is paused",
        ));
    }
    let invite_hash = digest(&input.invite);
    let admin: bool = app
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT admin FROM invites WHERE hash=?1 AND expires>?2",
            params![invite_hash, now()],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| bad("Invitation is invalid or expired"))?;
    let taken: bool = app.db.lock().unwrap().query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE username=?1 OR email=?2)",
        params![input.username, email],
        |r| r.get(0),
    )?;
    if taken {
        return Err(bad(
            "An account with these details already exists; sign in or recover it",
        ));
    }
    let hash = hash_password(&app, input.password).await?;
    let (challenge, code) = challenge();
    app.mail.send(&email, &code, "verify").await?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    if tx.execute(
        "DELETE FROM invites WHERE hash=?1 AND expires>?2",
        params![invite_hash, now()],
    )? != 1
    {
        return Err(bad("Invitation was already used"));
    }
    tx.execute(
        "INSERT INTO users(username,password,admin,email,role) VALUES(?1,?2,?3,?4,?5)",
        params![
            input.username,
            hash,
            admin,
            email,
            if admin { "owner" } else { "member" }
        ],
    )?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "UPDATE invitation_history SET status='used',redeemed_by=?2,resolved=?3 WHERE hash=?1",
        params![invite_hash, id, now()],
    )?;
    tx.execute(
        "INSERT INTO codes(challenge,user_id,hash,kind,expires) VALUES(?1,?2,?3,'verify',?4)",
        params![challenge, id, code_hash(&challenge, &code), now() + 600],
    )?;
    tx.commit()?;
    Ok(axum::Json(
        json!({"verification_required":true,"challenge":challenge}),
    ))
}
#[derive(Deserialize)]
pub struct Code {
    pub challenge: String,
    pub code: String,
    #[serde(default)]
    pub password: String,
}
fn redeem(app: &App, input: &Code, kind: &str, password: Option<&str>) -> ApiResult<i64> {
    if input.challenge.len() > 64
        || input.code.len() != 6
        || !input.code.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(bad("Invalid or expired code"));
    }
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let entry: Option<(i64, String, u32)> = tx
        .query_row(
            "SELECT c.user_id,c.hash,c.attempts FROM codes c JOIN users u ON u.id=c.user_id WHERE challenge=?1 AND kind=?2 AND expires>?3 AND u.banned=0",
            params![input.challenge, kind, now()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (id, expected, attempts) = entry.ok_or_else(|| bad("Invalid or expired code"))?;
    if attempts >= 5 {
        return Err(bad("Too many attempts; request a new code"));
    }
    tx.execute(
        "UPDATE codes SET attempts=attempts+1 WHERE challenge=?1",
        [&input.challenge],
    )?;
    if expected != code_hash(&input.challenge, &input.code) {
        tx.commit()?;
        return Err(bad("Invalid or expired code"));
    }
    tx.execute(
        "DELETE FROM codes WHERE user_id=?1 AND kind=?2",
        params![id, kind],
    )?;
    if let Some(password) = password {
        tx.execute(
            "UPDATE users SET password=?1 WHERE id=?2 AND verified=1",
            params![password, id],
        )?;
        tx.execute("DELETE FROM sessions WHERE user_id=?1", [id])?;
        tx.execute("DELETE FROM trusted_devices WHERE user_id=?1", [id])?;
        tx.execute("DELETE FROM login_codes WHERE user_id=?1", [id])?;
    } else {
        tx.execute("UPDATE users SET verified=1 WHERE id=?1", [id])?;
    }
    tx.commit()?;
    Ok(id)
}
pub async fn verify(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Code>,
) -> ApiResult<Response> {
    let id = redeem(&app, &input, "verify", None)?;
    Ok(super::session_response(app.session(id, &headers)?))
}
#[derive(Deserialize)]
pub struct EmailRequest {
    pub email: String,
}
pub async fn forgot(
    State(app): State<Shared>,
    axum::Json(input): axum::Json<EmailRequest>,
) -> ApiResult<axum::Json<Value>> {
    let email = address(&input.email)?;
    let (challenge, code) = challenge();
    let id: Option<i64> = app
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT id FROM users WHERE email=?1 AND verified=1 AND banned=0",
            [&email],
            |r| r.get(0),
        )
        .optional()?;
    let issue = if let Some(id) = id {
        let db = app.db.lock().unwrap();
        let recent: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM codes WHERE user_id=?1 AND kind='reset' AND expires>?2)",
            params![id, now() + 540],
            |r| r.get(0),
        )?;
        if recent || !app.mail.ready() {
            None
        } else {
            db.execute("DELETE FROM codes WHERE user_id=?1 AND kind='reset'", [id])?;
            db.execute("INSERT INTO codes(challenge,user_id,hash,kind,expires) VALUES(?1,?2,?3,'reset',?4)",params![challenge,id,code_hash(&challenge,&code),now()+600])?;
            Some((email, code))
        }
    } else {
        None
    };
    // Response is identical and does not wait on the provider, preventing
    // enumeration by delivery latency. Delivery errors contain no email/code.
    if let Some((email, code)) = issue {
        let app = app.clone();
        tokio::spawn(async move {
            if app.mail.send(&email, &code, "reset").await.is_err() {
                eprintln!("Password-reset email delivery failed");
            }
        });
    }
    Ok(axum::Json(
        json!({"challenge":challenge,"message":"If a verified account exists, a reset code will be emailed. Wait one minute before requesting another code."}),
    ))
}
pub async fn reset(
    State(app): State<Shared>,
    axum::Json(input): axum::Json<Code>,
) -> ApiResult<axum::Json<Value>> {
    if !valid_credentials("abc", &input.password) {
        return Err(bad("Password must contain 12–256 bytes"));
    }
    let hash = hash_password(&app, input.password.clone()).await?;
    redeem(&app, &input, "reset", Some(&hash))?;
    Ok(axum::Json(json!({"ok":true})))
}
#[derive(Deserialize)]
pub struct ResendRequest {
    pub email: String,
    pub username: String,
}
pub async fn resend(
    State(app): State<Shared>,
    axum::Json(input): axum::Json<ResendRequest>,
) -> ApiResult<axum::Json<Value>> {
    let email = address(&input.email)?;
    let (challenge, code) = challenge();
    let id: Option<i64> = app
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT id FROM users WHERE email=?1 AND username=?2 AND verified=0 AND banned=0",
            params![email, input.username],
            |r| r.get(0),
        )
        .optional()?;
    let issue = if let Some(id) = id {
        let db = app.db.lock().unwrap();
        let recent: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM codes WHERE user_id=?1 AND kind='verify' AND expires>?2)",
            params![id, now() + 540],
            |r| r.get(0),
        )?;
        if recent || !app.mail.ready() {
            None
        } else {
            db.execute("DELETE FROM codes WHERE user_id=?1 AND kind='verify'", [id])?;
            db.execute("INSERT INTO codes(challenge,user_id,hash,kind,expires) VALUES(?1,?2,?3,'verify',?4)",params![challenge,id,code_hash(&challenge,&code),now()+600])?;
            Some((email, code))
        }
    } else {
        None
    };
    if let Some((email, code)) = issue {
        let app = app.clone();
        tokio::spawn(async move {
            if app.mail.send(&email, &code, "verify").await.is_err() {
                eprintln!("Verification email delivery failed");
            }
        });
    }
    Ok(axum::Json(
        json!({"challenge":challenge,"message":"If an unverified account matches, a verification code will be emailed. Wait one minute before requesting another code."}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codes_expire_are_single_use_and_reset_revokes_sessions() {
        let (_dir, app) = crate::tests::fixture();
        let db = app.db.lock().unwrap();
        db.execute("INSERT INTO users(id,username,password,email,verified) VALUES(1,'family','old-hash','family@example.com',1)", []).unwrap();
        db.execute("INSERT INTO sessions VALUES('session',1,?1)", [now() + 60])
            .unwrap();
        db.execute(
            "INSERT INTO trusted_devices VALUES('device',1,?1)",
            [now() + 86400],
        )
        .unwrap();
        db.execute(
            "INSERT INTO login_codes VALUES('signin',1,'hash','binding',?1,0)",
            [now() + 60],
        )
        .unwrap();
        for (id, expires) in [
            ("expired", now() - 1),
            ("reset", now() + 60),
            ("attempts", now() + 60),
        ] {
            db.execute(
                "INSERT INTO codes(challenge,user_id,hash,kind,expires) VALUES(?1,1,?2,'reset',?3)",
                params![id, code_hash(id, "123456"), expires],
            )
            .unwrap();
        }
        drop(db);
        let input = |id: &str, code: &str| Code {
            challenge: id.into(),
            code: code.into(),
            password: String::new(),
        };
        assert!(redeem(&app, &input("expired", "123456"), "reset", Some("new-hash")).is_err());
        for _ in 0..5 {
            assert!(
                redeem(
                    &app,
                    &input("attempts", "654321"),
                    "reset",
                    Some("new-hash")
                )
                .is_err()
            );
        }
        assert!(
            redeem(
                &app,
                &input("attempts", "123456"),
                "reset",
                Some("new-hash")
            )
            .is_err()
        );
        assert!(redeem(&app, &input("reset", "123456"), "verify", None).is_err());
        assert_eq!(
            redeem(&app, &input("reset", "123456"), "reset", Some("new-hash")).unwrap(),
            1
        );
        assert!(redeem(&app, &input("reset", "123456"), "reset", Some("new-hash")).is_err());
        let db = app.db.lock().unwrap();
        for table in ["trusted_devices", "login_codes"] {
            assert_eq!(
                db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        assert_eq!(
            db.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("SELECT password FROM users WHERE id=1", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "new-hash"
        );
    }
}
