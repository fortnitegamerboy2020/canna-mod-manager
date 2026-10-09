//! Server-selected instant games and owner controls. All writes run inside once().
use super::*;

const GAMES: [&str; 6] = ["crash", "blackjack", "roulette", "dice", "slots", "cases"];

pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS gambling_house(id INTEGER PRIMARY KEY CHECK(id=1),paused INTEGER NOT NULL DEFAULT 0,daily_limit INTEGER NOT NULL DEFAULT 200 CHECK(daily_limit BETWEEN 1 AND 500)); INSERT OR IGNORE INTO gambling_house(id) VALUES(1);
        CREATE TABLE IF NOT EXISTS gambling_game_rules(game TEXT PRIMARY KEY,enabled INTEGER NOT NULL DEFAULT 1,min_stake INTEGER NOT NULL DEFAULT 1,max_stake INTEGER NOT NULL DEFAULT 1000000,payout_percent INTEGER NOT NULL DEFAULT 100);
        CREATE TABLE IF NOT EXISTS gambling_crate_prices(case_id TEXT PRIMARY KEY,cost INTEGER NOT NULL CHECK(cost BETWEEN 1 AND 1000000));")?;
    for game in GAMES {
        db.execute(
            "INSERT OR IGNORE INTO gambling_game_rules(game,payout_percent) VALUES(?1,?2)",
            params![game, if game == "dice" { 95 } else { 100 }],
        )?;
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameRule {
    game: String,
    enabled: bool,
    min_stake: i64,
    max_stake: i64,
    payout_percent: i64,
}

fn rule(db: &Connection, game: &str) -> ApiResult<GameRule> {
    Ok(db.query_row(
        "SELECT enabled,min_stake,max_stake,payout_percent FROM gambling_game_rules WHERE game=?1",
        [game],
        |r| {
            Ok(GameRule {
                game: game.into(),
                enabled: r.get(0)?,
                min_stake: r.get(1)?,
                max_stake: r.get(2)?,
                payout_percent: r.get(3)?,
            })
        },
    )?)
}

pub fn daily_limit(db: &Connection) -> ApiResult<i64> {
    Ok(db.query_row(
        "SELECT daily_limit FROM gambling_house WHERE id=1",
        [],
        |r| r.get(0),
    )?)
}

pub fn paused(db: &Connection, game: &str) -> ApiResult<bool> {
    let all: bool = db.query_row("SELECT paused FROM gambling_house WHERE id=1", [], |r| {
        r.get(0)
    })?;
    Ok(all || !rule(db, game)?.enabled)
}

pub fn new_game(db: &Connection, game: &str, stake: i64) -> ApiResult<i64> {
    let rule = rule(db, game)?;
    if paused(db, game)? {
        return Err(bad(
            "New wagers for this game are paused; existing hands and cashouts can finish",
        ));
    }
    if !(rule.min_stake..=rule.max_stake).contains(&stake) {
        return Err(bad(
            "Stake is outside this game's current limits; refresh the game rules",
        ));
    }
    Ok(rule.payout_percent)
}

pub fn case_cost(db: &Connection, case_id: &str) -> ApiResult<i64> {
    case_definition(case_id)?;
    Ok(db
        .query_row(
            "SELECT cost FROM gambling_crate_prices WHERE case_id=?1",
            [case_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(catalog()?["case"]["price"].as_i64().unwrap()))
}

pub fn rules_view(db: &Connection) -> ApiResult<Value> {
    let all: bool = db.query_row("SELECT paused FROM gambling_house WHERE id=1", [], |r| {
        r.get(0)
    })?;
    let games = GAMES
        .into_iter()
        .map(|game| rule(db, game))
        .collect::<ApiResult<Vec<_>>>()?;
    let crates = [
        "bo2-calling-cards",
        "mw2-calling-cards",
        "avatar-frames",
        "cod-emblems",
        "username-effects",
        "canna-case",
    ]
    .into_iter()
    .map(|id| Ok(json!({"case_id":id,"cost":case_cost(db,id)?})))
    .collect::<ApiResult<Vec<_>>>()?;
    Ok(json!({"paused":all,"daily_limit":daily_limit(db)?,"games":games,"crates":crates}))
}

pub fn recent(db: &Connection, actor: i64) -> ApiResult<Value> {
    let results = db.prepare("SELECT response,created FROM gambling_requests WHERE user_id=?1 AND kind IN ('roulette','dice','slots') ORDER BY created DESC,rowid DESC LIMIT 20")?.query_map([actor], |r| Ok((r.get::<_,String>(0)?, r.get::<_,i64>(1)?)))?.collect::<Result<Vec<_>,_>>()?;
    Ok(json!(
        results
            .into_iter()
            .filter_map(
                |(text, created)| serde_json::from_str::<Value>(&text).ok().map(|mut v| {
                    v["created"] = json!(created);
                    v
                })
            )
            .collect::<Vec<_>>()
    ))
}

pub fn metrics(db: &Connection) -> ApiResult<Value> {
    let (games,staked,paid):(i64,i64,i64) = db.query_row("SELECT count(*),COALESCE(sum(json_extract(response,'$.stake')),0),COALESCE(sum(json_extract(response,'$.payout')),0) FROM gambling_requests WHERE kind IN ('roulette','dice','slots') AND created>=?1", [now()-86400], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    let hands: i64 = db.query_row(
        "SELECT count(*) FROM gambling_blackjack WHERE status='playing'",
        [],
        |r| r.get(0),
    )?;
    let bets: i64 = db.query_row(
        "SELECT count(*) FROM gambling_crash_bets WHERE status='pending'",
        [],
        |r| r.get(0),
    )?;
    Ok(
        json!({"arcade_games_24h":games,"arcade_staked_24h":staked,"arcade_paid_24h":paid,"active_blackjack_hands":hands,"pending_crash_bets":bets}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulesInput {
    paused: bool,
    daily_limit: i64,
    games: Vec<GameRule>,
    #[serde(default)]
    crates: Vec<CratePrice>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CratePrice {
    case_id: String,
    cost: i64,
}

pub async fn admin_rules(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<RulesInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = community::owner(&app, &headers)?;
    if !(1..=500).contains(&input.daily_limit)
        || input.games.len() != GAMES.len()
        || input.crates.len() > 6
    {
        return Err(bad(
            "Choose all six game rules and a daily limit from 1 to 500",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for r in &input.games {
        if !GAMES.contains(&r.game.as_str())
            || !seen.insert(&r.game)
            || r.min_stake < 1
            || r.min_stake > r.max_stake
            || r.max_stake > MAX_STAKE
            || !(25..=150).contains(&r.payout_percent)
            || (!matches!(r.game.as_str(), "roulette" | "dice" | "slots")
                && r.payout_percent != 100)
        {
            return Err(bad(
                "Invalid game rules: use unique games, stakes 1–1000000 and arcade payout factors 25–150 percent",
            ));
        }
    }
    let mut crates = std::collections::HashSet::new();
    for c in &input.crates {
        case_definition(&c.case_id)?;
        if !crates.insert(&c.case_id) || !(1..=MAX_STAKE).contains(&c.cost) {
            return Err(bad(
                "Crate prices must be unique and from 1 to 1000000 Kash",
            ));
        }
    }
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let role: String = tx.query_row("SELECT role FROM users WHERE id=?1", [actor], |r| r.get(0))?;
    if role != "owner" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Owner access required"));
    }
    tx.execute(
        "UPDATE gambling_house SET paused=?1,daily_limit=?2 WHERE id=1",
        params![input.paused, input.daily_limit],
    )?;
    for r in &input.games {
        tx.execute("UPDATE gambling_game_rules SET enabled=?1,min_stake=?2,max_stake=?3,payout_percent=?4 WHERE game=?5",params![r.enabled,r.min_stake,r.max_stake,r.payout_percent,r.game])?;
    }
    for c in &input.crates {
        tx.execute("INSERT INTO gambling_crate_prices VALUES(?1,?2) ON CONFLICT(case_id) DO UPDATE SET cost=excluded.cost",params![c.case_id,c.cost])?;
    }
    let rules = rules_view(&tx)?;
    record(&tx, actor, "gambling_rules", &rules)?;
    tx.commit()?;
    Ok(axum::Json(json!({"ok":true,"rules":rules})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArcadeInput {
    request_id: String,
    game: String,
    stake: i64,
    #[serde(default)]
    choice: Option<String>,
    #[serde(default)]
    number: Option<i64>,
    #[serde(default)]
    under: Option<i64>,
}

const RED: [i64; 18] = [
    1, 3, 5, 7, 9, 12, 14, 16, 18, 19, 21, 23, 25, 27, 30, 32, 34, 36,
];
fn roulette_win(number: i64, choice: &str, pick: Option<i64>) -> bool {
    match choice {
        "number" => Some(number) == pick,
        "red" => RED.contains(&number),
        "black" => number != 0 && !RED.contains(&number),
        "even" => number != 0 && number % 2 == 0,
        "odd" => number % 2 == 1,
        "low" => (1..=18).contains(&number),
        "high" => (19..=36).contains(&number),
        _ => false,
    }
}
fn slot_factor(reels: [usize; 3]) -> i64 {
    if reels[0] == reels[1] && reels[1] == reels[2] {
        [50, 25, 15, 10, 8, 5][reels[0]]
    } else if reels[0] == reels[1] || reels[0] == reels[2] || reels[1] == reels[2] {
        1
    } else {
        0
    }
}
fn result(input: &ArcadeInput, factor: i64, rng: &mut impl Rng) -> ApiResult<Value> {
    let (base, payload) = match input.game.as_str() {
        "roulette" => {
            let choice = input.choice.as_deref().unwrap_or("");
            if !matches!(
                choice,
                "number" | "red" | "black" | "even" | "odd" | "low" | "high"
            ) || (choice == "number" && !input.number.is_some_and(|n| (0..=36).contains(&n)))
                || (choice != "number" && input.number.is_some())
                || input.under.is_some()
            {
                return Err(bad(
                    "Choose a roulette color, range, parity or a number from 0 to 36",
                ));
            }
            let n = rng.gen_range(0..37);
            let win = roulette_win(n, choice, input.number);
            (
                if win {
                    input.stake * if choice == "number" { 36 } else { 2 }
                } else {
                    0
                },
                json!({"number":n,"color":if n==0 {"green"} else if RED.contains(&n) {"red"} else {"black"},"choice":choice,"pick":input.number,"won":win}),
            )
        }
        "dice" => {
            let under = input
                .under
                .filter(|n| (2..=95).contains(n))
                .ok_or_else(|| bad("Choose a Dice roll-under chance from 2 to 95 percent"))?;
            if input.choice.is_some() || input.number.is_some() {
                return Err(bad("Dice accepts only a roll-under chance"));
            }
            let roll = rng.gen_range(0..10000);
            let win = roll < under * 100;
            // Apply the payout factor before integer division to avoid double rounding.
            return Ok(
                json!({"game":"dice","stake":input.stake,"payout":if win {input.stake*factor/under} else {0},"payout_percent":factor,"result":{"roll":roll as f64/100.0,"under":under,"won":win},"nominal_multiplier":factor as f64/under as f64}),
            );
        }
        "slots" => {
            if input.choice.is_some() || input.number.is_some() || input.under.is_some() {
                return Err(bad("Slots accepts only a stake"));
            }
            let reels = [
                rng.gen_range(0..6),
                rng.gen_range(0..6),
                rng.gen_range(0..6),
            ];
            let payout = slot_factor(reels);
            (
                input.stake * payout,
                json!({"reels":reels,"base_multiplier":payout,"won":payout>0}),
            )
        }
        _ => return Err(bad("Choose roulette, dice or slots")),
    };
    Ok(
        json!({"game":input.game,"stake":input.stake,"payout":base*factor/100,"payout_percent":factor,"result":payload}),
    )
}

pub async fn play(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<ArcadeInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = mutation_actor(&app, &headers)?;
    if !matches!(input.game.as_str(), "roulette" | "dice" | "slots") {
        return Err(bad("Choose roulette, dice or slots"));
    }
    once(
        &app,
        actor,
        &input.request_id,
        &input.game,
        &json!({"game":input.game,"stake":input.stake,"choice":input.choice,"number":input.number,"under":input.under}),
        |db| {
            let factor = new_game(db, &input.game, input.stake)?;
            let mut outcome = result(&input, factor, &mut OsRng)?;
            debit(db, actor, input.stake)?;
            let paid = credit(db, actor, outcome["payout"].as_i64().unwrap(), input.stake)?;
            outcome["payout"] = json!(paid);
            Ok(outcome)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};

    #[test]
    fn complete_wheel_and_slot_outcomes_match_published_odds() {
        for choice in ["red", "black", "even", "odd", "low", "high"] {
            assert!(!roulette_win(0, choice, None));
            assert_eq!(
                (0..37).filter(|n| roulette_win(*n, choice, None)).count(),
                18
            );
        }
        for pick in 0..37 {
            assert_eq!(
                (0..37)
                    .filter(|n| roulette_win(*n, "number", Some(pick)))
                    .count(),
                1
            );
        }
        let mut sum = 0;
        for a in 0..6 {
            for b in 0..6 {
                for c in 0..6 {
                    sum += slot_factor([a, b, c]);
                }
            }
        }
        assert_eq!(sum, 203); // all 216 equiprobable outcomes, including returned stakes
    }

    #[tokio::test]
    async fn arcade_replays_rules_daily_limit_and_private_history() {
        let (_dir, app) = fixture();
        let owner = account(&app, "arcade-owner", true);
        let member = account(&app, "arcade-member", false);
        {
            let db = app.db.lock().unwrap();
            wallet(&db, 2).unwrap();
            db.execute("UPDATE bot_wallets SET balance=1000 WHERE user_id=2", [])
                .unwrap();
        }
        let body = json!({"request_id":"arcade-retry","game":"dice","stake":10,"under":50});
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/arcade/play",
                body.clone(),
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        let first = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/arcade/play",
                body.clone(),
                Some(&member),
            )
            .await,
        )
        .await;
        assert_eq!(first["payout_percent"], 95);
        assert!(first["result"]["roll"].as_f64().is_some());
        {
            let db = app.db.lock().unwrap();
            db.execute("UPDATE gambling_house SET paused=1,daily_limit=1", [])
                .unwrap();
        }
        assert_eq!(
            value(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/gambling/arcade/play",
                    body.clone(),
                    Some(&member)
                )
                .await
            )
            .await,
            first
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/arcade/play",
                json!({"request_id":"next","game":"slots","stake":10}),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        let rules = {
            let db = app.db.lock().unwrap();
            rules_view(&db).unwrap()
        };
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/gambling/rules",
                rules.clone(),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let mut bad_rules = rules.clone();
        bad_rules["games"][0]["game"] = json!("slots");
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/gambling/rules",
                bad_rules,
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        let mut updated = rules;
        updated["paused"] = json!(false);
        updated["daily_limit"] = json!(200);
        updated["games"][2]["payout_percent"] = json!(150);
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/gambling/rules",
                updated,
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            value(
                call(
                    app.clone(),
                    "GET",
                    "/api/v1/gambling",
                    Value::Null,
                    Some(&owner)
                )
                .await
            )
            .await["recent_games"],
            json!([])
        );
        let view = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/gambling",
                Value::Null,
                Some(&member),
            )
            .await,
        )
        .await;
        assert_eq!(view["recent_games"].as_array().unwrap().len(), 1);
        assert_eq!(view["recent_games"][0]["stake"], 10);
        let metrics = {
            let db = app.db.lock().unwrap();
            metrics(&db).unwrap()
        };
        assert_eq!(metrics["arcade_games_24h"], 1);
    }

    #[tokio::test]
    async fn invalid_arcade_wagers_do_not_charge_and_global_pause_preserves_existing_hand() {
        let (_dir, app) = fixture();
        let member = account(&app, "arcade-invalid", false);
        {
            let db = app.db.lock().unwrap();
            wallet(&db, 1).unwrap();
            db.execute("UPDATE bot_wallets SET balance=1000 WHERE user_id=1", [])
                .unwrap();
        }
        for (i, body) in [
            json!({"game":"dice","under":1,"stake":10}),
            json!({"game":"roulette","choice":"number","number":37,"stake":10}),
            json!({"game":"slots","choice":"rig","stake":10}),
            json!({"game":"slots","stake":1000001}),
        ]
        .into_iter()
        .enumerate()
        {
            let mut body = body;
            body["request_id"] = json!(format!("invalid-{i}"));
            assert_eq!(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/gambling/arcade/play",
                    body,
                    Some(&member)
                )
                .await
                .status(),
                StatusCode::BAD_REQUEST
            );
        }
        let hand_id = {
            let db = app.db.lock().unwrap();
            assert_eq!(wallet(&db, 1).unwrap().0, 1000);
            super::super::tests::hand(&db, 1, &[9, 6], &[9, 7], &[5])
        };
        {
            let db = app.db.lock().unwrap();
            db.execute("UPDATE gambling_house SET paused=1", [])
                .unwrap();
        }
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/blackjack/deal",
                json!({"request_id":"paused-deal","stake":10}),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/blackjack/action",
                json!({"request_id":"finish","hand_id":hand_id,"action":"stand"}),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::OK
        );
    }
}
