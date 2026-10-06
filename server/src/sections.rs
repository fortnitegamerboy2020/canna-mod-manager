use super::*;

#[derive(Clone, serde::Serialize, Deserialize)]
pub struct Section {
    id: String,
    name: String,
    description: String,
    active: bool,
    vip_only: bool,
    #[serde(default = "default_group")]
    group: String,
}
fn default_group() -> String {
    "unity".into()
}
#[derive(Clone, serde::Serialize, Deserialize)]
pub struct Group {
    id: String,
    name: String,
}
#[derive(serde::Serialize, Deserialize)]
pub struct Layout {
    revision: i64,
    sections: Vec<Section>,
    #[serde(default)]
    groups: Vec<Group>,
    #[serde(default)]
    moves: std::collections::HashMap<String, String>,
}
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS forum_sections(id TEXT PRIMARY KEY,name TEXT NOT NULL,description TEXT NOT NULL,active INTEGER NOT NULL,vip_only INTEGER NOT NULL,position INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS forum_layout(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS section_reviews(hash TEXT PRIMARY KEY,actor INTEGER NOT NULL REFERENCES users(id),revision INTEGER NOT NULL,payload TEXT NOT NULL,expires INTEGER NOT NULL);")?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS forum_groups(id TEXT PRIMARY KEY,name TEXT NOT NULL,position INTEGER NOT NULL);")?;
    let has_group = db
        .prepare("PRAGMA table_info(forum_sections)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|name| name == "group_id");
    if !has_group {
        db.execute_batch("ALTER TABLE forum_sections ADD COLUMN group_id TEXT NOT NULL DEFAULT 'unity'; INSERT OR IGNORE INTO forum_groups VALUES('unity','Unity modding',0);")?;
    }
    if db.execute("INSERT OR IGNORE INTO forum_layout VALUES(1,0)", [])? == 1 {
        for (position, (id, name, description)) in [
            (
                "help",
                "Modding help",
                "Unity, BepInEx, Harmony and the bugs in between.",
            ),
            (
                "showcase",
                "Releases & showcases",
                "Show your work and follow what others are building.",
            ),
            (
                "discussion",
                "General discussion",
                "Ideas, game nights and everything around modding.",
            ),
            (
                "guides",
                "Guides & resources",
                "Useful code, walkthroughs and shared discoveries.",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            db.execute(
                "INSERT INTO forum_sections(id,name,description,active,vip_only,position) VALUES(?1,?2,?3,1,?4,?5)",
                params![id, name, description, id == "guides", position as i64],
            )?;
        }
    }
    Ok(())
}
fn layout(db: &Connection) -> ApiResult<Layout> {
    let revision = db.query_row("SELECT revision FROM forum_layout WHERE id=1", [], |r| {
        r.get(0)
    })?;
    let mut stmt = db.prepare(
        "SELECT id,name,description,active,vip_only,group_id FROM forum_sections ORDER BY position,id",
    )?;
    let sections = stmt
        .query_map([], |r| {
            Ok(Section {
                id: r.get(0)?,
                name: r.get(1)?,
                description: r.get(2)?,
                active: r.get(3)?,
                vip_only: r.get(4)?,
                group: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let groups = db
        .prepare("SELECT id,name FROM forum_groups ORDER BY position,id")?
        .query_map([], |r| {
            Ok(Group {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Layout {
        groups,
        revision,
        sections,
        moves: Default::default(),
    })
}
pub async fn list(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    app.auth(&headers)?;
    Ok(axum::Json(json!(layout(&app.db.lock().unwrap())?)))
}
fn validate(input: &Layout, existing: &Layout) -> ApiResult<()> {
    if input.sections.is_empty()
        || input.sections.len() > 32
        || !input.sections.iter().any(|s| s.active && !s.vip_only)
    {
        return Err(bad("Keep 1–32 sections and at least one open to members"));
    }
    if input.groups.is_empty() || input.groups.len() > 16 {
        return Err(bad("Keep 1–16 forum groups"));
    }
    let mut group_ids = std::collections::HashSet::new();
    let mut group_names = std::collections::HashSet::new();
    for group in &input.groups {
        if group.id.is_empty()
            || group.id.len() > 64
            || !group
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || !group_ids.insert(&group.id)
            || group.name.trim().is_empty()
            || group.name.len() > 80
            || group.name.chars().any(char::is_control)
            || !group_names.insert(group.name.trim().to_lowercase())
        {
            return Err(bad("Forum groups need unique IDs and names up to 80 bytes"));
        }
    }
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for s in &input.sections {
        if !group_ids.contains(&s.group)
            || s.id.is_empty()
            || s.id.len() > 64
            || !s
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || !ids.insert(&s.id)
            || s.name.trim().is_empty()
            || s.name.len() > 80
            || s.description.len() > 300
            || s.name.chars().any(char::is_control)
            || s.description.chars().any(char::is_control)
            || !names.insert(s.name.trim().to_lowercase())
        {
            return Err(bad(
                "Use unique names and IDs, names up to 80 bytes and descriptions up to 300 bytes",
            ));
        }
    }
    for (source, target) in &input.moves {
        if !existing.sections.iter().any(|s| &s.id == source)
            || ids.contains(source)
            || !ids.contains(target)
        {
            return Err(bad(
                "Choose a remaining category for each deleted category's discussions",
            ));
        }
    }
    Ok(())
}
pub async fn review(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(mut input): axum::Json<Layout>,
) -> ApiResult<axum::Json<Value>> {
    let actor = community::owner(&app, &headers)?;
    for s in &mut input.sections {
        s.name = s.name.trim().to_owned();
        s.description = s.description.trim().to_owned();
    }
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let existing = layout(&tx)?;
    if existing.revision != input.revision {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Sections changed; reload before reviewing",
        ));
    }
    if input.groups.is_empty() {
        input.groups = existing.groups.clone();
    }
    for group in &mut input.groups {
        group.name = group.name.trim().to_owned();
    }
    validate(&input, &existing)?;
    for removed in existing
        .sections
        .iter()
        .filter(|s| !input.sections.iter().any(|v| v.id == s.id))
    {
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM topics WHERE category=?1",
            [&removed.id],
            |r| r.get(0),
        )?;
        if count > 0 && !input.moves.contains_key(&removed.id) {
            return Err(bad(
                "This category has discussions; select a category to move them into",
            ));
        }
    }
    let token = Uuid::new_v4().to_string();
    tx.execute(
        "DELETE FROM section_reviews WHERE actor=?1 OR expires<=?2",
        params![actor, now()],
    )?;
    tx.execute(
        "INSERT INTO section_reviews VALUES(?1,?2,?3,?4,?5)",
        params![
            digest(&token),
            actor,
            input.revision,
            input_as_json(&input)?,
            now() + 600
        ],
    )?;
    tx.commit()?;
    Ok(axum::Json(json!({"token":token,"layout":input})))
}
fn input_as_json(input: &Layout) -> ApiResult<String> {
    serde_json::to_string(input).map_err(|_| bad("Invalid section layout"))
}
#[derive(Deserialize)]
pub struct Apply {
    token: String,
}
pub async fn apply(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Apply>,
) -> ApiResult<axum::Json<Value>> {
    let actor = community::owner(&app, &headers)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let (revision,payload):(i64,String)=tx.query_row("SELECT revision,payload FROM section_reviews WHERE hash=?1 AND actor=?2 AND expires>?3",params![digest(&input.token),actor,now()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(bad("Review changes again; confirmation expired or was already used"))?;
    let existing = layout(&tx)?;
    if revision != existing.revision {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Sections changed; reload and review again",
        ));
    }
    let input: Layout =
        serde_json::from_str(&payload).map_err(|_| bad("Invalid section review"))?;
    validate(&input, &existing)?;
    for removed in existing
        .sections
        .iter()
        .filter(|s| !input.sections.iter().any(|v| v.id == s.id))
    {
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM topics WHERE category=?1",
            [&removed.id],
            |r| r.get(0),
        )?;
        if count > 0 && !input.moves.contains_key(&removed.id) {
            return Err(bad(
                "This category has discussions; select a category to move them into",
            ));
        }
    }
    tx.execute("DELETE FROM forum_groups", [])?;
    for (position, group) in input.groups.iter().enumerate() {
        tx.execute(
            "INSERT INTO forum_groups VALUES(?1,?2,?3)",
            params![group.id, group.name, position as i64],
        )?;
    }
    for (position, s) in input.sections.iter().enumerate() {
        tx.execute("INSERT INTO forum_sections(id,name,description,active,vip_only,position,group_id) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET name=excluded.name,description=excluded.description,active=excluded.active,vip_only=excluded.vip_only,position=excluded.position,group_id=excluded.group_id",params![s.id,s.name,s.description,s.active,s.vip_only,position as i64,s.group])?;
    }
    for removed in existing
        .sections
        .iter()
        .filter(|s| !input.sections.iter().any(|v| v.id == s.id))
    {
        if let Some(target) = input.moves.get(&removed.id) {
            tx.execute(
                "UPDATE topics SET category=?1 WHERE category=?2",
                params![target, removed.id],
            )?;
        }
        tx.execute("DELETE FROM forum_sections WHERE id=?1", [&removed.id])?;
    }
    tx.execute("UPDATE forum_layout SET revision=revision+1 WHERE id=1", [])?;
    tx.execute("DELETE FROM section_reviews", [])?;
    tx.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,'forum-sections',?2,?3)",
        params![actor, payload, now()],
    )?;
    tx.commit()?;
    Ok(axum::Json(json!({"ok":true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};

    #[tokio::test]
    async fn groups_move_sections_without_changing_discussions() {
        let (_dir, app) = fixture();
        let owner = account(&app, "owner", true);
        let original = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/sections",
                Value::Null,
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(original["groups"][0]["name"], "Unity modding");
        let mut draft = original.clone();
        draft["groups"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"minecraft","name":"Minecraft"}));
        draft["sections"][0]["group"] = json!("minecraft");
        let mut invalid = draft.clone();
        invalid["groups"].as_array_mut().unwrap().remove(0);
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/review",
                invalid,
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        let review = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/review",
                draft,
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(
            value(
                call(
                    app.clone(),
                    "GET",
                    "/api/v1/sections",
                    Value::Null,
                    Some(&owner)
                )
                .await
            )
            .await,
            original
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/apply",
                json!({"token":review["token"]}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::OK
        );
        let result = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/sections",
                Value::Null,
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(result["groups"].as_array().unwrap().len(), 2);
        assert_eq!(result["sections"][0]["id"], "help");
        assert_eq!(result["sections"][0]["group"], "minecraft");
        let topic=value(call(app.clone(),"POST","/api/v1/topics",json!({"title":"Group discussion","body":"Posting inside the reassigned section","category":"help","app_id":1686940}),Some(&owner)).await).await;
        assert!(topic["id"].is_string());
        let db = app.db.lock().unwrap();
        initialize(&db).unwrap();
        assert_eq!(layout(&db).unwrap().groups.len(), 2);
    }

    #[tokio::test]
    async fn deletion_moves_discussions_and_requires_valid_destination() {
        let (_dir, app) = fixture();
        let owner = account(&app, "owner", true);
        let topic=value(call(app.clone(),"POST","/api/v1/topics",json!({"title":"Keep this discussion","body":"Keep this post","category":"help","app_id":1686940}),Some(&owner)).await).await;
        let mut draft = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/sections",
                Value::Null,
                Some(&owner),
            )
            .await,
        )
        .await;
        draft["sections"].as_array_mut().unwrap().remove(0);
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/review",
                draft.clone(),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        draft["moves"] = json!({"help":"discussion"});
        let review = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/review",
                draft,
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/apply",
                json!({"token":review["token"]}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::OK
        );
        let db = app.db.lock().unwrap();
        let category: String = db
            .query_row(
                "SELECT category FROM topics WHERE id=?1",
                [topic["id"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(category, "discussion");
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM posts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM forum_sections WHERE id='help'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
    #[tokio::test]
    async fn sections_require_owner_review_then_single_use_confirmation() {
        let (_dir, app) = fixture();
        let owner = account(&app, "owner", true);
        let member = account(&app, "member", false);
        let admin = account(&app, "admin", false);
        app.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE users SET role='admin',admin=1 WHERE username='admin'",
                [],
            )
            .unwrap();
        let original = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/sections",
                Value::Null,
                Some(&owner),
            )
            .await,
        )
        .await;
        let mut draft = original.clone();
        draft["sections"][0]["name"] = json!("Unity questions");
        draft["sections"].as_array_mut().unwrap().push(json!({"id":"new-section","name":"Game night","description":"Family sessions","active":true,"vip_only":false}));
        for auth in [&member, &admin] {
            assert_eq!(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/admin/sections/review",
                    draft.clone(),
                    Some(auth)
                )
                .await
                .status(),
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/apply",
                json!({"token":"no-review"}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        let review = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/review",
                draft.clone(),
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(
            value(
                call(
                    app.clone(),
                    "GET",
                    "/api/v1/sections",
                    Value::Null,
                    Some(&owner)
                )
                .await
            )
            .await,
            original
        );
        let token = json!({"token":review["token"]});
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/apply",
                token.clone(),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/apply",
                token.clone(),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/apply",
                token,
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        let saved = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/sections",
                Value::Null,
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(saved["revision"], 1);
        assert_eq!(saved["sections"][0]["name"], "Unity questions");
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/review",
                draft,
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        let mut invalid = saved.clone();
        invalid["sections"].as_array_mut().unwrap().remove(0);
        invalid["moves"] = json!({"help":"help"});
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/review",
                invalid,
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        let topic = json!({"title":"Family night","body":"A new discussion","category":"new-section","app_id":1686940});
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/topics",
                topic.clone(),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::OK
        );
        let mut closed = saved;
        closed["sections"][4]["active"] = json!(false);
        let review = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/review",
                closed,
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/apply",
                json!({"token":review["token"]}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            call(app.clone(), "POST", "/api/v1/topics", topic, Some(&member))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn section_reviews_expire_and_are_bound_to_layout_revision() {
        let (_dir, app) = fixture();
        let owner = account(&app, "owner", true);
        let draft = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/sections",
                Value::Null,
                Some(&owner),
            )
            .await,
        )
        .await;
        let review = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/review",
                draft.clone(),
                Some(&owner),
            )
            .await,
        )
        .await;
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE section_reviews SET expires=0", [])
            .unwrap();
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/apply",
                json!({"token":review["token"]}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        let review = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/review",
                draft,
                Some(&owner),
            )
            .await,
        )
        .await;
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE forum_layout SET revision=revision+1", [])
            .unwrap();
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/sections/apply",
                json!({"token":review["token"]}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM audit WHERE action='forum-sections'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
}
