//! Server-selected instant games and owner controls. All writes run inside once().
use super::*;

#[path = "gambling_new_games.rs"]
mod new_games;
const INSTANT_GAMES: [&str; 7] = [
    "roulette", "dice", "slots", "keno", "plinko", "wheel", "baccarat",
];
const GAMES: [&str; 10] = [
    "crash",
    "blackjack",
    "roulette",
    "dice",
    "slots",
    "keno",
    "plinko",
    "wheel",
    "baccarat",
    "cases",
];

pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS gambling_house(id INTEGER PRIMARY KEY CHECK(id=1),paused INTEGER NOT NULL DEFAULT 0,daily_limit INTEGER NOT NULL DEFAULT 200 CHECK(daily_limit BETWEEN 1 AND 500)); INSERT OR IGNORE INTO gambling_house(id) VALUES(1);
        CREATE TABLE IF NOT EXISTS gambling_game_rules(game TEXT PRIMARY KEY,enabled INTEGER NOT NULL DEFAULT 1,min_stake INTEGER NOT NULL DEFAULT 1,max_stake INTEGER NOT NULL DEFAULT 1000000,payout_percent INTEGER NOT NULL DEFAULT 100);
        CREATE TABLE IF NOT EXISTS gambling_crate_rarity(case_id TEXT NOT NULL,rarity TEXT NOT NULL,factor INTEGER NOT NULL CHECK(factor BETWEEN 0 AND 1000),PRIMARY KEY(case_id,rarity)); CREATE TABLE IF NOT EXISTS gambling_crate_prices(case_id TEXT PRIMARY KEY,cost INTEGER NOT NULL CHECK(cost BETWEEN 1 AND 1000000));")?;
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
                min_stake: 1,
                max_stake: MAX_STAKE,
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
    if !(1..=MAX_STAKE).contains(&stake) {
        return Err(bad(
            "Choose a positive whole-Kash stake within your balance",
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
        .unwrap_or(default_case_cost(case_id)))
}

pub(super) const RARITIES: [&str; 5] = ["common", "uncommon", "rare", "epic", "legendary"];
pub(super) fn rarity_factors(
    db: &Connection,
    id: &str,
) -> ApiResult<std::collections::BTreeMap<String, i64>> {
    RARITIES
        .into_iter()
        .map(|r| {
            Ok((
                r.into(),
                db.query_row(
                    "SELECT factor FROM gambling_crate_rarity WHERE case_id=?1 AND rarity=?2",
                    params![id, r],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(100),
            ))
        })
        .collect()
}
pub(super) fn weighted_pool<'a>(
    db: &Connection,
    catalog: &'a Value,
    id: &str,
) -> ApiResult<(Vec<(&'a Value, u64)>, u64)> {
    let (pool, _) = case_pool(catalog, id)?;
    let factors = rarity_factors(db, id)?;
    let weighted: Vec<_> = pool
        .into_iter()
        .filter_map(|item| {
            let w = item["weight"].as_u64().unwrap()
                * factors
                    .get(item["rarity"].as_str().unwrap_or("common"))
                    .copied()
                    .unwrap_or(100) as u64;
            (w > 0).then_some((item, w))
        })
        .collect();
    let sum = weighted.iter().map(|(_, w)| w).sum();
    Ok((weighted, sum))
}

pub(super) fn default_case_cost(id: &str) -> i64 {
    match id {
        "username-effects" => 75,
        "cod-emblems" => 60,
        "bo2-animated" => 250,
        "mw2-canna" => 175,
        "premium-cosmetics" => 350,
        "rank-emblems" => 90,
        _ => 100,
    }
}

pub fn rules_view(db: &Connection) -> ApiResult<Value> {
    let all: bool = db.query_row("SELECT paused FROM gambling_house WHERE id=1", [], |r| {
        r.get(0)
    })?;
    let games = GAMES
        .into_iter()
        .map(|game| rule(db, game))
        .collect::<ApiResult<Vec<_>>>()?;
    let crates = CASE_IDS.iter().copied().chain(std::iter::once("canna-case"))

    .map(|id| Ok(json!({"case_id":id,"cost":case_cost(db,id)?,"rarity_factors":rarity_factors(db,id)?})))
    .collect::<ApiResult<Vec<_>>>()?;
    Ok(json!({"paused":all,"daily_limit":daily_limit(db)?,"games":games,"crates":crates}))
}

pub fn recent(db: &Connection, actor: i64) -> ApiResult<Value> {
    let results = db.prepare("SELECT response,created FROM gambling_requests WHERE user_id=?1 AND kind IN ('roulette','dice','slots','keno','plinko','wheel','baccarat') ORDER BY created DESC,rowid DESC LIMIT 20")?.query_map([actor], |r| Ok((r.get::<_,String>(0)?, r.get::<_,i64>(1)?)))?.collect::<Result<Vec<_>,_>>()?;
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
    let (games,staked,paid):(i64,i64,i64) = db.query_row("SELECT count(*),CAST(MIN(9007199254740991,COALESCE(total(json_extract(response,'$.stake')),0)) AS INTEGER),CAST(MIN(9007199254740991,COALESCE(total(json_extract(response,'$.payout')),0)) AS INTEGER) FROM gambling_requests WHERE kind IN ('roulette','dice','slots','keno','plinko','wheel','baccarat') AND created>=?1", [now()-86400], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rarity_factors: Option<std::collections::BTreeMap<String, i64>>,
}

pub async fn admin_rules(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<RulesInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = community::owner(&app, &headers)?;
    if !(1..=500).contains(&input.daily_limit)
        || input.games.len() != GAMES.len()
        || input.crates.len() > CASE_IDS.len() + 1
    {
        return Err(bad("Choose all game rules and a daily limit from 1 to 500"));
    }
    let mut seen = std::collections::HashSet::new();
    for r in &input.games {
        if !GAMES.contains(&r.game.as_str())
            || !seen.insert(&r.game)
            || r.min_stake < 1
            || r.min_stake > r.max_stake
            || r.max_stake > MAX_STAKE
            || !(25..=150).contains(&r.payout_percent)
            || (!INSTANT_GAMES.contains(&r.game.as_str()) && r.payout_percent != 100)
        {
            return Err(bad(
                "Invalid game rules: use unique games, positive whole-Kash stakes and arcade payout factors 25–150 percent",
            ));
        }
    }
    let mut crates = std::collections::HashSet::new();
    for c in &input.crates {
        case_definition(&c.case_id)?;
        if let Some(factors) = &c.rarity_factors {
            if factors.len() != 5
                || RARITIES.iter().any(|r| !factors.contains_key(*r))
                || factors.values().any(|n| !(0..=1000).contains(n))
                || factors.values().all(|n| *n == 0)
            {
                return Err(bad(
                    "Supply five rarity factors from 0 to 1000, with at least one enabled tier",
                ));
            }
            let (pool, _) = case_pool(catalog()?, &c.case_id)?;
            if !pool.is_empty()
                && pool.iter().all(|item| {
                    factors
                        .get(item["rarity"].as_str().unwrap_or("common"))
                        .copied()
                        .unwrap_or(100)
                        == 0
                })
            {
                return Err(bad(
                    "These rarity factors exclude every cosmetic in this crate",
                ));
            }
        }
        if !crates.insert(&c.case_id) || !(1..=MAX_CRATE_PRICE).contains(&c.cost) {
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
        tx.execute("UPDATE gambling_game_rules SET enabled=?1,min_stake=?2,max_stake=?3,payout_percent=?4 WHERE game=?5",params![r.enabled,1,MAX_STAKE,r.payout_percent,r.game])?;
    }
    for c in &input.crates {
        tx.execute("INSERT INTO gambling_crate_prices VALUES(?1,?2) ON CONFLICT(case_id) DO UPDATE SET cost=excluded.cost",params![c.case_id,c.cost])?;
    }
    for c in &input.crates {
        if let Some(factors) = &c.rarity_factors {
            for (rarity, factor) in factors {
                tx.execute("INSERT INTO gambling_crate_rarity VALUES(?1,?2,?3) ON CONFLICT(case_id,rarity) DO UPDATE SET factor=excluded.factor",params![c.case_id,rarity,factor])?;
            }
        }
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
    #[serde(default)]
    picks: Option<Vec<i64>>,
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
    if !matches!(
        input.game.as_str(),
        "keno" | "plinko" | "wheel" | "baccarat"
    ) && input.picks.is_some()
    {
        return Err(bad("This game does not accept number picks"));
    }
    if matches!(
        input.game.as_str(),
        "keno" | "plinko" | "wheel" | "baccarat"
    ) && (input.number.is_some() || input.under.is_some())
    {
        return Err(bad("Unexpected number or chance for this game"));
    }
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
                    input.stake as i128 * if choice == "number" { 36 } else { 2 }
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
                json!({"game":"dice","stake":input.stake,"payout":if win {whole_return(input.stake as i128*factor as i128/under as i128)} else {0},"payout_percent":factor,"result":{"roll":roll as f64/100.0,"under":under,"won":win},"nominal_multiplier":factor as f64/under as f64}),
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
                input.stake as i128 * payout as i128,
                json!({"reels":reels,"base_multiplier":payout,"won":payout>0}),
            )
        }
        "keno" | "plinko" | "wheel" | "baccarat" => {
            let (payout, payload) = new_games::result(input, factor, rng)?;
            return Ok(
                json!({"game":input.game,"stake":input.stake,"payout":payout,"payout_percent":factor,"result":payload}),
            );
        }
        _ => return Err(bad("Choose a supported arcade game")),
    };
    Ok(
        json!({"game":input.game,"stake":input.stake,"payout":whole_return(base*factor as i128/100),"payout_percent":factor,"result":payload}),
    )
}

pub async fn play(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<ArcadeInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = mutation_actor(&app, &headers)?;
    if !INSTANT_GAMES.contains(&input.game.as_str()) {
        return Err(bad("Choose a supported arcade game"));
    }
    once(
        &app,
        actor,
        &input.request_id,
        &input.game,
        &json!({"game":input.game,"stake":input.stake,"choice":input.choice,"number":input.number,"under":input.under,"picks":input.picks}),
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
    fn large_arcade_stakes_use_single_wide_rounding_and_ignore_legacy_caps() {
        let (_dir, app) = fixture();
        account(&app, "large-arcade", false);
        let db = app.db.lock().unwrap();
        wallet(&db, 1).unwrap();
        db.execute(
            "UPDATE bot_wallets SET balance=?1 WHERE user_id=1",
            [MAX_STAKE],
        )
        .unwrap();
        db.execute(
            "UPDATE gambling_game_rules SET min_stake=100,max_stake=200",
            [],
        )
        .unwrap();
        assert_eq!(new_game(&db, "slots", 3_000_000).unwrap(), 100);
        assert_eq!(rule(&db, "slots").unwrap().max_stake, MAX_STAKE);
        assert!(new_game(&db, "slots", 0).is_err());
        assert!(debit(&db, 1, MAX_STAKE + 1).is_err());
        debit(&db, 1, MAX_STAKE).unwrap();
        assert_eq!(wallet(&db, 1).unwrap().0, 0);
        assert!(debit(&db, 1, 1).is_err());
        use rand::{SeedableRng, rngs::StdRng};
        let mut rng = StdRng::seed_from_u64(123);
        for game in INSTANT_GAMES {
            let mut body = json!({"request_id":"wide","game":game,"stake":MAX_STAKE});
            match game {
                "roulette" => body["choice"] = json!("red"),
                "dice" => body["under"] = json!(2),
                "keno" => body["picks"] = json!([1, 2, 3, 4]),
                "plinko" => body["choice"] = json!("high"),
                "baccarat" => body["choice"] = json!("banker"),
                _ => {}
            }
            let input: ArcadeInput = serde_json::from_value(body).unwrap();
            for _ in 0..32 {
                let result = result(&input, 150, &mut rng).unwrap();
                assert!(
                    (0..=MAX_STAKE).contains(&result["payout"].as_i64().unwrap()),
                    "{game}"
                );
            }
        }
        assert_eq!(
            super::whole_return(MAX_STAKE as i128 * 100000 / 100),
            MAX_STAKE
        );
        assert_eq!(super::whole_return(25 * 195 * 100 / 10000), 48);
        // Large history totals must not overflow SQLite SUM or break the admin page.
        for i in 0..1100 {
            db.execute("INSERT INTO gambling_requests(user_id,request_id,fingerprint,response,created,kind) VALUES(1,?1,'large',?2,?3,'slots')",params![format!("large-{i}"),json!({"stake":MAX_STAKE,"payout":MAX_STAKE}).to_string(),now()]).unwrap();
        }
        assert_eq!(metrics(&db).unwrap()["arcade_staked_24h"], MAX_STAKE);
    }

    #[tokio::test]
    async fn new_arcade_games_replay_once_and_obey_limits_and_owner_rarity_controls() {
        let (_dir, app) = fixture();
        let owner = account(&app, "expanded-owner", true);
        let member = account(&app, "expanded-member", false);
        {
            let db = app.db.lock().unwrap();
            wallet(&db, 2).unwrap();
            db.execute("UPDATE bot_wallets SET balance=1000 WHERE user_id=2", [])
                .unwrap();
        }
        for game in ["keno", "plinko", "wheel", "baccarat"] {
            let mut input = json!({"request_id":format!("new-{game}"),"game":game,"stake":10});
            if game == "keno" {
                input["picks"] = json!([1, 2, 3, 4]);
            }
            if game == "plinko" {
                input["choice"] = json!("high");
            }
            if game == "baccarat" {
                input["choice"] = json!("banker");
            }
            let first = value(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/gambling/arcade/play",
                    input.clone(),
                    Some(&member),
                )
                .await,
            )
            .await;
            assert_eq!(first["ok"], true);
            assert_eq!(first["game"], game);
            assert_eq!(
                value(
                    call(
                        app.clone(),
                        "POST",
                        "/api/v1/gambling/arcade/play",
                        input.clone(),
                        Some(&member)
                    )
                    .await
                )
                .await,
                first
            );
            input["stake"] = json!(11);
            assert_eq!(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/gambling/arcade/play",
                    input,
                    Some(&member)
                )
                .await
                .status(),
                StatusCode::CONFLICT
            );
        }
        let mut rules = rules_view(&app.db.lock().unwrap()).unwrap();
        assert_eq!(rules["games"].as_array().unwrap().len(), 10);
        let crates = rules["crates"].as_array_mut().unwrap();
        let crate_rule = crates
            .iter_mut()
            .find(|c| c["case_id"] == "username-effects")
            .unwrap();
        crate_rule["cost"] = json!(75);
        crate_rule["rarity_factors"] =
            json!({"common":0,"uncommon":0,"rare":0,"epic":0,"legendary":100});
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
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/gambling/rules",
                rules.clone(),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::OK
        );
        let opened = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/cases/open",
                json!({"request_id":"legendary-case","case_id":"username-effects"}),
                Some(&member),
            )
            .await,
        )
        .await;
        assert_eq!(opened["item"]["rarity"], "legendary");
        assert_eq!(opened["cost"], 75);
        rules["daily_limit"] = json!(1);
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/gambling/rules",
                rules,
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
                "/api/v1/gambling/arcade/play",
                json!({"request_id":"over-limit","game":"wheel","stake":1}),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }

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
