use super::*;

pub fn rank(points: i64) -> &'static str {
    match points {
        0..=24 => "Noob",
        25..=99 => "Apprentice",
        100..=249 => "Modder",
        250..=749 => "Expert",
        750..=1499 => "Master",
        1500..=2999 => "Grandmaster",
        _ => "Pro Hacker",
    }
}
pub async fn directory(
    State(app): State<Shared>,
    headers: HeaderMap,
    Query(page): Query<lists::Page>,
) -> ApiResult<axum::Json<Value>> {
    app.auth(&headers)?;
    let db = app.db.lock().unwrap();
    let total:i64=db.query_row("SELECT COUNT(*) FROM users WHERE verified=1 AND banned=0 AND instr(lower(username),lower(?1))>0",[page.term()],|r|r.get(0))?;
    let mut stmt=db.prepare("SELECT u.id,u.username,u.role,p.status,p.avatar FROM users u LEFT JOIN profiles p ON p.user_id=u.id WHERE u.verified=1 AND u.banned=0 AND instr(lower(u.username),lower(?1))>0 ORDER BY u.username,u.id LIMIT ?2 OFFSET ?3")?;
    let users=stmt.query_map(params![page.term(),page.limit(500),page.offset()],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"username":r.get::<_,String>(1)?,"role":r.get::<_,String>(2)?,"status":r.get::<_,Option<String>>(3)?.unwrap_or_default(),"avatar":r.get::<_,Option<String>>(4)?.is_some()})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(axum::Json(page.response(users, total)))
}
pub async fn profile(
    State(app): State<Shared>,
    Path(target): Path<i64>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    let (viewer, moderator) = app.auth(&headers)?;
    let db = app.db.lock().unwrap();
    let info:Option<Value>=db.query_row("SELECT u.username,u.role,u.banned,p.status,p.bio,p.avatar FROM users u LEFT JOIN profiles p ON p.user_id=u.id WHERE u.id=?1 AND u.verified=1",[target],|r|Ok(json!({"id":target,"username":r.get::<_,String>(0)?,"role":r.get::<_,String>(1)?,"banned":r.get::<_,bool>(2)?,"status":r.get::<_,Option<String>>(3)?.unwrap_or_default(),"bio":r.get::<_,Option<String>>(4)?.unwrap_or_default(),"avatar":r.get::<_,Option<String>>(5)?.is_some()}))).optional()?;
    let mut info = info.ok_or(ApiError(StatusCode::NOT_FOUND, "Profile not found"))?;
    if info["banned"] == true && !moderator {
        return Err(ApiError(StatusCode::NOT_FOUND, "Profile not found"));
    }
    let posts: i64 = db.query_row(
        "SELECT count(*) FROM posts WHERE user_id=?1",
        [target],
        |r| r.get(0),
    )?;
    let topics: i64 = db.query_row(
        "SELECT count(*) FROM topics WHERE user_id=?1",
        [target],
        |r| r.get(0),
    )?;
    let comments: i64 = db.query_row(
        "SELECT count(*) FROM profile_comments WHERE author=?1",
        [target],
        |r| r.get(0),
    )?;
    let (stars,votes):(f64,i64)=db.query_row("SELECT COALESCE(AVG(r.stars),0),count(*) FROM ratings r JOIN users u ON u.id=r.voter WHERE r.target=?1 AND u.banned=0",[target],|r|Ok((r.get(0)?,r.get(1)?)))?;
    let own_rating: Option<i64> = db
        .query_row(
            "SELECT stars FROM ratings WHERE target=?1 AND voter=?2",
            params![target, viewer],
            |r| r.get(0),
        )
        .optional()?;
    let points = posts * 5 + topics * 5 + comments;
    info["rank"] = json!(rank(points));
    info["points"] = json!(points);
    info["posts_count"] = json!(posts);
    info["stars"] = json!(stars);
    info["ratings_count"] = json!(votes);
    info["my_rating"] = json!(own_rating);
    let mut stmt=db.prepare("SELECT c.id,c.author,u.username,c.body,c.created FROM profile_comments c JOIN users u ON u.id=c.author WHERE c.target=?1 ORDER BY c.created DESC,c.rowid DESC LIMIT 100")?;
    let items=stmt.query_map([target],|r|Ok(json!({"id":r.get::<_,String>(0)?,"author_id":r.get::<_,i64>(1)?,"author":r.get::<_,String>(2)?,"body":r.get::<_,String>(3)?,"created":r.get::<_,i64>(4)?})))?.collect::<Result<Vec<_>,_>>()?;
    info["comments"] = json!(items);
    Ok(axum::Json(info))
}
#[derive(Deserialize)]
pub struct ProfileInput {
    status: String,
    bio: String,
}
pub async fn update(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<ProfileInput>,
) -> ApiResult<axum::Json<Value>> {
    let (actor, _) = app.auth(&headers)?;
    if input.status.len() > 80 || input.bio.len() > 2000 {
        return Err(bad("Status is limited to 80 bytes and bio to 2000 bytes"));
    }
    app.db.lock().unwrap().execute("INSERT INTO profiles(user_id,status,bio) VALUES(?1,?2,?3) ON CONFLICT(user_id) DO UPDATE SET status=excluded.status,bio=excluded.bio",params![actor,input.status,input.bio])?;
    Ok(axum::Json(json!({"ok":true})))
}
pub async fn upload_avatar(
    State(app): State<Shared>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<axum::Json<Value>> {
    let (actor, _) = app.auth(&headers)?;
    let _permit = app.upload_gate.try_acquire().map_err(|_| {
        ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Another upload is in progress",
        )
    })?;
    let png = tokio::task::spawn_blocking(move || normalize_avatar(&body))
        .await
        .map_err(|_| bad("Image conversion failed"))??;
    let id = Uuid::new_v4().to_string();
    let path = app.files.join(format!("{id}.avatar"));
    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .await?;
    let mut writer =
        crypto::Writer::new(file, &app.upload_key, format!("avatar:{actor}:{id}")).await?;
    writer.write(&png).await?;
    writer.finish().await?;
    let old: Option<String> = {
        let db = app.db.lock().unwrap();
        let old = db
            .query_row(
                "SELECT avatar FROM profiles WHERE user_id=?1",
                [actor],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        db.execute("INSERT INTO profiles(user_id,avatar) VALUES(?1,?2) ON CONFLICT(user_id) DO UPDATE SET avatar=excluded.avatar",params![actor,id])?;
        old
    };
    if let Some(old) = old {
        let _ = tokio::fs::remove_file(app.files.join(format!("{old}.avatar"))).await;
    }
    Ok(axum::Json(json!({"ok":true})))
}
fn normalize_avatar(bytes: &[u8]) -> ApiResult<Vec<u8>> {
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(bad("Profile pictures are limited to 2 MiB"));
    }
    let format = image::guess_format(bytes).map_err(|_| bad("Use a PNG, JPEG or WebP image"))?;
    if !matches!(
        format,
        image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP
    ) {
        return Err(bad("Use a PNG, JPEG or WebP image"));
    }
    let reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let (width, height) = reader.into_dimensions().map_err(|_| bad("Invalid image"))?;
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err(bad("Image dimensions must be at most 4096 by 4096"));
    }
    let image = image::load_from_memory_with_format(bytes, format)
        .map_err(|_| bad("Invalid image"))?
        .resize_to_fill(256, 256, image::imageops::FilterType::Lanczos3);
    let mut output = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|_| bad("Image conversion failed"))?;
    Ok(output.into_inner())
}
pub async fn avatar(
    State(app): State<Shared>,
    Path(target): Path<i64>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    app.auth(&headers)?;
    let id:Option<String>=app.db.lock().unwrap().query_row("SELECT p.avatar FROM profiles p JOIN users u ON u.id=p.user_id WHERE p.user_id=?1 AND u.verified=1 AND u.banned=0",[target],|r|r.get::<_,Option<String>>(0)).optional()?.flatten();
    let id = id.ok_or(ApiError(StatusCode::NOT_FOUND, "No profile picture"))?;
    let file = tokio::fs::File::open(app.files.join(format!("{id}.avatar"))).await?;
    Ok((
        [("content-type", "image/png"), ("cache-control", "no-store")],
        Body::from_stream(crypto::read(
            file,
            Zeroizing::new(*app.upload_key),
            format!("avatar:{target}:{id}"),
        )),
    )
        .into_response())
}
#[derive(Deserialize)]
pub struct Comment {
    body: String,
}
pub async fn comment(
    State(app): State<Shared>,
    Path(target): Path<i64>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Comment>,
) -> ApiResult<axum::Json<Value>> {
    let (actor, _) = app.auth(&headers)?;
    if input.body.trim().is_empty() || input.body.len() > 2000 {
        return Err(bad("Write between 1 and 2000 bytes"));
    }
    let db = app.db.lock().unwrap();
    security::content_quota(&db, actor)?;
    let active: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND verified=1 AND banned=0)",
        [target],
        |r| r.get(0),
    )?;
    if !active {
        return Err(ApiError(StatusCode::NOT_FOUND, "Profile not found"));
    }
    let count: i64 = db.query_row(
        "SELECT count(*) FROM profile_comments WHERE target=?1",
        [target],
        |r| r.get(0),
    )?;
    if count >= 500 {
        return Err(bad("Profile comment limit reached"));
    }
    db.execute(
        "INSERT INTO profile_comments VALUES(?1,?2,?3,?4,?5)",
        params![Uuid::new_v4().to_string(), target, actor, input.body, now()],
    )?;
    Ok(axum::Json(json!({"ok":true})))
}
#[derive(Deserialize)]
pub struct Rating {
    stars: i64,
}
pub async fn rate(
    State(app): State<Shared>,
    Path(target): Path<i64>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Rating>,
) -> ApiResult<axum::Json<Value>> {
    let (actor, _) = app.auth(&headers)?;
    if target == actor || !(1..=5).contains(&input.stars) {
        return Err(bad("Rate another member from 1 to 5 stars"));
    }
    let db = app.db.lock().unwrap();
    let active: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND verified=1 AND banned=0)",
        [target],
        |r| r.get(0),
    )?;
    if !active {
        return Err(ApiError(StatusCode::NOT_FOUND, "Profile not found"));
    }
    db.execute("INSERT INTO ratings VALUES(?1,?2,?3) ON CONFLICT(target,voter) DO UPDATE SET stars=excluded.stars",params![target,actor,input.stars])?;
    Ok(axum::Json(json!({"ok":true})))
}
pub async fn delete_comment(
    State(app): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    let (actor, moderator) = app.auth(&headers)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    if tx.execute(
        "DELETE FROM profile_comments WHERE id=?1 AND (author=?2 OR target=?2 OR ?3)",
        params![id, actor, moderator],
    )? != 1
    {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "You cannot remove this comment",
        ));
    }
    tx.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,'delete-profile-comment',?2,?3)",
        params![actor, id, now()],
    )?;
    tx.commit()?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    #[tokio::test]
    async fn avatar_round_trip_is_encrypted_and_private() {
        use tower::ServiceExt;
        let (_dir, app) = fixture();
        let auth = account(&app, "family", false);
        let image = image::DynamicImage::new_rgba8(32, 64);
        let mut encoded = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/api/v1/profiles/me/avatar")
            .header("authorization", format!("Bearer {auth}"))
            .header("content-type", "image/png")
            .body(Body::from(encoded.into_inner()))
            .unwrap();
        assert_eq!(
            router(app.clone()).oneshot(request).await.unwrap().status(),
            StatusCode::OK
        );
        let file = std::fs::read_dir(&app.files)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(std::fs::read(file).unwrap().starts_with(b"CANNAEN1"));
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/profiles/1/avatar",
                Value::Null,
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        let response = call(
            app,
            "GET",
            "/api/v1/profiles/1/avatar",
            Value::Null,
            Some(&auth),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let png = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let image = image::load_from_memory(&png).unwrap();
        assert_eq!((image.width(), image.height()), (256, 256));
    }
    #[tokio::test]
    async fn reputation_is_one_vote_per_member_and_profiles_require_auth() {
        let (_dir, app) = fixture();
        let first = account(&app, "first", false);
        let second = account(&app, "second", false);
        assert_eq!(
            call(app.clone(), "GET", "/api/v1/profiles/1", Value::Null, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/profiles/1/rating",
                json!({"stars":5}),
                Some(&first)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/profiles/1/rating",
                json!({"stars":6}),
                Some(&second)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        for stars in [2, 5] {
            assert_eq!(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/profiles/1/rating",
                    json!({"stars":stars}),
                    Some(&second)
                )
                .await
                .status(),
                StatusCode::OK
            );
        }
        let profile = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/profiles/1",
                Value::Null,
                Some(&second),
            )
            .await,
        )
        .await;
        assert_eq!(profile["ratings_count"], 1);
        assert_eq!(profile["stars"], 5.0);
        assert_eq!(profile["role"], "member");
        assert_eq!(profile["rank"], "Noob");
        let input = json!({"status":"Modding","bio":"<script>never rendered as HTML</script>"});
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/profiles/me",
                input,
                Some(&first)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/profiles/1/comments",
                json!({"body":"Nice mods!"}),
                Some(&second)
            )
            .await
            .status(),
            StatusCode::OK
        );
        let profile = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/profiles/1",
                Value::Null,
                Some(&first),
            )
            .await,
        )
        .await;
        assert_eq!(profile["comments"].as_array().unwrap().len(), 1);
        let id = profile["comments"][0]["id"].as_str().unwrap();
        assert_eq!(
            call(
                app,
                "DELETE",
                &format!("/api/v1/profile-comments/{id}"),
                Value::Null,
                Some(&first)
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
    }
    #[test]
    fn avatar_normalization_strips_metadata_and_rejects_nonimages() {
        assert!(normalize_avatar(b"<svg onload='alert(1)'></svg>").is_err());
        let image = image::DynamicImage::new_rgba8(32, 64);
        let mut encoded = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let png = normalize_avatar(&encoded.into_inner()).unwrap();
        let normalized = image::load_from_memory(&png).unwrap();
        assert_eq!((normalized.width(), normalized.height()), (256, 256));
        assert_eq!(rank(0), "Noob");
        assert_eq!(rank(1500), "Grandmaster");
        assert_eq!(rank(3000), "Pro Hacker");
    }
}
