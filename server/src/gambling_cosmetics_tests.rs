use super::*;
use crate::tests::{account, call, fixture, value};

#[tokio::test]
async fn equipment_migration_preserves_old_slots_and_is_idempotent() {
    let (_dir, app) = fixture();
    account(&app, "legacy-equipment", false);
    let db = app.db.lock().unwrap();
    db.execute_batch("DROP TABLE gambling_equipped; CREATE TABLE gambling_equipped(user_id INTEGER PRIMARY KEY,frame TEXT,banner TEXT); INSERT INTO gambling_equipped VALUES(1,'frame-mint-halo','banner-canna-night');").unwrap();
    initialize(&db).unwrap();
    initialize(&db).unwrap();
    let info = equipped(&db, 1).unwrap();
    assert_eq!(info["frame"]["id"], "frame-mint-halo");
    assert_eq!(info["banner"]["id"], "banner-canna-night");
    assert!(info["emblem"].is_null() && info["name_effect"].is_null());
}

#[tokio::test]
async fn emblems_and_gradients_require_ownership_preserve_independent_slots_and_clear_explicitly() {
    let (_dir, app) = fixture();
    let a = account(&app, "cosmetic-owner", false);
    let b = account(&app, "cosmetic-other", false);
    let catalog = catalog().unwrap();
    let emblem = catalog["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["collection"] == "cod-ranks")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let effect = "name-effect-aurora";
    let selection =
        json!({"frame":"frame-mint-halo","banner":null,"emblem":emblem,"name_effect":effect});
    assert_eq!(
        call(
            app.clone(),
            "POST",
            "/api/v1/gambling/cosmetics/equip",
            selection.clone(),
            Some(&a)
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    {
        let db = app.db.lock().unwrap();
        for id in ["frame-mint-halo", emblem, effect] {
            db.execute("INSERT INTO gambling_cosmetics VALUES(1,?1,1)", [id])
                .unwrap();
        }
    }
    let saved = value(
        call(
            app.clone(),
            "POST",
            "/api/v1/gambling/cosmetics/equip",
            selection.clone(),
            Some(&a),
        )
        .await,
    )
    .await;
    assert_eq!(saved["equipped"]["emblem"], emblem);
    assert_eq!(saved["equipped"]["name_effect"], effect);
    assert_eq!(
        call(
            app.clone(),
            "POST",
            "/api/v1/gambling/cosmetics/equip",
            selection,
            Some(&b)
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    let wrong = json!({"frame":null,"banner":null,"emblem":effect,"name_effect":emblem});
    assert_eq!(
        call(
            app.clone(),
            "POST",
            "/api/v1/gambling/cosmetics/equip",
            wrong,
            Some(&a)
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    // A legacy client may edit a frame without knowing the new slot names.
    let legacy = value(
        call(
            app.clone(),
            "POST",
            "/api/v1/gambling/cosmetics/equip",
            json!({"frame":null,"banner":null}),
            Some(&a),
        )
        .await,
    )
    .await;
    assert_eq!(legacy["equipped"]["emblem"], emblem);
    assert_eq!(legacy["equipped"]["name_effect"], effect);
    let me = value(call(app.clone(), "GET", "/api/v1/me", Value::Null, Some(&a)).await).await;
    assert_eq!(me["cosmetics"]["name_effect"]["style"], "aurora");
    let profile = value(
        call(
            app.clone(),
            "GET",
            "/api/v1/profiles/1",
            Value::Null,
            Some(&b),
        )
        .await,
    )
    .await;
    assert_eq!(profile["cosmetics"]["emblem"]["id"], emblem);
    let directory = value(
        call(
            app.clone(),
            "GET",
            "/api/v1/profiles?page=1",
            Value::Null,
            Some(&a),
        )
        .await,
    )
    .await;
    assert!(
        directory["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == 1 && i["cosmetics"]["name_effect"]["id"] == effect)
    );
    let cleared = value(
        call(
            app.clone(),
            "POST",
            "/api/v1/gambling/cosmetics/equip",
            json!({"frame":null,"banner":null,"emblem":null,"name_effect":null}),
            Some(&a),
        )
        .await,
    )
    .await;
    assert!(
        cleared["equipped"]["emblem"].is_null() && cleared["equipped"]["name_effect"].is_null()
    );
    assert_eq!(
        app.db
            .lock()
            .unwrap()
            .query_row(
                "SELECT SUM(count) FROM gambling_cosmetics WHERE user_id=1",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        3
    );
}

#[tokio::test]
async fn new_crates_select_their_kind_and_replay_without_second_charge() {
    let (_dir, app) = fixture();
    let auth = account(&app, "new-crates", false);
    {
        let db = app.db.lock().unwrap();
        wallet(&db, 1).unwrap();
        db.execute("UPDATE bot_wallets SET balance=1000 WHERE user_id=1", [])
            .unwrap();
    }
    for (case_id, kind) in [
        ("cod-emblems", "emblem"),
        ("username-effects", "name_effect"),
    ] {
        let input = json!({"request_id":case_id,"case_id":case_id});
        let first = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/cases/open",
                input.clone(),
                Some(&auth),
            )
            .await,
        )
        .await;
        let replay = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/cases/open",
                input,
                Some(&auth),
            )
            .await,
        )
        .await;
        assert_eq!(first, replay);
        assert_eq!(first["item"]["kind"], kind);
        assert_eq!(first["count"], 1);
    }
    let db = app.db.lock().unwrap();
    assert_eq!(wallet(&db, 1).unwrap().0, 865); // 60-Kash emblems + 75-Kash effects
    assert_eq!(
        db.query_row(
            "SELECT SUM(count) FROM gambling_cosmetics WHERE user_id=1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}

#[tokio::test]
async fn fast_crash_snapshots_are_authenticated_fresh_and_exclude_large_catalogs() {
    let (_dir, app) = fixture();
    let auth = account(&app, "fast-crash", false);
    let path = "/api/v1/gambling?crash_only=true";
    assert_eq!(
        call(app.clone(), "GET", path, Value::Null, None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let first = value(call(app.clone(), "GET", path, Value::Null, Some(&auth)).await).await;
    let id = first["crash"]["id"].clone();
    {
        let db = app.db.lock().unwrap();
        db.execute("UPDATE bot_wallets SET balance=777 WHERE user_id=1", [])
            .unwrap();
    }
    let next = value(call(app.clone(), "GET", path, Value::Null, Some(&auth)).await).await;
    assert_eq!(next["member_id"], 1);
    assert_eq!(next["wallet"]["balance"], 777);
    assert_eq!(next["crash"]["id"], id);
    assert!(next["server_time_ms"].as_i64().unwrap() >= first["server_time_ms"].as_i64().unwrap());
    for key in ["cosmetics", "cases", "blackjack", "recent_games", "rules"] {
        assert!(next.get(key).is_none(), "fast response leaked {key}");
    }
    assert!(next["crash"].get("planned_crash_multiplier").is_none());
    assert!(next["crash"].get("planned_crash_at_ms").is_none());
}
