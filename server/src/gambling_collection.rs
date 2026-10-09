//! Account-owned collection actions. A replay never recycles twice.
use super::*;

pub(super) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS gambling_favorites(user_id INTEGER NOT NULL REFERENCES users(id),item_id TEXT NOT NULL,PRIMARY KEY(user_id,item_id)); CREATE TABLE IF NOT EXISTS gambling_styles(user_id INTEGER NOT NULL REFERENCES users(id),slot INTEGER NOT NULL CHECK(slot BETWEEN 1 AND 5),name TEXT NOT NULL,equipment TEXT NOT NULL,PRIMARY KEY(user_id,slot));")
}
pub(super) fn view(db: &Connection, actor: i64) -> ApiResult<Value> {
    let favorites = db
        .prepare("SELECT item_id FROM gambling_favorites WHERE user_id=?1 ORDER BY item_id")?
        .query_map([actor], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let styles = db
        .prepare("SELECT slot,name,equipment FROM gambling_styles WHERE user_id=?1 ORDER BY slot")?
        .query_map([actor], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter_map(|(slot, name, equipment)| {
            serde_json::from_str::<Value>(&equipment)
                .ok()
                .map(|v| json!({"slot":slot,"name":name,"equipped":v}))
        })
        .collect::<Vec<_>>();
    Ok(
        json!({"favorites":favorites,"styles":styles,"recycle_values":{"common":5,"uncommon":10,"rare":20,"epic":40,"legendary":100}}),
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManageInput {
    request_id: String,
    action: String,
    #[serde(default)]
    item_id: Option<String>,
    #[serde(default)]
    favorite: Option<bool>,
    #[serde(default)]
    slot: Option<i64>,
    #[serde(default)]
    name: Option<String>,
}
pub async fn manage(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<ManageInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = mutation_actor(&app, &headers)?;
    let payload = json!({"action":input.action,"item_id":input.item_id,"favorite":input.favorite,"slot":input.slot,"name":input.name});
    once(
        &app,
        actor,
        &input.request_id,
        "collection_manage",
        &payload,
        |db| match input.action.as_str() {
            "favorite" | "recycle" => {
                if input.slot.is_some() || input.name.is_some() {
                    return Err(bad("Unexpected style settings"));
                }
                let id = input
                    .item_id
                    .as_deref()
                    .ok_or_else(|| bad("Choose a cosmetic"))?;
                let item = catalog()?["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|i| i["id"] == id)
                    .ok_or_else(|| bad("Cosmetic not found"))?;
                let count: i64 = db
                    .query_row(
                        "SELECT count FROM gambling_cosmetics WHERE user_id=?1 AND item_id=?2",
                        params![actor, id],
                        |r| r.get(0),
                    )
                    .optional()?
                    .unwrap_or(0);
                if count < 1 {
                    return Err(ApiError(
                        StatusCode::FORBIDDEN,
                        "You do not own this cosmetic",
                    ));
                }
                if input.action == "favorite" {
                    let favorite = input
                        .favorite
                        .ok_or_else(|| bad("Choose a favorite state"))?;
                    if favorite {
                        db.execute(
                            "INSERT OR IGNORE INTO gambling_favorites VALUES(?1,?2)",
                            params![actor, id],
                        )?;
                    } else {
                        db.execute(
                            "DELETE FROM gambling_favorites WHERE user_id=?1 AND item_id=?2",
                            params![actor, id],
                        )?;
                    }
                    Ok(json!({"item_id":id,"favorite":favorite}))
                } else {
                    if input.favorite.is_some() {
                        return Err(bad("Unexpected favorite setting"));
                    }
                    if count < 2 {
                        return Err(bad(
                            "Only spare copies can be recycled; your first copy is kept",
                        ));
                    }
                    let reward = match item["rarity"].as_str() {
                        Some("legendary") => 100,
                        Some("epic") => 40,
                        Some("rare") => 20,
                        Some("uncommon") => 10,
                        _ => 5,
                    };
                    if cannabot::MAX_KASH - wallet(db, actor)?.0 < reward {
                        return Err(bad("Your wallet is full; the spare copy was kept"));
                    }
                    db.execute("UPDATE gambling_cosmetics SET count=count-1 WHERE user_id=?1 AND item_id=?2 AND count>1",params![actor,id])?;
                    let paid = credit(db, actor, reward, reward)?;
                    Ok(json!({"item_id":id,"count":count-1,"reward":paid}))
                }
            }
            "save_style" | "load_style" | "delete_style" => {
                if input.item_id.is_some() || input.favorite.is_some() {
                    return Err(bad("Unexpected cosmetic settings"));
                }
                let slot = input
                    .slot
                    .filter(|s| (1..=5).contains(s))
                    .ok_or_else(|| bad("Choose style slot 1 to 5"))?;
                if input.action == "save_style" {
                    let name = input.name.as_deref().unwrap_or("").trim();
                    if name.is_empty()
                        || name.chars().count() > 40
                        || name.chars().any(char::is_control)
                    {
                        return Err(bad("Name the style using 1 to 40 characters"));
                    }
                    let equipment:Value=db.query_row("SELECT frame,banner,emblem,name_effect FROM gambling_equipped WHERE user_id=?1",[actor],|r|Ok(json!({"frame":r.get::<_,Option<String>>(0)?,"banner":r.get::<_,Option<String>>(1)?,"emblem":r.get::<_,Option<String>>(2)?,"name_effect":r.get::<_,Option<String>>(3)?}))).optional()?.unwrap_or(json!({"frame":null,"banner":null,"emblem":null,"name_effect":null}));
                    db.execute("INSERT INTO gambling_styles VALUES(?1,?2,?3,?4) ON CONFLICT(user_id,slot) DO UPDATE SET name=excluded.name,equipment=excluded.equipment",params![actor,slot,name,equipment.to_string()])?;
                    Ok(json!({"slot":slot,"name":name,"equipped":equipment}))
                } else {
                    if input.name.is_some() {
                        return Err(bad("Unexpected style name"));
                    }
                    if input.action == "delete_style" {
                        db.execute(
                            "DELETE FROM gambling_styles WHERE user_id=?1 AND slot=?2",
                            params![actor, slot],
                        )?;
                        return Ok(json!({"slot":slot}));
                    }
                    let text: String = db
                        .query_row(
                            "SELECT equipment FROM gambling_styles WHERE user_id=?1 AND slot=?2",
                            params![actor, slot],
                            |r| r.get(0),
                        )
                        .optional()?
                        .ok_or_else(|| bad("Save this style slot first"))?;
                    let equipment: Value = serde_json::from_str(&text)
                        .map_err(|_| bad("Saved style is unavailable"))?;
                    for kind in ["frame", "banner", "emblem", "name_effect"] {
                        if let Some(id) = equipment[kind].as_str() {
                            let available =
                                catalog()?["items"].as_array().unwrap().iter().any(|i| {
                                    i["id"] == id && i["kind"] == kind && i["paused"] != true
                                });
                            let owned:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM gambling_cosmetics WHERE user_id=?1 AND item_id=?2 AND count>0)",params![actor,id],|r|r.get(0))?;
                            if !available || !owned {
                                return Err(bad(
                                    "This style includes an unavailable or paused cosmetic; your current style was kept",
                                ));
                            }
                        }
                    }
                    db.execute("INSERT INTO gambling_equipped(user_id,frame,banner,emblem,name_effect) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(user_id) DO UPDATE SET frame=excluded.frame,banner=excluded.banner,emblem=excluded.emblem,name_effect=excluded.name_effect",params![actor,equipment["frame"].as_str(),equipment["banner"].as_str(),equipment["emblem"].as_str(),equipment["name_effect"].as_str()])?;
                    Ok(json!({"slot":slot,"equipped":equipment}))
                }
            }
            _ => Err(bad("Choose a collection action")),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    #[tokio::test]
    async fn recycling_preserves_first_copy_and_style_ownership_and_replays() {
        let (_dir, app) = fixture();
        let a = account(&app, "collection-owner", false);
        let b = account(&app, "collection-other", false);
        let id = "frame-mint-halo";
        app.db
            .lock()
            .unwrap()
            .execute("INSERT INTO gambling_cosmetics VALUES(1,?1,3)", [id])
            .unwrap();
        let route = "/api/v1/gambling/cosmetics/manage";
        let input = json!({"request_id":"recycle","action":"recycle","item_id":id});
        let first = value(call(app.clone(), "POST", route, input.clone(), Some(&a)).await).await;
        assert_eq!(first["count"], 2);
        assert!(first["reward"].as_i64().unwrap() > 0);
        assert_eq!(
            value(call(app.clone(), "POST", route, input, Some(&a)).await).await,
            first
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                route,
                json!({"request_id":"foreign","action":"recycle","item_id":id}),
                Some(&b)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                route,
                json!({"request_id":"two","action":"recycle","item_id":id}),
                Some(&a)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                route,
                json!({"request_id":"three","action":"recycle","item_id":id}),
                Some(&a)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                route,
                json!({"request_id":"favorite","action":"favorite","item_id":id,"favorite":true}),
                Some(&a)
            )
            .await
            .status(),
            StatusCode::OK
        );
        app.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO gambling_equipped(user_id,frame) VALUES(1,?1)",
                [id],
            )
            .unwrap();
        assert_eq!(
            call(
                app.clone(),
                "POST",
                route,
                json!({"request_id":"save","action":"save_style","slot":1,"name":"First"}),
                Some(&a)
            )
            .await
            .status(),
            StatusCode::OK
        );
        app.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE gambling_equipped SET frame=NULL WHERE user_id=1",
                [],
            )
            .unwrap();
        assert_eq!(
            call(
                app.clone(),
                "POST",
                route,
                json!({"request_id":"load","action":"load_style","slot":1}),
                Some(&a)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            equipped(&app.db.lock().unwrap(), 1).unwrap()["frame"]["id"],
            id
        );
        assert_eq!(
            view(&app.db.lock().unwrap(), 2).unwrap()["styles"],
            json!([])
        );
    }
}
