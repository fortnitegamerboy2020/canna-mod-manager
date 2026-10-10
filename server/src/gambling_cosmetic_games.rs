//! Consumable contracts and escrowed duels use server catalog terms and the
//! same single-transaction receipt path as the wallet. No client awards items.
use super::*;

const RARITIES: [&str; 5] = ["common", "uncommon", "rare", "epic", "legendary"];
const SHOP_OFFSET: i64 = 19 * 3600; // Noon PDT, as requested.

pub(super) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS cosmetic_duels(id TEXT PRIMARY KEY,host INTEGER NOT NULL REFERENCES users(id),guest INTEGER REFERENCES users(id),host_item TEXT NOT NULL,guest_item TEXT,winner INTEGER REFERENCES users(id),status TEXT NOT NULL CHECK(status IN ('open','settled','cancelled','expired')),created INTEGER NOT NULL,expires INTEGER NOT NULL);
        CREATE INDEX IF NOT EXISTS cosmetic_duels_open ON cosmetic_duels(status,expires);
        CREATE TABLE IF NOT EXISTS cosmetic_shop_buys(user_id INTEGER NOT NULL REFERENCES users(id),day INTEGER NOT NULL,item_id TEXT NOT NULL,PRIMARY KEY(user_id,day,item_id));")
}
fn rank(item: &Value) -> ApiResult<usize> {
    RARITIES
        .iter()
        .position(|r| item["rarity"] == *r)
        .ok_or_else(|| bad("Cosmetic rarity is unavailable"))
}
fn next_pool(item: &Value) -> ApiResult<Vec<&'static Value>> {
    let next = rank(item)? + 1;
    if next >= RARITIES.len() {
        return Err(bad("Legendary decorations are already the highest rarity"));
    }
    let pool = catalog()?["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| {
            i["paused"] != true
                && i["shop_only"] != true
                && i["kind"] == item["kind"]
                && i["rarity"] == RARITIES[next]
        })
        .collect::<Vec<_>>();
    if pool.is_empty() {
        return Err(bad(
            "There are no available decorations at the next rarity for this kind",
        ));
    }
    Ok(pool)
}
pub(super) fn expire(db: &Connection) -> ApiResult<()> {
    let expired = db
        .prepare(
            "SELECT id,host,host_item FROM cosmetic_duels WHERE status='open' AND expires<=?1",
        )?
        .query_map([now()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (id, host, item) in expired {
        db.execute_batch("SAVEPOINT expire_cosmetic;")?;
        let returned = (|| -> ApiResult<()> {
            db.execute(
                "UPDATE cosmetic_duels SET status='expired' WHERE id=?1",
                [id],
            )?;
            market::give(db, host, &item)?;
            Ok(())
        })();
        if returned.is_err() {
            db.execute_batch("ROLLBACK TO expire_cosmetic;")?;
        }
        db.execute_batch("RELEASE expire_cosmetic;")?;
    }
    Ok(())
}
fn shop_day(time: i64) -> i64 {
    (time - SHOP_OFFSET).div_euclid(86400)
}
fn shop_entries(day: i64) -> ApiResult<Vec<Value>> {
    let mut pool = catalog()?["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["paused"] != true && i["shop_only"] == true)
        .collect::<Vec<_>>();
    pool.sort_by_key(|i| digest(&format!("canna-shop-v1:{day}:{}", i["id"])));
    Ok(pool
        .into_iter()
        .take(8)
        .map(|i| json!({"item_id":i["id"],"price":shop_price(i)}))
        .collect())
}
fn shop_price(item: &Value) -> i64 {
    [150, 300, 700, 1600, 4000][rank(item).unwrap_or(0)]
}
pub async fn contracts(
    State(app): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    let actor = app.auth(&headers)?.0;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    expire(&tx)?;
    let duels=tx.prepare("SELECT d.id,d.host,u.username,d.host_item,d.status,d.expires,d.winner FROM cosmetic_duels d JOIN users u ON u.id=d.host WHERE d.status='open' OR (d.host=?1 OR d.guest=?1) AND d.created>?2 ORDER BY d.created DESC,d.rowid DESC LIMIT 100")?.query_map(params![actor,now()-86400],|r|Ok(json!({"id":r.get::<_,String>(0)?,"host_id":r.get::<_,i64>(1)?,"host":r.get::<_,String>(2)?,"item_id":r.get::<_,String>(3)?,"status":r.get::<_,String>(4)?,"expires":r.get::<_,i64>(5)?,"winner_id":r.get::<_,Option<i64>>(6)?})))?.collect::<Result<Vec<_>,_>>()?;
    let day = shop_day(now());
    let purchased = tx
        .prepare("SELECT item_id FROM cosmetic_shop_buys WHERE user_id=?1 AND day=?2")?
        .query_map(params![actor, day], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let result = json!({"member_id":actor,"duels":duels,"upgrade_chance_percent":35,"trade_up_copies":5,"rarities":RARITIES,"shop":{"day":day,"next_rotation":(day+1)*86400+SHOP_OFFSET,"items":shop_entries(day)?,"purchased":purchased}});
    tx.commit()?;
    Ok(axum::Json(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractInput {
    request_id: String,
    action: String,
    #[serde(default)]
    items: Vec<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    duel_id: Option<String>,
    #[serde(default)]
    day: Option<i64>,
}
pub async fn contract_action(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<ContractInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = mutation_actor(&app, &headers)?;
    let kind = match input.action.as_str() {
        "trade_up" => "cosmetic_trade_up",
        "upgrade" => "cosmetic_upgrade",
        "create_duel" | "join_duel" => "cosmetic_duel",
        "cancel_duel" => "cosmetic_cancel",
        "shop_buy" => "cosmetic_shop",
        _ => return Err(bad("Choose a cosmetic contract action")),
    };
    once(
        &app,
        actor,
        &input.request_id,
        kind,
        &json!({"action":input.action,"items":input.items,"target":input.target,"duel_id":input.duel_id,"day":input.day}),
        |db| {
            expire(db)?;
            if wagering_paused(db)? && !matches!(input.action.as_str(), "cancel_duel" | "shop_buy")
            {
                return Err(bad("New wagering is paused"));
            }
            match input.action.as_str() {
                "trade_up" | "upgrade" => {
                    let trade = input.action == "trade_up";
                    if input.items.len() != if trade { 5 } else { 1 }
                        || input.duel_id.is_some()
                        || input.day.is_some()
                        || trade && input.target.is_some()
                    {
                        return Err(bad(
                            "Choose five matching-rarity copies for a trade-up, or one copy and a target for an upgrade",
                        ));
                    }
                    let first = market::item(&input.items[0])?;
                    let pool = next_pool(first)?;
                    for id in &input.items {
                        let item = market::item(id)?;
                        if item["kind"] != first["kind"] || item["rarity"] != first["rarity"] {
                            return Err(bad("Trade-up copies must have the same kind and rarity"));
                        }
                    }
                    let target = if trade {
                        *pool.choose(&mut rand::thread_rng()).unwrap()
                    } else {
                        let id = input
                            .target
                            .as_deref()
                            .ok_or_else(|| bad("Choose an upgrade target"))?;
                        *pool.iter().find(|i|i["id"]==id).ok_or_else(||bad("Upgrade target must be the same kind and exactly one rarity higher"))?
                    };
                    if market::capacity(db, actor, target["id"].as_str().unwrap())? < 1 {
                        return Err(bad("Target copy capacity reached"));
                    }
                    for id in &input.items {
                        market::take(db, actor, id)?;
                    }
                    let won = trade || rand::thread_rng().gen_range(0..100) < 35;
                    if won {
                        market::give(db, actor, target["id"].as_str().unwrap())?;
                    }
                    Ok(
                        json!({"action":input.action,"won":won,"consumed":input.items,"item":if won {Some(target)}else{None},"chance_percent":if trade {100}else{35}}),
                    )
                }
                "create_duel" => {
                    if input.items.len() != 1
                        || input.target.is_some()
                        || input.duel_id.is_some()
                        || input.day.is_some()
                    {
                        return Err(bad("Choose one decoration for your duel"));
                    }
                    let pending: i64 = db.query_row(
                        "SELECT count(*) FROM cosmetic_duels WHERE host=?1 AND status='open'",
                        [actor],
                        |r| r.get(0),
                    )?;
                    let total: i64 = db.query_row(
                        "SELECT count(*) FROM cosmetic_duels WHERE status='open'",
                        [],
                        |r| r.get(0),
                    )?;
                    if pending >= 5 || total >= 1000 {
                        return Err(bad("Open duel capacity reached"));
                    }
                    market::take(db, actor, &input.items[0])?;
                    let id = Uuid::new_v4().to_string();
                    db.execute("INSERT INTO cosmetic_duels(id,host,host_item,status,created,expires) VALUES(?1,?2,?3,'open',?4,?5)",params![id,actor,input.items[0],now(),now()+600])?;
                    Ok(json!({"duel_id":id,"status":"open","item_id":input.items[0]}))
                }
                "join_duel" | "cancel_duel" => {
                    let cancel = input.action == "cancel_duel";
                    if input.items.len() != usize::from(!cancel)
                        || input.target.is_some()
                        || input.day.is_some()
                    {
                        return Err(bad(
                            "Choose one matching decoration to join, or no decoration to cancel",
                        ));
                    }
                    let id = input
                        .duel_id
                        .as_deref()
                        .filter(|id| identifier(id))
                        .ok_or_else(|| bad("Choose a duel"))?;
                    let (host, host_item, status, expires): (i64, String, String, i64) = db
                        .query_row(
                            "SELECT host,host_item,status,expires FROM cosmetic_duels WHERE id=?1",
                            [id],
                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                        )
                        .optional()?
                        .ok_or_else(|| bad("Duel not found"))?;
                    if status != "open" || !cancel && expires <= now() {
                        return Err(ApiError(
                            StatusCode::CONFLICT,
                            "This duel is already closed",
                        ));
                    }
                    if cancel {
                        if host != actor {
                            return Err(ApiError(
                                StatusCode::FORBIDDEN,
                                "Only the host can cancel this duel",
                            ));
                        }
                        db.execute(
                            "UPDATE cosmetic_duels SET status='cancelled' WHERE id=?1",
                            [id],
                        )?;
                        market::give(db, actor, &host_item)?;
                        return Ok(json!({"duel_id":id,"status":"cancelled"}));
                    }
                    if host == actor {
                        return Err(bad("Another member must join your duel"));
                    }
                    let active: bool = db.query_row(
                        "SELECT verified=1 AND banned=0 FROM users WHERE id=?1",
                        [host],
                        |r| r.get(0),
                    )?;
                    if !active {
                        return Err(bad(
                            "The host is unavailable; this duel will expire and refund",
                        ));
                    }
                    let a = market::item(&host_item)?;
                    let b = market::item(&input.items[0])?;
                    if a["rarity"] != b["rarity"] || a["kind"] != b["kind"] {
                        return Err(bad("Both decorations must have the same kind and rarity"));
                    }
                    market::take(db, actor, &input.items[0])?;
                    // Release the held slot inside this transaction before
                    // checking that either possible winner can receive both.
                    db.execute(
                        "UPDATE cosmetic_duels SET status='settled' WHERE id=?1",
                        [id],
                    )?;
                    for user in [host, actor] {
                        for item in [&host_item, &input.items[0]] {
                            let needed = if host_item == input.items[0] { 2 } else { 1 };
                            if market::capacity(db, user, item)? < needed {
                                return Err(bad(
                                    "A player's collection is full; duel cannot settle",
                                ));
                            }
                        }
                    }
                    let winner = if rand::thread_rng().gen_bool(0.5) {
                        host
                    } else {
                        actor
                    };
                    market::give(db, winner, &host_item)?;
                    market::give(db, winner, &input.items[0])?;
                    db.execute("UPDATE cosmetic_duels SET guest=?1,guest_item=?2,winner=?3,status='settled' WHERE id=?4",params![actor,input.items[0],winner,id])?;
                    Ok(
                        json!({"duel_id":id,"status":"settled","winner_id":winner,"items":[host_item,input.items[0]]}),
                    )
                }
                "shop_buy" => {
                    if !input.items.is_empty() || input.duel_id.is_some() {
                        return Err(bad("Unexpected shop terms"));
                    }
                    let day = shop_day(now());
                    if input.day != Some(day) {
                        return Err(ApiError(
                            StatusCode::CONFLICT,
                            "The shop rotated; refresh before purchasing",
                        ));
                    }
                    let id = input
                        .target
                        .as_deref()
                        .ok_or_else(|| bad("Choose a shop decoration"))?;
                    let entries = shop_entries(day)?;
                    let entry = entries
                        .iter()
                        .find(|i| i["item_id"] == id)
                        .ok_or_else(|| bad("This decoration is not in today's shop"))?;
                    let price = entry["price"].as_i64().unwrap();
                    if db.query_row("SELECT EXISTS(SELECT 1 FROM cosmetic_shop_buys WHERE user_id=?1 AND day=?2 AND item_id=?3)",params![actor,day,id],|r|r.get::<_,bool>(0))? {return Err(bad("You already bought this decoration today"));}
                    debit(db, actor, price)?;
                    market::give(db, actor, id)?;
                    db.execute(
                        "INSERT INTO cosmetic_shop_buys VALUES(?1,?2,?3)",
                        params![actor, day, id],
                    )?;
                    Ok(json!({"item_id":id,"price":price,"day":day}))
                }
                _ => Err(bad("Unknown cosmetic action")),
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    fn pair() -> (&'static Value, &'static Value) {
        let a = catalog().unwrap()["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["paused"] != true && next_pool(i).is_ok())
            .unwrap();
        (a, next_pool(a).unwrap()[0])
    }
    #[tokio::test]
    async fn contracts_preserve_kind_ownership_and_exactly_once_consumption() {
        let (_dir, app) = fixture();
        let session = account(&app, "contract", false);
        let (a, b) = pair();
        let id = a["id"].as_str().unwrap();
        {
            let db = app.db.lock().unwrap();
            for _ in 0..6 {
                market::give(&db, 1, id).unwrap();
            }
        }
        let input = json!({"request_id":"trade-once","action":"trade_up","items":vec![id;5]});
        let result = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/contracts/action",
                input.clone(),
                Some(&session),
            )
            .await,
        )
        .await;
        assert_eq!(result["won"], true);
        assert_eq!(result["item"]["kind"], a["kind"]);
        assert_eq!(result["item"]["rarity"], b["rarity"]);
        assert_eq!(
            result,
            value(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/gambling/contracts/action",
                    input,
                    Some(&session)
                )
                .await
            )
            .await
        );
        let invalid =
            json!({"request_id":"bad-upgrade","action":"upgrade","items":[id],"target":id});
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/contracts/action",
                invalid,
                Some(&session)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(market::copies(&app.db.lock().unwrap(), 1, id).unwrap(), 1);
    }
    #[tokio::test]
    async fn duel_refunds_expired_and_cannot_be_joined_twice() {
        let (_dir, app) = fixture();
        let host = account(&app, "host", false);
        let guest = account(&app, "guest", false);
        let other = account(&app, "other", false);
        let (a, _) = pair();
        let id = a["id"].as_str().unwrap();
        {
            let db = app.db.lock().unwrap();
            for user in 1..=3 {
                market::give(&db, user, id).unwrap();
            }
        }
        let created = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/contracts/action",
                json!({"request_id":"create","action":"create_duel","items":[id]}),
                Some(&host),
            )
            .await,
        )
        .await;
        let duel = &created["duel_id"];
        let (a, b) = tokio::join!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/contracts/action",
                json!({"request_id":"join","action":"join_duel","items":[id],"duel_id":duel}),
                Some(&guest)
            ),
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/contracts/action",
                json!({"request_id":"join","action":"join_duel","items":[id],"duel_id":duel}),
                Some(&other)
            )
        );
        assert_eq!(
            usize::from(a.status() == StatusCode::OK) + usize::from(b.status() == StatusCode::OK),
            1
        );
        {
            let db = app.db.lock().unwrap();
            assert_eq!(
                (1..=3)
                    .map(|u| market::copies(&db, u, id).unwrap())
                    .sum::<i64>(),
                3
            );
            market::give(&db, 1, id).unwrap();
        }
        let created = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/contracts/action",
                json!({"request_id":"expire","action":"create_duel","items":[id]}),
                Some(&host),
            )
            .await,
        )
        .await;
        {
            let db = app.db.lock().unwrap();
            db.execute(
                "UPDATE cosmetic_duels SET expires=0 WHERE id=?1",
                [created["duel_id"].as_str().unwrap()],
            )
            .unwrap();
        }
        let before = market::copies(&app.db.lock().unwrap(), 1, id).unwrap();
        value(
            call(
                app.clone(),
                "GET",
                "/api/v1/gambling/contracts",
                Value::Null,
                Some(&host),
            )
            .await,
        )
        .await;
        assert_eq!(
            market::copies(&app.db.lock().unwrap(), 1, id).unwrap(),
            before + 1
        );
    }
    #[test]
    fn noon_pdt_rotation_is_stable_at_boundary() {
        assert_eq!(shop_day(SHOP_OFFSET - 1), -1);
        assert_eq!(shop_day(SHOP_OFFSET), 0);
        assert_eq!(shop_day(SHOP_OFFSET + 86399), 0);
        assert_eq!(shop_day(SHOP_OFFSET + 86400), 1);
    }
    #[tokio::test]
    async fn shop_reservations_rotate_and_purchase_once_without_reward_pool_leaks() {
        let (_dir, app) = fixture();
        let session = account(&app, "shop-member", false);
        let day = shop_day(now());
        let entries = shop_entries(day).unwrap();
        assert_eq!(entries.len(), 8);
        assert_eq!(entries, shop_entries(day).unwrap());
        assert_ne!(entries, shop_entries(day + 1).unwrap());
        let items = catalog().unwrap()["items"].as_array().unwrap();
        assert_eq!(items.iter().filter(|i| i["shop_only"] == true).count(), 100);
        for source in items.iter().filter(|i| next_pool(i).is_ok()) {
            assert!(
                next_pool(source)
                    .unwrap()
                    .iter()
                    .all(|i| i["shop_only"] != true)
            );
        }
        for case in super::super::CASE_IDS {
            assert!(
                super::super::case_pool(catalog().unwrap(), case)
                    .unwrap()
                    .0
                    .iter()
                    .all(|i| i["shop_only"] != true)
            );
        }
        let id = entries[0]["item_id"].as_str().unwrap();
        let price = entries[0]["price"].as_i64().unwrap();
        {
            let db = app.db.lock().unwrap();
            wallet(&db, 1).unwrap();
            db.execute("UPDATE bot_wallets SET balance=10000 WHERE user_id=1", [])
                .unwrap();
        }
        let input = json!({"request_id":"shop-once","action":"shop_buy","target":id,"day":day});
        let first = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/contracts/action",
                input.clone(),
                Some(&session),
            )
            .await,
        )
        .await;
        assert_eq!(first["price"], price);
        assert_eq!(
            first,
            value(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/gambling/contracts/action",
                    input,
                    Some(&session)
                )
                .await
            )
            .await
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/contracts/action",
                json!({"request_id":"shop-twice","action":"shop_buy","target":id,"day":day}),
                Some(&session)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/contracts/action",
                json!({"request_id":"shop-stale","action":"shop_buy","target":id,"day":day-1}),
                Some(&session)
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        let db = app.db.lock().unwrap();
        assert_eq!(market::copies(&db, 1, id).unwrap(), 1);
        assert_eq!(wallet(&db, 1).unwrap().0, 10000 - price);
    }
}
