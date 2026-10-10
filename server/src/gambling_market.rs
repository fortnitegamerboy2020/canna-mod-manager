//! A listing holds one real inventory copy. Purchase, custody release and wallet
//! transfer share the idempotent Immediate transaction used by other Kash actions.
use super::*;

pub(super) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS gambling_market(id TEXT PRIMARY KEY,seller INTEGER NOT NULL REFERENCES users(id),item_id TEXT NOT NULL,price INTEGER NOT NULL CHECK(price BETWEEN 1 AND 9007199254740991),status TEXT NOT NULL CHECK(status IN ('listed','sold','cancelled')),buyer INTEGER REFERENCES users(id),created INTEGER NOT NULL,closed INTEGER);
        CREATE INDEX IF NOT EXISTS gambling_market_open ON gambling_market(status,created);
        CREATE INDEX IF NOT EXISTS gambling_market_seller ON gambling_market(seller,status);")
}
pub(super) fn item(id: &str) -> ApiResult<&'static Value> {
    catalog()?["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id && item["paused"] != true)
        .ok_or_else(|| bad("Cosmetic is unavailable or paused"))
}
pub(super) fn copies(db: &Connection, actor: i64, id: &str) -> ApiResult<i64> {
    Ok(db
        .query_row(
            "SELECT count FROM gambling_cosmetics WHERE user_id=?1 AND item_id=?2",
            params![actor, id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0))
}
pub(super) fn take(db: &Connection, actor: i64, id: &str) -> ApiResult<()> {
    item(id)?;
    let count = copies(db, actor, id)?;
    if count < 1 {
        return Err(bad("You do not own that cosmetic"));
    }
    if count == 1 {
        db.execute(
            "DELETE FROM gambling_cosmetics WHERE user_id=?1 AND item_id=?2",
            params![actor, id],
        )?;
        for column in ["frame", "banner", "emblem", "name_effect"] {
            db.execute(
                &format!(
                    "UPDATE gambling_equipped SET {column}=NULL WHERE user_id=?1 AND {column}=?2"
                ),
                params![actor, id],
            )?;
        }
    } else {
        db.execute(
            "UPDATE gambling_cosmetics SET count=count-1 WHERE user_id=?1 AND item_id=?2",
            params![actor, id],
        )?;
    }
    Ok(())
}
pub(super) fn capacity(db: &Connection, actor: i64, id: &str) -> ApiResult<i64> {
    let held:i64=db.query_row("SELECT (SELECT count(*) FROM gambling_market WHERE seller=?1 AND item_id=?2 AND status='listed')+(SELECT count(*) FROM cosmetic_duels WHERE host=?1 AND host_item=?2 AND status='open')",params![actor,id],|r|r.get(0))?;
    Ok(1_000_000 - copies(db, actor, id)? - held)
}
pub(super) fn give(db: &Connection, actor: i64, id: &str) -> ApiResult<()> {
    if capacity(db, actor, id)? < 1 {
        return Err(bad(
            "Cosmetic copy capacity reached; recycle or sell a spare before collecting this item",
        ));
    }
    db.execute("INSERT INTO gambling_cosmetics VALUES(?1,?2,1) ON CONFLICT(user_id,item_id) DO UPDATE SET count=count+1",params![actor,id])?;
    Ok(())
}
pub(super) fn transfer_credit(db: &Connection, actor: i64, amount: i64) -> ApiResult<()> {
    let balance = wallet(db, actor)?.0;
    if amount < 0 || amount > cannabot::MAX_KASH - balance {
        return Err(bad("Recipient wallet is full; transfer cannot complete"));
    }
    db.execute(
        "UPDATE bot_wallets SET balance=balance+?1 WHERE user_id=?2",
        params![amount, actor],
    )?;
    Ok(())
}
#[derive(Deserialize)]
pub struct MarketQuery {
    #[serde(default)]
    after: i64,
}
pub async fn market(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<MarketQuery>,
) -> ApiResult<axum::Json<Value>> {
    let actor = app.auth(&headers)?.0;
    if query.after < 0 {
        return Err(bad("Invalid market cursor"));
    }
    let db = app.db.lock().unwrap();
    let rows=db.prepare("SELECT m.rowid,m.id,m.seller,u.username,m.item_id,m.price,m.status,m.created FROM gambling_market m JOIN users u ON u.id=m.seller WHERE (m.status='listed' OR m.seller=?1) AND m.rowid>?2 ORDER BY m.rowid LIMIT 101")?
        .query_map(params![actor,query.after],|r|Ok(json!({"cursor":r.get::<_,i64>(0)?,"id":r.get::<_,String>(1)?,"seller_id":r.get::<_,i64>(2)?,"seller":r.get::<_,String>(3)?,"item_id":r.get::<_,String>(4)?,"price":r.get::<_,i64>(5)?,"status":r.get::<_,String>(6)?,"created":r.get::<_,i64>(7)?})))?.collect::<Result<Vec<_>,_>>()?;
    let has_more = rows.len() > 100;
    let rows = rows.into_iter().take(100).collect::<Vec<_>>();
    let next = rows
        .last()
        .and_then(|r| r["cursor"].as_i64())
        .unwrap_or(query.after);
    Ok(axum::Json(
        json!({"member_id":actor,"listings":rows,"has_more":has_more,"next":next}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketInput {
    request_id: String,
    action: String,
    #[serde(default)]
    item_id: Option<String>,
    #[serde(default)]
    listing_id: Option<String>,
    #[serde(default)]
    price: Option<i64>,
}
pub async fn market_action(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<MarketInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = mutation_actor(&app, &headers)?;
    once(
        &app,
        actor,
        &input.request_id,
        "market_action",
        &json!({"action":input.action,"item_id":input.item_id,"listing_id":input.listing_id,"price":input.price}),
        |db| match input.action.as_str() {
            "list" => {
                if input.listing_id.is_some() {
                    return Err(bad("Unexpected listing identity"));
                }
                let id = input
                    .item_id
                    .as_deref()
                    .ok_or_else(|| bad("Choose an owned cosmetic"))?;
                let price = input
                    .price
                    .filter(|p| (1..=MAX_STAKE).contains(p))
                    .ok_or_else(|| bad("Choose a positive whole-Kash price"))?;
                let count: i64 = db.query_row(
                    "SELECT count(*) FROM gambling_market WHERE seller=?1 AND status='listed'",
                    [actor],
                    |r| r.get(0),
                )?;
                let all: i64 = db.query_row(
                    "SELECT count(*) FROM gambling_market WHERE status='listed'",
                    [],
                    |r| r.get(0),
                )?;
                if count >= 20 || all >= 10_000 {
                    return Err(bad("Marketplace listing capacity reached"));
                }
                take(db, actor, id)?;
                let listing = Uuid::new_v4().to_string();
                db.execute("INSERT INTO gambling_market(id,seller,item_id,price,status,created) VALUES(?1,?2,?3,?4,'listed',?5)",params![listing,actor,id,price,now()])?;
                Ok(json!({"listing_id":listing,"item_id":id,"price":price,"status":"listed"}))
            }
            "buy" | "cancel" => {
                if input.item_id.is_some() || input.price.is_some() {
                    return Err(bad(
                        "Listing terms cannot be changed during purchase or cancellation",
                    ));
                }
                let listing = input
                    .listing_id
                    .as_deref()
                    .filter(|s| identifier(s))
                    .ok_or_else(|| bad("Choose a listing"))?;
                let (seller, id, price, status): (i64, String, i64, String) = db
                    .query_row(
                        "SELECT seller,item_id,price,status FROM gambling_market WHERE id=?1",
                        [listing],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                    )
                    .optional()?
                    .ok_or_else(|| bad("Listing not found"))?;
                if status != "listed" {
                    return Err(ApiError(StatusCode::CONFLICT, "Listing is already closed"));
                }
                if input.action == "cancel" {
                    if seller != actor {
                        return Err(ApiError(
                            StatusCode::FORBIDDEN,
                            "Only the seller can cancel this listing",
                        ));
                    }
                    db.execute(
                        "UPDATE gambling_market SET status='cancelled',closed=?1 WHERE id=?2",
                        params![now(), listing],
                    )?;
                    give(db, seller, &id)?;
                    Ok(json!({"listing_id":listing,"item_id":id,"status":"cancelled"}))
                } else {
                    if seller == actor {
                        return Err(bad("You cannot buy your own listing"));
                    }
                    item(&id)?;
                    debit(db, actor, price)?;
                    transfer_credit(db, seller, price)?;
                    give(db, actor, &id)?;
                    db.execute(
                        "UPDATE gambling_market SET status='sold',buyer=?1,closed=?2 WHERE id=?3",
                        params![actor, now(), listing],
                    )?;
                    Ok(
                        json!({"listing_id":listing,"item_id":id,"price":price,"seller_id":seller,"status":"sold"}),
                    )
                }
            }
            _ => Err(bad("Choose list, buy or cancel")),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    fn setup(app: &App) -> String {
        let id = catalog().unwrap()["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["paused"] != true)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let db = app.db.lock().unwrap();
        give(&db, 1, &id).unwrap();
        wallet(&db, 1).unwrap();
        wallet(&db, 2).unwrap();
        db.execute("UPDATE bot_wallets SET balance=1000", [])
            .unwrap();
        id
    }
    #[tokio::test]
    async fn purchase_moves_one_copy_and_exact_kash_once() {
        let (_dir, app) = fixture();
        let seller = account(&app, "seller", false);
        let buyer = account(&app, "buyer", false);
        let id = setup(&app);
        let input = json!({"request_id":"list-once","action":"list","item_id":id,"price":250});
        let listed = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/market/action",
                input.clone(),
                Some(&seller),
            )
            .await,
        )
        .await;
        assert_eq!(
            listed,
            value(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/gambling/market/action",
                    input,
                    Some(&seller)
                )
                .await
            )
            .await
        );
        let purchase =
            json!({"request_id":"buy-once","action":"buy","listing_id":listed["listing_id"]});
        let bought = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/market/action",
                purchase.clone(),
                Some(&buyer),
            )
            .await,
        )
        .await;
        assert_eq!(
            bought,
            value(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/gambling/market/action",
                    purchase,
                    Some(&buyer)
                )
                .await
            )
            .await
        );
        let db = app.db.lock().unwrap();
        assert_eq!(copies(&db, 1, &id).unwrap(), 0);
        assert_eq!(copies(&db, 2, &id).unwrap(), 1);
        assert_eq!(wallet(&db, 1).unwrap().0, 1250);
        assert_eq!(wallet(&db, 2).unwrap().0, 750);
    }
    #[tokio::test]
    async fn unauthorized_cancel_overflow_and_two_buyers_preserve_custody() {
        let (_dir, app) = fixture();
        let seller = account(&app, "seller", false);
        let buyer = account(&app, "buyer", false);
        let other = account(&app, "other", false);
        let id = setup(&app);
        let listed = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/market/action",
                json!({"request_id":"list","action":"list","item_id":id,"price":10}),
                Some(&seller),
            )
            .await,
        )
        .await;
        let listing = &listed["listing_id"];
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/market/action",
                json!({"request_id":"steal","action":"cancel","listing_id":listing}),
                Some(&buyer)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        {
            let db = app.db.lock().unwrap();
            db.execute(
                "UPDATE bot_wallets SET balance=?1 WHERE user_id=1",
                [cannabot::MAX_KASH],
            )
            .unwrap();
        }
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/market/action",
                json!({"request_id":"full","action":"buy","listing_id":listing}),
                Some(&buyer)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        {
            let db = app.db.lock().unwrap();
            assert_eq!(wallet(&db, 2).unwrap().0, 1000);
            assert_eq!(copies(&db, 2, &id).unwrap(), 0);
            db.execute("UPDATE bot_wallets SET balance=1000 WHERE user_id=1", [])
                .unwrap();
            wallet(&db, 3).unwrap();
            db.execute("UPDATE bot_wallets SET balance=1000 WHERE user_id=3", [])
                .unwrap();
        }
        let (a, b) = tokio::join!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/market/action",
                json!({"request_id":"buyer","action":"buy","listing_id":listing}),
                Some(&buyer)
            ),
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/market/action",
                json!({"request_id":"other","action":"buy","listing_id":listing}),
                Some(&other)
            )
        );
        assert_eq!(
            usize::from(a.status() == StatusCode::OK) + usize::from(b.status() == StatusCode::OK),
            1
        );
        let db = app.db.lock().unwrap();
        assert_eq!(
            copies(&db, 2, &id).unwrap() + copies(&db, 3, &id).unwrap(),
            1
        );
        assert_eq!(
            wallet(&db, 1).unwrap().0 + wallet(&db, 2).unwrap().0 + wallet(&db, 3).unwrap().0,
            3000
        );
    }
}
