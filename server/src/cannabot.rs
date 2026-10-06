use super::*;
use rand::Rng;
pub const MAX_KASH: i64 = 9_007_199_254_740_991;
const WALLET_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS bot_wallets(user_id INTEGER PRIMARY KEY REFERENCES users(id),balance INTEGER NOT NULL DEFAULT 0 CHECK(balance BETWEEN 0 AND 9007199254740991),earned INTEGER NOT NULL DEFAULT 0,daily INTEGER NOT NULL DEFAULT -1,last_fish INTEGER NOT NULL DEFAULT 0,last_flip INTEGER NOT NULL DEFAULT 0,fish_day INTEGER NOT NULL DEFAULT -1,fish_count INTEGER NOT NULL DEFAULT 0,badge TEXT NOT NULL DEFAULT 'none');";
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(WALLET_SCHEMA)?;
    let schema: String = db.query_row(
        "SELECT sql FROM sqlite_master WHERE name='bot_wallets'",
        [],
        |r| r.get(0),
    )?;
    if schema.contains("BETWEEN 0 AND 10000)") {
        let tx = db.unchecked_transaction()?;
        tx.execute_batch("ALTER TABLE bot_wallets RENAME TO bot_wallets_legacy;")?;
        tx.execute_batch(WALLET_SCHEMA)?;
        tx.execute_batch("INSERT INTO bot_wallets SELECT * FROM bot_wallets_legacy; DROP TABLE bot_wallets_legacy;")?;
        tx.commit()?;
    }
    db.execute_batch("CREATE TABLE IF NOT EXISTS bot_catches(user_id INTEGER NOT NULL REFERENCES users(id),species TEXT NOT NULL,count INTEGER NOT NULL,PRIMARY KEY(user_id,species));")
}
// Each flip draws independently from the OS CSPRNG; no history, account or stake weighting.
fn coin_side() -> &'static str {
    if OsRng.gen_bool(0.5) {
        "heads"
    } else {
        "tails"
    }
}
// Commands execute inside the same transaction as the chat message.
pub fn run(db: &Connection, actor: i64, body: &str) -> ApiResult<Option<String>> {
    if !body.starts_with('/') {
        return Ok(None);
    }
    let args: Vec<&str> = body.split_whitespace().collect();
    let command = args.first().copied().unwrap_or("");
    if body.len() > 100 {
        return Err(bad("CannaBot commands must be under 100 bytes"));
    }
    if command == "/help" {
        return Ok(Some("CannaBot · /fish — fish once per minute (50 catches/day); /daily — 100 free Kash each UTC day; /balance; /collection; /coinflip heads|tails amount — wager Kash up to your balance (also /flip); /badges; /equip none|angler|emerald|legend. Kash cannot be bought, redeemed or transferred, and unlock chat badges only. Each coin flip is independently random with 50/50 odds. The wager game paying 2× your stake when you win, up to 20 flips/day.".into()));
    }
    db.execute(
        "INSERT OR IGNORE INTO bot_wallets(user_id) VALUES(?1)",
        [actor],
    )?;
    let (balance,earned,daily,last_fish,last_flip,fish_day,fish_count):(i64,i64,i64,i64,i64,i64,i64)=db.query_row("SELECT balance,earned,daily,last_fish,last_flip,fish_day,fish_count FROM bot_wallets WHERE user_id=?1",[actor],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)))?;
    let day = now() / 86400;
    let answer = match command {
        "/balance" if args.len() == 1 => format!(
            "You have {balance} Kash · {earned} total earned. /daily and /fish earn Kash; /badges shows cosmetic unlocks."
        ),
        "/daily" if args.len() == 1 => {
            if daily == day {
                return Ok(Some(
                    "Daily Kash already claimed. Come back after 00:00 UTC.".into(),
                ));
            }
            let reward = 100.min(MAX_KASH - balance);
            db.execute("UPDATE bot_wallets SET balance=balance+?1,earned=MIN(1000000,earned+?1),daily=?2 WHERE user_id=?3",params![reward,day,actor])?;
            db.execute(
                "UPDATE notifications SET read=1 WHERE user_id=?1 AND dedup=?2",
                params![actor, format!("daily:{day}")],
            )?;
            format!(
                "Daily reward: {reward} Kash. Balance: {}.",
                balance + reward
            )
        }
        "/fish" if args.len() == 1 => {
            if now() - last_fish < 60 {
                return Ok(Some(format!(
                    "Your fishing rod is resting. Try again in {} seconds.",
                    60 - (now() - last_fish)
                )));
            }
            if fish_day == day && fish_count >= 50 {
                return Ok(Some(
                    "Today's fishing limit is 50 catches. Come back tomorrow.".into(),
                ));
            }
            let roll = OsRng.gen_range(0..100);
            let (species, reward) = match roll {
                0..=49 => ("pond perch", 5),
                50..=79 => ("river trout", 10),
                80..=94 => ("emerald bass", 20),
                95..=98 => ("moon koi", 40),
                _ => ("legendary Canna carp", 75),
            };
            let reward = reward.min(MAX_KASH - balance);
            db.execute("UPDATE bot_wallets SET balance=balance+?1,earned=MIN(1000000,earned+?1),last_fish=?2,fish_day=?3,fish_count=CASE WHEN fish_day=?3 THEN fish_count+1 ELSE 1 END WHERE user_id=?4",params![reward,now(),day,actor])?;
            db.execute("INSERT INTO bot_catches VALUES(?1,?2,1) ON CONFLICT(user_id,species) DO UPDATE SET count=MIN(1000000,count+1)",params![actor,species])?;
            format!(
                "You caught a {species}! +{reward} Kash · Balance {}. Your collection keeps the catch.",
                balance + reward
            )
        }
        "/collection" if args.len() == 1 => {
            let mut stmt = db.prepare(
                "SELECT species,count FROM bot_catches WHERE user_id=?1 ORDER BY species",
            )?;
            let rows = stmt
                .query_map([actor], |r| {
                    Ok(format!(
                        "{} ×{}",
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            if rows.is_empty() {
                "No catches yet. Try /fish.".into()
            } else {
                format!("Your catches: {}", rows.join(" · "))
            }
        }
        "/badges" if args.len() == 1 => format!(
            "Leaf skins (total earned Kash): Mint Stripes 200{} (/equip angler) · Emerald Veins 500{} (/equip emerald) · Amethyst Spots 1500{} (/equip legend). /equip none removes your skin.",
            if earned >= 200 { " — unlocked" } else { "" },
            if earned >= 500 { " — unlocked" } else { "" },
            if earned >= 1500 { " — unlocked" } else { "" }
        ),
        "/equip" if args.len() == 2 => {
            let needed = match args[1] {
                "none" => 0,
                "angler" => 200,
                "emerald" => 500,
                "legend" => 1500,
                _ => return Err(bad("Choose none, angler, emerald or legend")),
            };
            if earned < needed {
                return Err(bad(
                    "That badge is still locked; earn more Kash with /daily or /fish",
                ));
            }
            db.execute(
                "UPDATE bot_wallets SET badge=?1 WHERE user_id=?2",
                params![args[1], actor],
            )?;
            format!("Equipped {} chat badge.", args[1])
        }
        "/coinflip" | "/flip" if args.len() != 3 => {
            "Use /coinflip heads amount or /coinflip tails amount (positive whole Kash amount)."
                .into()
        }
        "/flip" | "/coinflip" if args.len() == 3 => {
            if !matches!(args[1], "heads" | "tails") {
                return Err(bad("Use /coinflip heads 10 or /coinflip tails 10"));
            }
            let stake: i64 = args[2]
                .parse()
                .map_err(|_| bad("Use a positive whole-number Kash amount"))?;
            if stake < 1 || stake > balance {
                return Err(bad("Bet a positive amount up to your Kash balance"));
            }
            if balance
                .checked_add(stake)
                .is_none_or(|payout| payout > MAX_KASH)
            {
                return Err(bad(
                    "That payout exceeds supported integer precision; choose a smaller amount",
                ));
            }
            if now() - last_flip < 5 {
                return Err(ApiError(
                    StatusCode::TOO_MANY_REQUESTS,
                    "Wait five seconds between flips",
                ));
            }
            // Persisted daily allowance survives restarts and is separate from other commands.
            db.execute("INSERT INTO bot_flip_limits VALUES(?1,?2,0) ON CONFLICT(user_id) DO UPDATE SET count=CASE WHEN day=excluded.day THEN count ELSE 0 END,day=excluded.day",params![actor,day])?;
            let count: i64 = db.query_row(
                "SELECT count FROM bot_flip_limits WHERE user_id=?1",
                [actor],
                |r| r.get(0),
            )?;
            if count >= 20 {
                return Err(ApiError(
                    StatusCode::TOO_MANY_REQUESTS,
                    "Today's limit is 20 flips",
                ));
            }
            let side = coin_side();
            let change = if side == args[1] { stake } else { -stake };
            db.execute(
                "UPDATE bot_wallets SET balance=balance+?1,earned=MIN(1000000,earned+MAX(0,?1)),last_flip=?2 WHERE user_id=?3",
                params![change, now(), actor],
            )?;
            db.execute(
                "UPDATE bot_flip_limits SET count=count+1 WHERE user_id=?1",
                [actor],
            )?;
            db.execute("INSERT INTO audit(actor,action,target,created) VALUES(?1,'coinflip',?2,?3)",params![actor,json!({"choice":args[1],"outcome":side,"wager":stake,"balance_before":balance,"balance_after":balance+change,"random_source":"OS CSPRNG"}).to_string(),now()])?;
            format!(
                "The coin landed {side}. You {} {stake} Kash. Balance: {}. (50/50 odds)",
                if change > 0 { "won" } else { "lost" },
                balance + change
            )
        }
        _ => "Unknown command or extra arguments. Type /help for CannaBot commands.".into(),
    };
    Ok(Some(answer))
}
#[cfg(test)]
mod tests {
    #[test]
    fn old_wallets_keep_balances_and_large_all_in_bets_work() {
        let (_dir, app) = fixture();
        account(&app, "bettor", false);
        let db = app.db.lock().unwrap();
        db.execute_batch("DROP TABLE bot_wallets; CREATE TABLE bot_wallets(user_id INTEGER PRIMARY KEY REFERENCES users(id),balance INTEGER NOT NULL DEFAULT 0 CHECK(balance BETWEEN 0 AND 10000),earned INTEGER NOT NULL DEFAULT 0,daily INTEGER NOT NULL DEFAULT -1,last_fish INTEGER NOT NULL DEFAULT 0,last_flip INTEGER NOT NULL DEFAULT 0,fish_day INTEGER NOT NULL DEFAULT -1,fish_count INTEGER NOT NULL DEFAULT 0,badge TEXT NOT NULL DEFAULT 'none');INSERT INTO bot_wallets(user_id,balance,earned,badge) VALUES(1,5000,1000,'emerald');").unwrap();
        initialize(&db).unwrap();
        initialize(&db).unwrap();
        assert_eq!(
            db.query_row("SELECT balance FROM bot_wallets WHERE user_id=1", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            5000
        );
        assert_eq!(
            db.query_row("SELECT badge FROM bot_wallets WHERE user_id=1", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "emerald"
        );
        db.execute("UPDATE bot_wallets SET balance=50000 WHERE user_id=1", [])
            .unwrap();
        assert!(run(&db, 1, "/coinflip heads 50001").is_err());
        let reply = run(&db, 1, "/coinflip heads 50000").unwrap().unwrap();
        let balance = db
            .query_row("SELECT balance FROM bot_wallets WHERE user_id=1", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(
            balance,
            if reply.contains("landed heads.") {
                100000
            } else {
                0
            }
        );
        db.execute(
            "UPDATE bot_wallets SET balance=50000,daily=-1 WHERE user_id=1",
            [],
        )
        .unwrap();
        run(&db, 1, "/daily").unwrap();
        assert_eq!(
            db.query_row("SELECT balance FROM bot_wallets WHERE user_id=1", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            50100
        );
    }
    use super::*;
    use crate::tests::{account, call, fixture, value};
    #[test]
    fn coinflip_alias_and_plain_flip_share_fair_result_and_wager_limits() {
        let (_dir, app) = fixture();
        account(&app, "coin-player", false);
        let db = app.db.lock().unwrap();
        run(&db, 1, "/daily").unwrap();
        for _ in 0..20 {
            let answer = run(&db, 1, "/coinflip").unwrap().unwrap();
            assert!(answer.contains("Use /coinflip heads amount"));
        }
        let balance: i64 = db
            .query_row("SELECT balance FROM bot_wallets WHERE user_id=1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(balance, 100);
        let answer = run(&db, 1, "/coinflip heads 10").unwrap().unwrap();
        let balance: i64 = db
            .query_row("SELECT balance FROM bot_wallets WHERE user_id=1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            balance,
            if answer.contains("landed heads.") {
                110
            } else {
                90
            }
        );
        assert!(run(&db, 1, "/flip heads 10").is_err());
        assert!(run(&db, 1, "/coinflip heads 10").is_err());
        assert!(run(&db, 1, "/coinflip heads -1").is_err());
    }
    #[test]
    fn coin_draws_can_repeat_and_audit_matches_payout() {
        // A guard against a deterministic alternating implementation, not a proof of randomness.
        let draws: Vec<_> = (0..4096).map(|_| coin_side()).collect();
        for pair in [
            ["heads", "heads"],
            ["tails", "tails"],
            ["heads", "tails"],
            ["tails", "heads"],
        ] {
            assert!(draws.windows(2).any(|w| w == pair));
        }
        let heads = draws.iter().filter(|s| **s == "heads").count();
        println!(
            "Independent draws: heads={heads}, tails={}, repeated_neighbors={}",
            draws.len() - heads,
            draws.windows(2).filter(|w| w[0] == w[1]).count()
        );
        let (_dir, app) = fixture();
        account(&app, "auditplayer", false);
        let db = app.db.lock().unwrap();
        db.execute("INSERT INTO bot_wallets(user_id,balance) VALUES(1,100)", [])
            .unwrap();
        let answer = run(&db, 1, "/coinflip heads 10").unwrap().unwrap();
        let target: String = db
            .query_row(
                "SELECT target FROM audit WHERE action='coinflip'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let event: Value = serde_json::from_str(&target).unwrap();
        assert_eq!(event["choice"], "heads");
        assert_eq!(event["wager"], 10);
        assert!(answer.contains(&format!("landed {}.", event["outcome"].as_str().unwrap())));
        let balance: i64 = db
            .query_row("SELECT balance FROM bot_wallets WHERE user_id=1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(event["balance_after"], balance);
    }
    #[tokio::test]
    async fn rewards_cooldowns_and_badges_are_server_enforced() {
        let (_dir, app) = fixture();
        let member = account(&app, "angler", false);
        let send = |body: &str| json!({"body":body});
        assert!(
            call(
                app.clone(),
                "POST",
                "/api/v1/chat",
                send("/daily"),
                Some(&member)
            )
            .await
            .status()
            .is_success()
        );
        assert!(
            call(
                app.clone(),
                "POST",
                "/api/v1/chat",
                send("/daily"),
                Some(&member)
            )
            .await
            .status()
            .is_success()
        );
        assert!(
            call(
                app.clone(),
                "POST",
                "/api/v1/chat",
                send("/fish"),
                Some(&member)
            )
            .await
            .status()
            .is_success()
        );
        assert!(
            call(
                app.clone(),
                "POST",
                "/api/v1/chat",
                send("/fish"),
                Some(&member)
            )
            .await
            .status()
            .is_success()
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/chat",
                send("/equip legend"),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        let balance: i64 = app
            .db
            .lock()
            .unwrap()
            .query_row("SELECT balance FROM bot_wallets", [], |r| r.get(0))
            .unwrap();
        assert!((105..=175).contains(&balance));
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/chat",
                send("/flip heads -100"),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert!(
            call(
                app.clone(),
                "POST",
                "/api/v1/chat",
                send("/flip heads 25"),
                Some(&member)
            )
            .await
            .status()
            .is_success()
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/chat",
                send("/flip heads 25"),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        let rows = value(call(app, "GET", "/api/v1/chat", Value::Null, Some(&member)).await).await;
        assert!(rows.as_array().unwrap().iter().any(|r| r["bot"] == true));
    }
}
