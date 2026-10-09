//! Fictional, non-purchasable Kash games. Decisions and wallet changes are persisted
//! together; clients display results but never deal cards or choose payouts.
use super::*;
use rand::{Rng, seq::SliceRandom};
use rusqlite::TransactionBehavior;
use serde::{Deserialize, Serialize};

#[path = "gambling_arcade.rs"]
mod arcade;
pub use arcade::{admin_rules, play};
#[path = "gambling_collection.rs"]
mod collection;
pub use collection::manage;

const CASE_IDS: [&str; 9] = [
    "bo2-calling-cards",
    "mw2-calling-cards",
    "avatar-frames",
    "cod-emblems",
    "username-effects",
    "bo2-animated",
    "mw2-canna",
    "premium-cosmetics",
    "rank-emblems",
];
const MAX_STAKE: i64 = cannabot::MAX_KASH;
const MAX_CRATE_PRICE: i64 = 1_000_000;
const MAX_MULTIPLIER: i64 = 100_000; // hundredths: 1000.00x
const BETTING_MS: i64 = 10_000;
const INTERMISSION_MS: i64 = 4_000;
const PARTICIPANT_PAGE_SIZE: i64 = 200;
const NOTICE: &str = "Kash is community play currency.";

#[cfg(test)]
#[path = "gambling_cosmetics_tests.rs"]
mod cosmetic_tests;

pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS gambling_config(id INTEGER PRIMARY KEY CHECK(id=1),mode TEXT NOT NULL DEFAULT 'random' CHECK(mode IN ('random','controlled')),paused INTEGER NOT NULL DEFAULT 0);
         INSERT OR IGNORE INTO gambling_config(id) VALUES(1);
         CREATE TABLE IF NOT EXISTS gambling_crash_queue(id INTEGER PRIMARY KEY AUTOINCREMENT,multiplier INTEGER NOT NULL CHECK(multiplier BETWEEN 100 AND 100000));
         CREATE TABLE IF NOT EXISTS gambling_crash_rounds(id INTEGER PRIMARY KEY AUTOINCREMENT,created_ms INTEGER NOT NULL,start_ms INTEGER NOT NULL,crash_ms INTEGER NOT NULL,multiplier INTEGER NOT NULL CHECK(multiplier BETWEEN 100 AND 100000),mode TEXT NOT NULL CHECK(mode IN ('random','controlled')),settled INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE IF NOT EXISTS gambling_crash_bets(round_id INTEGER NOT NULL REFERENCES gambling_crash_rounds(id),user_id INTEGER NOT NULL REFERENCES users(id),stake INTEGER NOT NULL CHECK(stake BETWEEN 1 AND 9007199254740991),auto_multiplier INTEGER,status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','won','lost')),payout INTEGER NOT NULL DEFAULT 0,cashout_multiplier INTEGER,cashout_at_ms INTEGER,PRIMARY KEY(round_id,user_id));
         CREATE TABLE IF NOT EXISTS gambling_blackjack(id TEXT PRIMARY KEY,user_id INTEGER NOT NULL REFERENCES users(id),stake INTEGER NOT NULL CHECK(stake BETWEEN 1 AND 9007199254740991),deck TEXT NOT NULL,cursor INTEGER NOT NULL,player TEXT NOT NULL,dealer TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'playing',payout INTEGER NOT NULL DEFAULT 0,created INTEGER NOT NULL);
         CREATE UNIQUE INDEX IF NOT EXISTS gambling_one_active_hand ON gambling_blackjack(user_id) WHERE status='playing';
         CREATE TABLE IF NOT EXISTS gambling_requests(user_id INTEGER NOT NULL REFERENCES users(id),request_id TEXT NOT NULL,fingerprint TEXT NOT NULL,response TEXT NOT NULL,created INTEGER NOT NULL,kind TEXT NOT NULL DEFAULT 'legacy',PRIMARY KEY(user_id,request_id));
         CREATE INDEX IF NOT EXISTS gambling_requests_daily ON gambling_requests(user_id,created);
         CREATE TABLE IF NOT EXISTS gambling_cosmetics(user_id INTEGER NOT NULL REFERENCES users(id),item_id TEXT NOT NULL,count INTEGER NOT NULL CHECK(count BETWEEN 1 AND 1000000),PRIMARY KEY(user_id,item_id));
         CREATE TABLE IF NOT EXISTS gambling_equipped(user_id INTEGER PRIMARY KEY REFERENCES users(id),frame TEXT,banner TEXT);",
    )?;
    let columns = db
        .prepare("PRAGMA table_info(gambling_requests)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !columns.iter().any(|name| name == "kind") {
        db.execute_batch(
            "ALTER TABLE gambling_requests ADD COLUMN kind TEXT NOT NULL DEFAULT 'legacy';",
        )?;
    }
    let columns = db
        .prepare("PRAGMA table_info(gambling_crash_bets)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !columns.iter().any(|name| name == "cashout_at_ms") {
        // Historical wins have no recorded clock time; leave them unknown.
        db.execute_batch("ALTER TABLE gambling_crash_bets ADD COLUMN cashout_at_ms INTEGER;")?;
    }
    let columns = db
        .prepare("PRAGMA table_info(gambling_equipped)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    for column in ["emblem", "name_effect"] {
        if !columns.iter().any(|name| name == column) {
            db.execute_batch(&format!(
                "ALTER TABLE gambling_equipped ADD COLUMN {column} TEXT;"
            ))?;
        }
    }
    // Warm immutable metadata during initialization, before this connection is
    // shared or used by gameplay transactions. Invalid metadata stays unavailable.
    let _ = cached_catalog();
    arcade::initialize(db)?;
    collection::initialize(db)?;
    migrate_crash_ceiling(db)?;
    migrate_wager_ceiling(db)?;
    let columns = db
        .prepare("PRAGMA table_info(gambling_config)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !columns.iter().any(|name| name == "luck_percent") {
        db.execute_batch("ALTER TABLE gambling_config ADD COLUMN luck_percent INTEGER NOT NULL DEFAULT 100 CHECK(luck_percent BETWEEN 25 AND 10000);")?;
    }
    Ok(())
}

fn milliseconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

// Startup-only migration. Dropping the original parent without renaming it
// keeps existing bet foreign keys pointing to the restored original name.
fn migrate_crash_ceiling(db: &Connection) -> rusqlite::Result<()> {
    let mut tables = Vec::new();
    for name in ["gambling_crash_queue", "gambling_crash_rounds"] {
        let sql: String =
            db.query_row("SELECT sql FROM sqlite_master WHERE name=?1", [name], |r| {
                r.get(0)
            })?;
        if sql.contains("BETWEEN 100 AND 10000)") {
            tables.push((name, sql));
        }
    }
    if tables.is_empty() {
        return Ok(());
    }
    let foreign_keys: bool = db.pragma_query_value(None, "foreign_keys", |r| r.get(0))?;
    db.pragma_update(None, "foreign_keys", false)?;
    let migrated = (|| {
        let tx = db.unchecked_transaction()?;
        for (name, sql) in tables {
            let sequence: Option<i64> = tx
                .query_row(
                    "SELECT seq FROM sqlite_sequence WHERE name=?1",
                    [name],
                    |r| r.get(0),
                )
                .optional()?;
            let temporary = format!("{name}_ceiling_migration");
            let sql = sql
                .replacen(name, &temporary, 1)
                .replace("BETWEEN 100 AND 10000)", "BETWEEN 100 AND 100000)");
            tx.execute_batch(&sql)?;
            tx.execute_batch(&format!("INSERT INTO {temporary} SELECT * FROM {name}; DROP TABLE {name}; ALTER TABLE {temporary} RENAME TO {name};"))?;
            if let Some(sequence) = sequence
                && tx.execute(
                    "UPDATE sqlite_sequence SET seq=MAX(seq,?1) WHERE name=?2",
                    params![sequence, name],
                )? == 0
            {
                tx.execute(
                    "INSERT INTO sqlite_sequence(name,seq) VALUES(?1,?2)",
                    params![name, sequence],
                )?;
            }
        }
        let invalid: i64 =
            tx.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })?;
        if invalid != 0 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        tx.commit()
    })();
    db.pragma_update(None, "foreign_keys", foreign_keys)?;
    migrated
}

// Preserve accepted bets, saved shoes, receipts and the one-active-hand index.
fn migrate_wager_ceiling(db: &Connection) -> rusqlite::Result<()> {
    let mut tables = Vec::new();
    for (name, old_limit) in [
        ("gambling_crash_bets", 1000000),
        ("gambling_blackjack", 2000000),
    ] {
        let sql: String =
            db.query_row("SELECT sql FROM sqlite_master WHERE name=?1", [name], |r| {
                r.get(0)
            })?;
        let old = format!("stake BETWEEN 1 AND {old_limit}");
        if sql.contains(&old) {
            tables.push((name, sql, old));
        }
    }
    if tables.is_empty() {
        return Ok(());
    }
    let foreign_keys: bool = db.pragma_query_value(None, "foreign_keys", |r| r.get(0))?;
    db.pragma_update(None, "foreign_keys", false)?;
    let migrated = (|| {
        let tx = db.unchecked_transaction()?;
        for (name, sql, old) in tables {
            let indexes = tx.prepare("SELECT sql FROM sqlite_master WHERE type='index' AND tbl_name=?1 AND sql IS NOT NULL")?.query_map([name], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            let temporary = format!("{name}_wager_migration");
            let sql = sql
                .replacen(name, &temporary, 1)
                .replace(&old, "stake BETWEEN 1 AND 9007199254740991");
            tx.execute_batch(&sql)?;
            tx.execute_batch(&format!("INSERT INTO {temporary} SELECT * FROM {name}; DROP TABLE {name}; ALTER TABLE {temporary} RENAME TO {name};"))?;
            for index in indexes {
                tx.execute_batch(&index)?;
            }
        }
        let invalid: i64 =
            tx.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })?;
        if invalid != 0 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        tx.commit()
    })();
    db.pragma_update(None, "foreign_keys", foreign_keys)?;
    migrated
}

// Large stakes use wide intermediates and stay exact in browser JSON/wallets.
fn whole_return(amount: i128) -> i64 {
    amount.clamp(0, cannabot::MAX_KASH as i128) as i64
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
}

fn wallet(db: &Connection, actor: i64) -> ApiResult<(i64, i64, i64)> {
    db.execute(
        "INSERT OR IGNORE INTO bot_wallets(user_id) VALUES(?1)",
        [actor],
    )?;
    Ok(db.query_row(
        "SELECT balance,earned,daily FROM bot_wallets WHERE user_id=?1",
        [actor],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?)
}

fn debit(db: &Connection, actor: i64, stake: i64) -> ApiResult<()> {
    if !(1..=MAX_STAKE).contains(&stake) {
        return Err(bad(
            "Choose a positive whole-Kash stake within your balance",
        ));
    }
    wallet(db, actor)?;
    if db.execute(
        "UPDATE bot_wallets SET balance=balance-?1 WHERE user_id=?2 AND balance>=?1",
        params![stake, actor],
    )? != 1
    {
        return Err(bad("Your Kash balance is too low for that stake"));
    }
    Ok(())
}

fn credit(db: &Connection, actor: i64, payout: i64, stake: i64) -> ApiResult<i64> {
    let balance = wallet(db, actor)?.0;
    let paid = payout.max(0).min(cannabot::MAX_KASH - balance);
    let profit = (paid - stake).max(0);
    db.execute(
        "UPDATE bot_wallets SET balance=balance+?1,earned=MIN(1000000,earned+?2) WHERE user_id=?3",
        params![paid, profit, actor],
    )?;
    Ok(paid)
}

// The optional identity header binds a pending browser action to its original
// member even if another tab replaces the shared session cookie. Old clients
// remain compatible; authentication always runs before this additional guard.
pub(super) fn mutation_actor(app: &App, headers: &HeaderMap) -> ApiResult<i64> {
    let actor = app.auth(headers)?.0;
    let mut values = headers.get_all("x-canna-member").iter();
    if let Some(value) = values.next() {
        let expected = value
            .to_str()
            .ok()
            .filter(|value| {
                !value.is_empty() && value.len() <= 19 && value.bytes().all(|c| c.is_ascii_digit())
            })
            .and_then(|value| value.parse::<i64>().ok());
        if values.next().is_some() || expected != Some(actor) || actor <= 0 {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "Your signed-in account changed; reload this page before starting another action",
            ));
        }
    }
    Ok(actor)
}

pub(super) fn wagering_paused(db: &Connection) -> ApiResult<bool> {
    Ok(
        db.query_row("SELECT paused FROM gambling_house WHERE id=1", [], |r| {
            r.get(0)
        })?,
    )
}

fn record(db: &Connection, actor: i64, kind: &str, target: &Value) -> ApiResult<()> {
    db.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,?2,?3,?4)",
        params![actor, kind, target.to_string(), now()],
    )?;
    Ok(())
}

fn once(
    app: &App,
    actor: i64,
    request: &str,
    kind: &str,
    payload: &Value,
    action: impl FnOnce(&Connection) -> ApiResult<Value>,
) -> ApiResult<axum::Json<Value>> {
    if !identifier(request) {
        return Err(bad(
            "Supply a request_id of 1 to 80 letters, digits, hyphens or underscores",
        ));
    }
    let fingerprint = digest(&format!("{kind}:{payload}"));
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let cached: Option<(String, String)> = tx
        .query_row(
            "SELECT fingerprint,response FROM gambling_requests WHERE user_id=?1 AND request_id=?2",
            params![actor, request],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((previous, result)) = cached {
        if previous != fingerprint {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "That request_id was already used for another action",
            ));
        }
        let result = serde_json::from_str(&result)
            .map_err(|_| bad("Stored game response is unavailable"))?;
        return Ok(axum::Json(result));
    }
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM gambling_requests WHERE user_id=?1 AND created>=?2 AND kind IN ('crash_bet','blackjack_deal','cosmetic_case','roulette','dice','slots','keno','plinko','wheel','baccarat')",
        params![actor, (now() / 86400) * 86400],
        |r| r.get(0),
    )?;
    if count >= arcade::daily_limit(&tx)?
        && matches!(
            kind,
            "crash_bet"
                | "blackjack_deal"
                | "cosmetic_case"
                | "roulette"
                | "dice"
                | "slots"
                | "keno"
                | "plinko"
                | "wheel"
                | "baccarat"
        )
    {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Today's new-game limit was reached; existing games remain playable",
        ));
    }
    let mut result = action(&tx)?;
    result["ok"] = json!(true);
    result["balance"] = json!(wallet(&tx, actor)?.0);
    tx.execute(
        "INSERT INTO gambling_requests(user_id,request_id,fingerprint,response,created,kind) VALUES(?1,?2,?3,?4,?5,?6)",
        params![actor, request, fingerprint, result.to_string(), now(), kind],
    )?;
    record(&tx, actor, kind, &result)?;
    tx.commit()?;
    Ok(axum::Json(result))
}

#[derive(Debug, Clone)]
struct Round {
    id: i64,
    start: i64,
    crash: i64,
    multiplier: i64,
    mode: String,
}

fn latest_round(db: &Connection) -> ApiResult<Option<Round>> {
    Ok(db.query_row(
        "SELECT id,start_ms,crash_ms,multiplier,mode FROM gambling_crash_rounds ORDER BY id DESC LIMIT 1",
        [],
        |r| Ok(Round { id: r.get(0)?, start: r.get(1)?, crash: r.get(2)?, multiplier: r.get(3)?, mode: r.get(4)? }),
    ).optional()?)
}

fn multiplier_for_sample(sample: f64, luck_percent: i64) -> i64 {
    (97.0 * (luck_percent as f64 / 100.0) / (1.0 - sample))
        .floor()
        .clamp(100.0, MAX_MULTIPLIER as f64) as i64
}

fn random_multiplier(luck_percent: i64) -> i64 {
    multiplier_for_sample(OsRng.gen_range(0.0..1.0), luck_percent)
}

fn at_multiplier(start: i64, multiplier: i64) -> i64 {
    start.saturating_add(((multiplier as f64 / 100.0).ln() * 10_000.0).ceil() as i64)
}

fn running_multiplier(round: &Round, time: i64) -> i64 {
    if time < round.start {
        return 100;
    }
    if time >= round.crash {
        return round.multiplier;
    }
    (((time - round.start) as f64 / 10_000.0).exp() * 100.0)
        .floor()
        .max(100.0)
        .min((round.multiplier - 1).max(100) as f64) as i64
}

fn settle_bet(
    db: &Connection,
    round: i64,
    actor: i64,
    multiplier: Option<i64>,
    cashout_at: i64,
) -> ApiResult<()> {
    let stake: Option<i64> = db.query_row(
        "SELECT stake FROM gambling_crash_bets WHERE round_id=?1 AND user_id=?2 AND status='pending'",
        params![round, actor],
        |r| r.get(0),
    ).optional()?;
    if let Some(stake) = stake {
        let payout = match multiplier {
            Some(multiplier) => credit(
                db,
                actor,
                whole_return(stake as i128 * multiplier as i128 / 100),
                stake,
            )?,
            None => 0,
        };
        db.execute(
            "UPDATE gambling_crash_bets SET status=?1,payout=?2,cashout_multiplier=?3,cashout_at_ms=?4 WHERE round_id=?5 AND user_id=?6 AND status='pending'",
            params![if multiplier.is_some() {"won"} else {"lost"}, payout, multiplier, multiplier.map(|_| cashout_at), round, actor],
        )?;
    }
    Ok(())
}

fn advance(db: &Connection, time: i64) -> ApiResult<Option<Round>> {
    let last = latest_round(db)?;
    if let Some(round) = &last {
        let bets = db.prepare(
            "SELECT user_id,auto_multiplier FROM gambling_crash_bets WHERE round_id=?1 AND status='pending'",
        )?.query_map([round.id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        for (actor, auto) in bets {
            if let Some(auto) = auto.filter(|auto| {
                // Reaching the hard ceiling completes a maximum-target auto bet.
                // Settle from its scheduled time even when the next poll is late.
                (*auto < round.multiplier
                    || (*auto == MAX_MULTIPLIER && round.multiplier == MAX_MULTIPLIER))
                    && time >= at_multiplier(round.start, *auto)
            }) {
                settle_bet(
                    db,
                    round.id,
                    actor,
                    Some(auto),
                    at_multiplier(round.start, auto),
                )?;
            } else if time >= round.crash {
                settle_bet(db, round.id, actor, None, time)?;
            }
        }
        if time >= round.crash {
            db.execute(
                "UPDATE gambling_crash_rounds SET settled=1 WHERE id=?1",
                [round.id],
            )?;
        }
        if time < round.crash.saturating_add(INTERMISSION_MS) {
            return Ok(last);
        }
    }
    let (mode, paused, luck_percent): (String, bool, i64) = db.query_row(
        "SELECT mode,paused,luck_percent FROM gambling_config WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if paused || arcade::paused(db, "crash")? {
        return Ok(None);
    }
    let queued: Option<(i64, i64)> = if mode == "controlled" {
        db.query_row(
            "SELECT id,multiplier FROM gambling_crash_queue ORDER BY id LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
    } else {
        None
    };
    let multiplier = if let Some((id, multiplier)) = queued {
        db.execute("DELETE FROM gambling_crash_queue WHERE id=?1", [id])?;
        multiplier
    } else {
        random_multiplier(luck_percent)
    };
    let start = time.saturating_add(BETTING_MS);
    let crash = at_multiplier(start, multiplier);
    db.execute(
        "INSERT INTO gambling_crash_rounds(created_ms,start_ms,crash_ms,multiplier,mode) VALUES(?1,?2,?3,?4,?5)",
        params![time, start, crash, multiplier, mode],
    )?;
    Ok(Some(Round {
        id: db.last_insert_rowid(),
        start,
        crash,
        multiplier,
        mode,
    }))
}

fn bet_view(db: &Connection, round: i64, actor: i64) -> ApiResult<Value> {
    Ok(db.query_row(
        "SELECT stake,auto_multiplier,status,payout,cashout_multiplier,cashout_at_ms FROM gambling_crash_bets WHERE round_id=?1 AND user_id=?2",
        params![round, actor],
        |r| Ok(json!({"round_id":round,"stake":r.get::<_,i64>(0)?,"auto_cashout":r.get::<_,Option<i64>>(1)?.map(|v|v as f64/100.0),"status":r.get::<_,String>(2)?,"payout":r.get::<_,i64>(3)?,"cashout_multiplier":r.get::<_,Option<i64>>(4)?.map(|v|v as f64/100.0),"cashout_at_ms":r.get::<_,Option<i64>>(5)?})),
    ).optional()?.unwrap_or(Value::Null))
}

fn participant_view(db: &Connection, round: &Round, after: i64) -> ApiResult<Value> {
    let count: i64 = db.query_row(
        "SELECT count(*) FROM gambling_crash_bets WHERE round_id=?1",
        [round.id],
        |r| r.get(0),
    )?;
    let mut participants = db.prepare(
        "SELECT b.user_id,CASE WHEN u.verified=1 AND u.banned=0 THEN u.username ELSE NULL END,b.stake,b.status,b.cashout_multiplier,b.cashout_at_ms FROM gambling_crash_bets b JOIN users u ON u.id=b.user_id WHERE b.round_id=?1 AND b.user_id>?2 ORDER BY b.user_id LIMIT ?3",
    )?.query_map(params![round.id,after,PARTICIPANT_PAGE_SIZE+1], |r| {
        let id: i64 = r.get(0)?;
        let name: Option<String> = r.get(1)?;
        let cashout_at: Option<i64> = r.get(5)?;
        Ok(json!({"user_id":id,"username":name,"display_name":name.as_deref().unwrap_or("Unavailable member"),"profile_url":name.as_ref().map(|_|format!("/members/{id}")),"stake":r.get::<_,i64>(2)?,"status":r.get::<_,String>(3)?,"cashout_multiplier":r.get::<_,Option<i64>>(4)?.map(|v|v as f64/100.0),"cashout_at_ms":cashout_at,"cashout_elapsed_ms":cashout_at.map(|at|at.saturating_sub(round.start).max(0))}))
    })?.collect::<Result<Vec<_>,_>>()?;
    let has_more = participants.len() > PARTICIPANT_PAGE_SIZE as usize;
    participants.truncate(PARTICIPANT_PAGE_SIZE as usize);
    let next = has_more.then(|| participants.last().unwrap()["user_id"].clone());
    Ok(
        json!({"participants":participants,"participant_count":count,"participant_has_more":has_more,"participant_next_after_user_id":next,"participant_page_size":PARTICIPANT_PAGE_SIZE}),
    )
}

fn crash_view(
    db: &Connection,
    round: Option<&Round>,
    actor: i64,
    time: i64,
    owner: bool,
    after: i64,
) -> ApiResult<Value> {
    let (mode, paused): (String, bool) = db.query_row(
        "SELECT mode,paused FROM gambling_config WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let history = db.prepare("SELECT id,multiplier,mode FROM gambling_crash_rounds WHERE crash_ms<=?1 ORDER BY id DESC LIMIT 20")?
        .query_map([time], |r| Ok(json!({"id":r.get::<_,i64>(0)?,"crash_multiplier":r.get::<_,i64>(1)? as f64/100.0,"mode":r.get::<_,String>(2)?})))?
        .collect::<Result<Vec<_>,_>>()?;
    let paused = paused || arcade::paused(db, "crash")?;
    let mut view = json!({"id":null,"phase":"paused","multiplier":1.0,"mode":mode,"paused":paused,"owner_visible":true,"bet":null,"history":history,"participants":[],"participant_count":0,"participant_has_more":false,"participant_next_after_user_id":null,"participant_page_size":PARTICIPANT_PAGE_SIZE});
    // Pausing stops new rounds, not the display of the most recent round's bets.
    let previous = if round.is_none() {
        latest_round(db)?
    } else {
        None
    };
    let round = round.or(previous.as_ref());
    if let Some(round) = round {
        view["id"] = json!(round.id);
        view["phase"] = json!(if time < round.start {
            "betting"
        } else if time < round.crash {
            "running"
        } else {
            "crashed"
        });
        view["mode"] = json!(round.mode);
        view["betting_ends_ms"] = json!(round.start);
        view["multiplier"] = json!(running_multiplier(round, time) as f64 / 100.0);
        view["bet"] = bet_view(db, round.id, actor)?;
        for (key, value) in participant_view(db, round, after)?.as_object().unwrap() {
            view[key] = value.clone();
        }
        if time >= round.crash {
            view["crash_multiplier"] = json!(round.multiplier as f64 / 100.0);
            view["crashed_at_ms"] = json!(round.crash);
        }
        if owner {
            view["planned_crash_multiplier"] = json!(round.multiplier as f64 / 100.0);
            view["planned_crash_at_ms"] = json!(round.crash);
        }
    }
    Ok(view)
}

fn centi(multiplier: f64) -> ApiResult<i64> {
    if !multiplier.is_finite() || !(1.0..=1000.0).contains(&multiplier) {
        return Err(bad("Crash multipliers must be from 1.00 to 1000.00"));
    }
    Ok((multiplier * 100.0).round() as i64)
}

#[derive(Deserialize, Serialize)]
pub struct BetInput {
    request_id: String,
    round_id: i64,
    stake: i64,
    auto_cashout: Option<f64>,
}
pub async fn crash_bet(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<BetInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = mutation_actor(&app, &headers)?;
    once(
        &app,
        actor,
        &input.request_id,
        "crash_bet",
        &json!({"round_id":input.round_id,"stake":input.stake,"auto_cashout":input.auto_cashout}),
        |db| {
            arcade::new_game(db, "crash", input.stake)?;
            let time = milliseconds();
            let round = advance(db, time)?.ok_or_else(|| bad("New Crash rounds are paused"))?;
            let paused: bool =
                db.query_row("SELECT paused FROM gambling_config WHERE id=1", [], |r| {
                    r.get(0)
                })?;
            if paused || round.id != input.round_id || time >= round.start {
                return Err(bad("That round is no longer accepting bets"));
            }
            let exists: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM gambling_crash_bets WHERE round_id=?1 AND user_id=?2)",
                params![round.id, actor],
                |r| r.get(0),
            )?;
            if exists {
                return Err(ApiError(
                    StatusCode::CONFLICT,
                    "You already placed a bet in this round",
                ));
            }
            let auto = input.auto_cashout.map(centi).transpose()?;
            if auto == Some(100) {
                return Err(bad("Automatic cash-out must exceed 1.00x"));
            }
            debit(db, actor, input.stake)?;
            db.execute("INSERT INTO gambling_crash_bets(round_id,user_id,stake,auto_multiplier) VALUES(?1,?2,?3,?4)", params![round.id,actor,input.stake,auto])?;
            Ok(json!({"bet":bet_view(db,round.id,actor)?}))
        },
    )
}

#[derive(Deserialize, Serialize)]
pub struct CashoutInput {
    request_id: String,
    round_id: i64,
}
pub async fn crash_cashout(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<CashoutInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = mutation_actor(&app, &headers)?;
    once(
        &app,
        actor,
        &input.request_id,
        "crash_cashout",
        &json!({"round_id":input.round_id}),
        |db| {
            let time = milliseconds();
            advance(db, time)?;
            let round = db.query_row("SELECT id,start_ms,crash_ms,multiplier,mode FROM gambling_crash_rounds WHERE id=?1", [input.round_id], |r| Ok(Round{id:r.get(0)?,start:r.get(1)?,crash:r.get(2)?,multiplier:r.get(3)?,mode:r.get(4)?})).optional()?.ok_or_else(||bad("Crash round not found"))?;
            let bet = bet_view(db, round.id, actor)?;
            if bet.is_null() {
                return Err(bad("You have no bet in this round"));
            }
            if bet["status"] == "pending" {
                if time < round.start {
                    return Err(bad("The round has not started"));
                }
                settle_bet(
                    db,
                    round.id,
                    actor,
                    if time < round.crash {
                        Some(running_multiplier(&round, time))
                    } else {
                        None
                    },
                    time,
                )?;
            }
            Ok(json!({"bet":bet_view(db,round.id,actor)?}))
        },
    )
}

#[derive(Debug)]
struct Hand {
    id: String,
    actor: i64,
    stake: i64,
    deck: Vec<u8>,
    cursor: usize,
    player: Vec<u8>,
    dealer: Vec<u8>,
    status: String,
    payout: i64,
}
fn cards(text: String) -> rusqlite::Result<Vec<u8>> {
    serde_json::from_str(&text).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}
fn load_hand(db: &Connection, id: &str, actor: i64) -> ApiResult<Hand> {
    db.query_row("SELECT id,user_id,stake,deck,cursor,player,dealer,status,payout FROM gambling_blackjack WHERE id=?1 AND user_id=?2", params![id,actor], |r| {
        let deck = cards(r.get(3)?)?;
        let stored_cursor: u32 = r.get(4)?;
        let cursor = usize::try_from(stored_cursor).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(4, i64::from(stored_cursor)))?;
        if cursor > 52 || cursor > deck.len() {
            return Err(rusqlite::Error::IntegralValueOutOfRange(4, i64::from(stored_cursor)));
        }
        Ok(Hand{id:r.get(0)?,actor:r.get(1)?,stake:r.get(2)?,deck,cursor,player:cards(r.get(5)?)?,dealer:cards(r.get(6)?)?,status:r.get(7)?,payout:r.get(8)?})
    }).optional()?.ok_or(ApiError(StatusCode::NOT_FOUND,"Blackjack hand not found"))
}
fn total(cards: &[u8]) -> i64 {
    let mut aces = 0;
    let mut total = 0;
    for card in cards {
        let rank = card % 13;
        total += match rank {
            0 => {
                aces += 1;
                11
            }
            1..=8 => i64::from(rank + 1),
            _ => 10,
        };
    }
    while total > 21 && aces > 0 {
        total -= 10;
        aces -= 1;
    }
    total
}
fn draw(hand: &mut Hand, dealer: bool) -> ApiResult<()> {
    let card = hand
        .deck
        .get(hand.cursor)
        .copied()
        .ok_or_else(|| bad("The stored card shoe is exhausted"))?;
    hand.cursor += 1;
    if dealer {
        hand.dealer.push(card);
    } else {
        hand.player.push(card);
    }
    Ok(())
}
fn card_view(card: u8) -> Value {
    let ranks = [
        "A", "2", "3", "4", "5", "6", "7", "8", "9", "10", "J", "Q", "K",
    ];
    let suits = ["spades", "hearts", "clubs", "diamonds"];
    json!({"rank":ranks[usize::from(card%13)],"suit":suits[usize::from(card/13)]})
}
fn hand_view(hand: &Hand) -> Value {
    let playing = hand.status == "playing";
    let dealer: Vec<Value> = hand
        .dealer
        .iter()
        .enumerate()
        .map(|(i, card)| {
            if playing && i > 0 {
                json!({"hidden":true})
            } else {
                card_view(*card)
            }
        })
        .collect();
    let mut value = json!({"id":hand.id,"stake":hand.stake,"cards":hand.player.iter().map(|c|card_view(*c)).collect::<Vec<_>>(),"dealer":dealer,"player_total":total(&hand.player),"status":hand.status,"payout":hand.payout,"can_double":playing && hand.player.len()==2 && hand.stake<=MAX_STAKE/2});
    if !playing {
        value["dealer_total"] = json!(total(&hand.dealer));
    }
    value
}
fn save_hand(db: &Connection, hand: &Hand) -> ApiResult<()> {
    db.execute("UPDATE gambling_blackjack SET stake=?1,cursor=?2,player=?3,dealer=?4,status=?5,payout=?6 WHERE id=?7 AND user_id=?8",params![hand.stake,hand.cursor as i64,json!(hand.player).to_string(),json!(hand.dealer).to_string(),hand.status,hand.payout,hand.id,hand.actor])?;
    Ok(())
}
fn finish_hand(db: &Connection, hand: &mut Hand, natural: bool) -> ApiResult<()> {
    if hand.status != "playing" {
        return Ok(());
    }
    let player = total(&hand.player);
    let dealer = total(&hand.dealer);
    let (status, payout) = if player > 21 {
        ("lost", 0)
    } else if player == dealer {
        ("push", hand.stake)
    } else if natural && player == 21 {
        ("blackjack", whole_return(hand.stake as i128 * 5 / 2))
    } else if natural && dealer == 21 {
        ("lost", 0)
    } else if dealer > 21 || player > dealer {
        ("won", whole_return(hand.stake as i128 * 2))
    } else {
        ("lost", 0)
    };
    hand.status = status.into();
    hand.payout = credit(db, hand.actor, payout, hand.stake)?;
    Ok(())
}

#[derive(Deserialize, Serialize)]
pub struct DealInput {
    request_id: String,
    stake: i64,
}
pub async fn blackjack_deal(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<DealInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = mutation_actor(&app, &headers)?;
    once(
        &app,
        actor,
        &input.request_id,
        "blackjack_deal",
        &json!({"stake":input.stake}),
        |db| {
            arcade::new_game(db, "blackjack", input.stake)?;
            let active:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM gambling_blackjack WHERE user_id=?1 AND status='playing')",[actor],|r|r.get(0))?;
            if active {
                return Err(ApiError(
                    StatusCode::CONFLICT,
                    "Finish your existing blackjack hand first",
                ));
            }
            debit(db, actor, input.stake)?;
            let mut deck: Vec<u8> = (0..52).collect();
            deck.shuffle(&mut OsRng);
            let mut hand = Hand {
                id: Uuid::new_v4().to_string(),
                actor,
                stake: input.stake,
                deck,
                cursor: 0,
                player: Vec::new(),
                dealer: Vec::new(),
                status: "playing".into(),
                payout: 0,
            };
            draw(&mut hand, false)?;
            draw(&mut hand, true)?;
            draw(&mut hand, false)?;
            draw(&mut hand, true)?;
            if total(&hand.player) == 21 || total(&hand.dealer) == 21 {
                finish_hand(db, &mut hand, true)?;
            }
            db.execute(
                "INSERT INTO gambling_blackjack VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    hand.id,
                    actor,
                    hand.stake,
                    json!(hand.deck).to_string(),
                    hand.cursor as i64,
                    json!(hand.player).to_string(),
                    json!(hand.dealer).to_string(),
                    hand.status,
                    hand.payout,
                    now()
                ],
            )?;
            Ok(json!({"blackjack":hand_view(&hand)}))
        },
    )
}

#[derive(Deserialize, Serialize)]
pub struct HandInput {
    request_id: String,
    hand_id: String,
    action: String,
}
pub async fn blackjack_action(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<HandInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = mutation_actor(&app, &headers)?;
    once(
        &app,
        actor,
        &input.request_id,
        "blackjack_action",
        &json!({"hand_id":input.hand_id,"action":input.action}),
        |db| {
            let mut hand = load_hand(db, &input.hand_id, actor)?;
            if hand.status != "playing" {
                return Ok(json!({"blackjack":hand_view(&hand)}));
            }
            match input.action.as_str() {
                "hit" => {
                    draw(&mut hand, false)?;
                    if total(&hand.player) > 21 {
                        finish_hand(db, &mut hand, false)?;
                    }
                }
                "stand" => {
                    while total(&hand.dealer) < 17 {
                        draw(&mut hand, true)?;
                    }
                    finish_hand(db, &mut hand, false)?;
                }
                "double" => {
                    if hand.player.len() != 2 {
                        return Err(bad("Double is available only on your first two cards"));
                    }
                    if hand.stake > MAX_STAKE / 2 {
                        return Err(bad("Double down exceeds supported whole-Kash precision"));
                    }
                    debit(db, actor, hand.stake)?;
                    hand.stake *= 2;
                    draw(&mut hand, false)?;
                    if total(&hand.player) <= 21 {
                        while total(&hand.dealer) < 17 {
                            draw(&mut hand, true)?;
                        }
                    }
                    finish_hand(db, &mut hand, false)?;
                }
                _ => return Err(bad("Choose hit, stand or double")),
            }
            save_hand(db, &hand)?;
            Ok(json!({"blackjack":hand_view(&hand)}))
        },
    )
}

struct CosmeticCatalog {
    value: Value,
    version: String,
    cases: Value,
    response: Vec<u8>,
}

fn build_catalog(source: &str) -> ApiResult<CosmeticCatalog> {
    let mut catalog: Value =
        serde_json::from_str(source).map_err(|_| bad("Cosmetics catalog is unavailable"))?;
    let paused = catalog["paused_collections"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for item in catalog["items"]
        .as_array_mut()
        .ok_or_else(|| bad("Cosmetics catalog is unavailable"))?
    {
        if paused
            .iter()
            .any(|collection| collection == &item["collection"])
        {
            item["paused"] = json!(true);
        }
    }
    let items = catalog["items"]
        .as_array()
        .ok_or_else(|| bad("Cosmetics catalog is unavailable"))?;
    if items.is_empty() || items.len() > 2000 {
        return Err(bad("Cosmetics catalog is unavailable"));
    }
    let mut ids = std::collections::HashSet::new();
    let mut sum = 0_u64;
    for item in items {
        let id = item["id"]
            .as_str()
            .ok_or_else(|| bad("Cosmetics catalog is unavailable"))?;
        if !identifier(id)
            || !ids.insert(id)
            || !matches!(
                item["kind"].as_str(),
                Some("frame" | "banner" | "emblem" | "name_effect")
            )
            || !matches!(
                (item["kind"].as_str(), item["collection"].as_str()),
                (Some("frame"), Some("frames"))
                    | (Some("banner"), Some("bo2" | "mw2" | "canna"))
                    | (Some("emblem"), Some("mw2-emblems" | "cod-ranks"))
                    | (Some("name_effect"), Some("username-effects"))
            )
        {
            return Err(bad("Cosmetics catalog is unavailable"));
        }
        if item["kind"] == "name_effect"
            && !matches!(
                item["style"].as_str(),
                Some(
                    "aurora"
                        | "canna"
                        | "sunset"
                        | "royal"
                        | "ice"
                        | "rainbow"
                        | "ember"
                        | "ocean"
                        | "nebula"
                        | "candy"
                        | "forest"
                        | "silver"
                        | "toxic"
                        | "rose"
                        | "lava"
                        | "midnight"
                        | "prism"
                        | "bliss"
                )
            )
        {
            return Err(bad("Cosmetics catalog is unavailable"));
        }
        let weight = item["weight"]
            .as_u64()
            .ok_or_else(|| bad("Cosmetics catalog is unavailable"))?;
        sum = sum
            .checked_add(weight)
            .ok_or_else(|| bad("Cosmetics catalog is unavailable"))?;
    }
    if sum == 0
        || sum > 1_000_000
        || catalog["case"]["id"] != "canna-case"
        || !catalog["case"]["price"]
            .as_i64()
            .is_some_and(|p| (1..=MAX_CRATE_PRICE).contains(&p))
    {
        return Err(bad("Cosmetics catalog is unavailable"));
    }
    let version = hex::encode(Sha256::digest(source.as_bytes()));
    let cases = cases_view(&catalog)?;
    let response =
        serde_json::to_vec(&json!({"version":version,"catalog":catalog["items"],"cases":cases}))
            .map_err(|_| bad("Cosmetics catalog is unavailable"))?;
    Ok(CosmeticCatalog {
        value: catalog,
        version,
        cases,
        response,
    })
}

fn cached_catalog() -> ApiResult<&'static CosmeticCatalog> {
    // Catalog bytes are compiled into this executable. A deployment gets a new
    // fingerprint and cache; no account data or ownership enters this cache.
    static CATALOG: std::sync::OnceLock<Option<CosmeticCatalog>> = std::sync::OnceLock::new();
    CATALOG
        .get_or_init(|| build_catalog(include_str!("../web/cosmetics/catalog.json")).ok())
        .as_ref()
        .ok_or_else(|| bad("Cosmetics catalog is unavailable"))
}

fn catalog() -> ApiResult<&'static Value> {
    Ok(&cached_catalog()?.value)
}

pub async fn cosmetics_catalog(
    State(app): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    // Even a fully warmed process cache requires current membership per request.
    app.auth(&headers)?;
    let catalog = cached_catalog()?;
    Ok((
        [
            ("content-type", "application/json"),
            ("cache-control", "private, no-store"),
            ("vary", "Cookie, Authorization"),
            ("x-content-type-options", "nosniff"),
        ],
        catalog.response.as_slice(),
    )
        .into_response())
}

fn case_definition(id: &str) -> ApiResult<(&'static str, Option<&'static str>, &'static str)> {
    match id {
        "bo2-calling-cards" => Ok(("BO2 calling cards crate", Some("bo2"), "banner")),
        "mw2-calling-cards" => Ok(("MW2 calling cards crate", Some("mw2"), "banner")),
        "avatar-frames" => Ok(("Avatar frames crate", Some("frames"), "frame")),
        "cod-emblems" => Ok(("Call of Duty emblems crate", None, "emblem")),
        "username-effects" => Ok((
            "Animated usernames crate",
            Some("username-effects"),
            "name_effect",
        )),
        "bo2-animated" => Ok(("BO2 motion crate", Some("bo2"), "banner")),
        "mw2-canna" => Ok(("MW2 green collection", Some("mw2"), "banner")),
        "premium-cosmetics" => Ok(("Rare & legendary vault", None, "all")),
        "rank-emblems" => Ok(("Rank & prestige crate", Some("cod-ranks"), "emblem")),
        // Old clients can still open the original mixed case. Its inventory IDs
        // remain valid, while new clients show separate collections.
        "canna-case" => Ok(("Canna cosmetics case", None, "mixed")),
        _ => Err(bad("Cosmetic case not found")),
    }
}

fn case_pool<'a>(catalog: &'a Value, id: &str) -> ApiResult<(Vec<&'a Value>, u64)> {
    let (_, collection, kind) = case_definition(id)?;
    let mut pool = Vec::new();
    let mut sum = 0_u64;
    for item in catalog["items"]
        .as_array()
        .ok_or_else(|| bad("Cosmetics catalog is unavailable"))?
    {
        if item["paused"] == true {
            continue;
        }
        // The legacy mixed crate remains compatible with clients that only
        // understand frame and banner equipment.
        if kind == "mixed" && !matches!(item["kind"].as_str(), Some("frame" | "banner")) {
            continue;
        }
        if (id == "bo2-animated" && item["animated"] != true)
            || (id == "mw2-canna"
                && !["blunt trauma", "high command", "joint ops"]
                    .iter()
                    .any(|n| {
                        item["name"]
                            .as_str()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(n)
                    }))
            || (id == "premium-cosmetics"
                && !matches!(item["rarity"].as_str(), Some("rare" | "epic" | "legendary")))
        {
            continue;
        }
        if (!matches!(kind, "mixed" | "all") && item["kind"] != kind)
            || collection.is_some_and(|collection| item["collection"] != collection)
        {
            continue;
        }
        let weight = item["weight"]
            .as_u64()
            .ok_or_else(|| bad("Cosmetics catalog is unavailable"))?;
        if weight > 0 {
            sum = sum
                .checked_add(weight)
                .filter(|sum| *sum <= 1_000_000)
                .ok_or_else(|| bad("Cosmetics catalog is unavailable"))?;
            pool.push(item);
        }
    }
    Ok((pool, sum))
}

#[cfg(test)]
fn select_case_item(catalog: &Value, id: &str, mut roll: u64) -> ApiResult<Value> {
    let (pool, sum) = case_pool(catalog, id)?;
    if sum == 0 || roll >= sum {
        return Err(bad("Cosmetic case is unavailable"));
    }
    for item in pool {
        let weight = item["weight"].as_u64().unwrap();
        if roll < weight {
            return Ok(item.clone());
        }
        roll -= weight;
    }
    Err(bad("Cosmetic case is unavailable"))
}

fn cases_view(catalog: &Value) -> ApiResult<Value> {
    CASE_IDS.into_iter()
        .map(|id| {
            let (name, collection, kind) = case_definition(id)?;
            let (pool, sum) = case_pool(catalog, id)?;
            let paused = catalog["paused_collections"].as_array().is_some_and(|paused| collection.is_some_and(|collection| paused.iter().any(|value| value == collection)));
            Ok(json!({"id":id,"name":name,"cost":arcade::default_case_cost(id),"collection":collection,"kind":kind,"item_count":pool.len(),"available":sum>0,"paused":paused,"pause_reason":if paused {Some("Paused due to artwork quality. Owned calling cards are saved.")} else {None},"contents":name,"items":pool.iter().map(|item|json!({"id":item["id"],"odds_percent":100.0*item["weight"].as_u64().unwrap() as f64/sum as f64})).collect::<Vec<_>>(),"duplicates":"Duplicates add copies; recycle spare copies for Kash while keeping the first copy."}))
        })
        .collect::<ApiResult<Vec<_>>>()
        .map(|cases| json!(cases))
}

fn cosmetics_view(
    db: &Connection,
    actor: i64,
    catalog: &CosmeticCatalog,
    include_catalog: bool,
) -> ApiResult<(Value, Option<Value>)> {
    let owned = db
        .prepare("SELECT item_id,count FROM gambling_cosmetics WHERE user_id=?1 ORDER BY item_id")?
        .query_map([actor], |r| {
            Ok(json!({"id":r.get::<_,String>(0)?,"count":r.get::<_,i64>(1)?}))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let (frame, banner, emblem, name_effect): (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = db
        .query_row(
            "SELECT frame,banner,emblem,name_effect FROM gambling_equipped WHERE user_id=?1",
            [actor],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .unwrap_or_default();
    let mut cosmetics = json!({"catalog_version":catalog.version,"owned":owned,"equipped":{"frame":frame,"banner":banner,"emblem":emblem,"name_effect":name_effect}});
    cosmetics["collection"] = collection::view(db, actor)?;
    let cases = if include_catalog {
        cosmetics["catalog"] = catalog.value["items"].clone();
        Some(catalog.cases.clone())
    } else {
        None
    };
    Ok((cosmetics, cases))
}

pub fn equipped(db: &Connection, actor: i64) -> ApiResult<Value> {
    let (frame, banner, emblem, name_effect): (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = db
        .query_row(
            "SELECT frame,banner,emblem,name_effect FROM gambling_equipped WHERE user_id=?1",
            [actor],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .unwrap_or_default();
    let catalog = catalog()?;
    let items = catalog["items"].as_array().unwrap();
    let find = |id: Option<String>| {
        id.and_then(|id| {
            items
                .iter()
                .find(|item| item["id"] == id && item["paused"] != true)
                .cloned()
        })
        .unwrap_or(Value::Null)
    };
    Ok(
        json!({"frame":find(frame),"banner":find(banner),"emblem":find(emblem),"name_effect":find(name_effect)}),
    )
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseInput {
    request_id: String,
    case_id: String,
}
pub async fn case_open(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<CaseInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = mutation_actor(&app, &headers)?;
    let catalog = catalog()?;
    once(
        &app,
        actor,
        &input.request_id,
        "cosmetic_case",
        &json!({"case_id":input.case_id}),
        |db| {
            let cost = arcade::case_cost(db, &input.case_id)?;
            arcade::new_game(db, "cases", cost)?;
            let (pool, sum) = arcade::weighted_pool(db, catalog, &input.case_id)?;
            if sum == 0 {
                return Err(bad("Cosmetic case is unavailable or paused"));
            }
            let mut roll = OsRng.gen_range(0..sum);
            let item = pool
                .into_iter()
                .find_map(|(item, weight)| {
                    if roll < weight {
                        Some(item.clone())
                    } else {
                        roll -= weight;
                        None
                    }
                })
                .ok_or_else(|| bad("Cosmetic case is unavailable"))?;
            debit(db, actor, cost)?;
            let id = item["id"].as_str().unwrap();
            db.execute("INSERT INTO gambling_cosmetics VALUES(?1,?2,1) ON CONFLICT(user_id,item_id) DO UPDATE SET count=MIN(1000000,count+1)",params![actor,id])?;
            let count: i64 = db.query_row(
                "SELECT count FROM gambling_cosmetics WHERE user_id=?1 AND item_id=?2",
                params![actor, id],
                |r| r.get(0),
            )?;
            Ok(json!({"case_id":input.case_id,"item":item,"count":count,"cost":cost}))
        },
    )
}

#[derive(Deserialize)]
pub struct EquipInput {
    frame: Option<String>,
    banner: Option<String>,
    #[serde(default, deserialize_with = "nullable_equipment")]
    emblem: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable_equipment")]
    name_effect: Option<Option<String>>,
}
fn nullable_equipment<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}
pub async fn equip(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<EquipInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = mutation_actor(&app, &headers)?;
    let catalog = catalog()?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let previous: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = tx
        .query_row(
            "SELECT frame,banner,emblem,name_effect FROM gambling_equipped WHERE user_id=?1",
            [actor],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .unwrap_or_default();
    // Older clients do not know these slots. Omission preserves them, while an
    // explicit JSON null removes them. Every newly selected item still needs ownership.
    let emblem = input.emblem.unwrap_or_else(|| previous.2.clone());
    let name_effect = input.name_effect.unwrap_or_else(|| previous.3.clone());
    for (kind, id, previous_id) in [
        ("frame", &input.frame, &previous.0),
        ("banner", &input.banner, &previous.1),
        ("emblem", &emblem, &previous.2),
        ("name_effect", &name_effect, &previous.3),
    ] {
        if let Some(id) = id {
            let item = catalog["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["id"] == *id && i["kind"] == kind)
                .ok_or_else(|| bad("Choose an item of the correct cosmetic kind"))?;
            if item["paused"] == true && previous_id.as_ref() != Some(id) {
                return Err(bad(
                    "This cosmetic collection is paused due to artwork quality",
                ));
            }
            let owned: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM gambling_cosmetics WHERE user_id=?1 AND item_id=?2)",
                params![actor, id],
                |r| r.get(0),
            )?;
            if !owned {
                return Err(ApiError(
                    StatusCode::FORBIDDEN,
                    "That cosmetic is not in your collection",
                ));
            }
        }
    }
    tx.execute("INSERT INTO gambling_equipped(user_id,frame,banner,emblem,name_effect) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(user_id) DO UPDATE SET frame=excluded.frame,banner=excluded.banner,emblem=excluded.emblem,name_effect=excluded.name_effect",params![actor,input.frame,input.banner,emblem,name_effect])?;
    tx.commit()?;
    Ok(axum::Json(
        json!({"ok":true,"equipped":{"frame":input.frame,"banner":input.banner,"emblem":emblem,"name_effect":name_effect}}),
    ))
}

#[derive(Deserialize)]
pub struct RequestInput {
    request_id: String,
}
pub async fn daily(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<RequestInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = mutation_actor(&app, &headers)?;
    once(
        &app,
        actor,
        &input.request_id,
        "kash_daily",
        &json!({}),
        |db| {
            let (balance, _, claimed) = wallet(db, actor)?;
            let day = now() / 86400;
            if claimed == day {
                return Err(bad(
                    "Daily Kash was already claimed; return after 00:00 UTC",
                ));
            }
            let reward = 100.min(cannabot::MAX_KASH - balance);
            db.execute("UPDATE bot_wallets SET balance=balance+?1,earned=MIN(1000000,earned+?1),daily=?2 WHERE user_id=?3",params![reward,day,actor])?;
            db.execute(
                "UPDATE notifications SET read=1 WHERE user_id=?1 AND dedup=?2",
                params![actor, format!("daily:{day}")],
            )?;
            Ok(json!({"reward":reward}))
        },
    )
}

#[derive(Default, Deserialize)]
pub struct OverviewQuery {
    crash_round_id: Option<i64>,
    crash_after_user_id: Option<i64>,
    catalog_version: Option<String>,
    #[serde(default)]
    crash_only: bool,
}

pub async fn overview(
    State(app): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<OverviewQuery>,
) -> ApiResult<axum::Json<Value>> {
    let actor = app.auth(&headers)?.0;
    if query.crash_round_id.is_some_and(|id| id <= 0)
        || query.crash_after_user_id.is_some_and(|id| id < 0)
        || (query.crash_after_user_id.is_some() && query.crash_round_id.is_none())
    {
        return Err(bad(
            "Supply the current Crash round and a valid participant cursor",
        ));
    }
    if query.catalog_version.as_ref().is_some_and(|version| {
        version.len() != 64
            || !version
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    }) {
        return Err(bad("Supply a valid cosmetic catalog fingerprint"));
    }
    // Resolve and validate immutable metadata before acquiring the game DB lock.
    // The participant-only branch needs no catalog at all.
    let catalog = if query.crash_after_user_id.is_none() && !query.crash_only {
        Some(cached_catalog()?)
    } else {
        None
    };
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let time = milliseconds();
    let round = advance(&tx, time)?;
    let crash = crash_view(
        &tx,
        round.as_ref(),
        actor,
        time,
        false,
        query.crash_after_user_id.unwrap_or(0),
    )?;
    if query.crash_round_id.is_some_and(|id| crash["id"] != id) {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "The Crash round changed; reload its participants",
        ));
    }
    if query.crash_after_user_id.is_some() {
        // Participant polling shares the authenticated clock and transaction,
        // without repeating the wallet, blackjack or large cosmetics catalog.
        tx.commit()?;
        return Ok(axum::Json(
            json!({"member_id":actor,"server_time_ms":time,"crash":crash}),
        ));
    }
    let (balance, earned, daily) = wallet(&tx, actor)?;
    if query.crash_only {
        tx.commit()?;
        return Ok(axum::Json(
            json!({"member_id":actor,"server_time_ms":time,"crash":crash,"wallet":{"balance":balance,"earned":earned,"daily_available":daily!=now()/86400}}),
        ));
    }
    let hand_id:Option<String>=tx.query_row("SELECT id FROM gambling_blackjack WHERE user_id=?1 ORDER BY created DESC,rowid DESC LIMIT 1",[actor],|r|r.get(0)).optional()?;
    let hand = hand_id
        .map(|id| load_hand(&tx, &id, actor).map(|h| hand_view(&h)))
        .transpose()?
        .unwrap_or(Value::Null);
    let catalog = catalog.unwrap();
    let include_catalog = query.catalog_version.as_deref() != Some(catalog.version.as_str());
    let (cosmetics, cases) = cosmetics_view(&tx, actor, catalog, include_catalog)?;
    let result = json!({"member_id":actor,"wallet":{"balance":balance,"earned":earned,"daily_available":daily!=now()/86400},"server_time_ms":time,"notice":NOTICE,"crash":crash,"blackjack":hand,"cosmetics":cosmetics,"cases":cases,"rules":arcade::rules_view(&tx)?,"recent_games":arcade::recent(&tx,actor)?,"limits":{"max_stake":MAX_STAKE,"max_new_games_per_utc_day":arcade::daily_limit(&tx)?}});
    tx.commit()?;
    Ok(axum::Json(result))
}

fn admin_view(db: &Connection, actor: i64, time: i64) -> ApiResult<Value> {
    let round = advance(db, time)?;
    let (mode, paused): (String, bool) = db.query_row(
        "SELECT mode,paused FROM gambling_config WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let luck_percent: i64 = db.query_row(
        "SELECT luck_percent FROM gambling_config WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    let queue = db
        .prepare("SELECT multiplier FROM gambling_crash_queue ORDER BY id")?
        .query_map([], |r| {
            Ok(json!({"crash_multiplier":r.get::<_,i64>(0)? as f64/100.0}))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let crash = crash_view(db, round.as_ref(), actor, time, true, 0)?;
    Ok(
        json!({"mode":mode,"paused":paused,"luck_percent":luck_percent,"queue":queue,"crash":crash,"planned_crash_multiplier":crash["planned_crash_multiplier"],"planned_crash_at_ms":crash["planned_crash_at_ms"],"server_time_ms":time,"notice":NOTICE,"rules":arcade::rules_view(db)?,"metrics":arcade::metrics(db)?,"limits":{"queued_rounds":20,"min_multiplier":1.0,"max_multiplier":1000.0},"queue_empty_behavior":"Controlled mode draws random rounds when its queue is empty."}),
    )
}

pub async fn admin_state(
    State(app): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    let actor = community::owner(&app, &headers)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let role: String = tx.query_row("SELECT role FROM users WHERE id=?1", [actor], |r| r.get(0))?;
    if role != "owner" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Owner access required"));
    }
    let result = admin_view(&tx, actor, milliseconds())?;
    tx.commit()?;
    Ok(axum::Json(result))
}

#[derive(Deserialize, Serialize)]
pub struct QueueInput {
    min_multiplier: f64,
    max_multiplier: f64,
    rounds: u32,
}
#[derive(Deserialize, Serialize)]
pub struct ConfigInput {
    mode: String,
    paused: bool,
    queue: Vec<QueueInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    luck_percent: Option<i64>,
}
pub async fn admin_config(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<ConfigInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = community::owner(&app, &headers)?;
    if !matches!(input.mode.as_str(), "random" | "controlled")
        || input.queue.len() > 20
        || input
            .luck_percent
            .is_some_and(|n| !(25..=10000).contains(&n))
    {
        return Err(bad(
            "Choose random or controlled mode, luck 25–10000 percent and at most 20 queued rounds",
        ));
    }
    let mut expanded = Vec::new();
    for group in &input.queue {
        let min = centi(group.min_multiplier)?;
        let max = centi(group.max_multiplier)?;
        if min > max
            || group.rounds == 0
            || group.rounds > 20
            || expanded.len() + group.rounds as usize > 20
        {
            return Err(bad(
                "Queue at most 20 rounds with valid increasing multiplier ranges",
            ));
        }
        for _ in 0..group.rounds {
            expanded.push(OsRng.gen_range(min..=max));
        }
    }
    if input.mode == "random" && !expanded.is_empty() {
        return Err(bad(
            "Switch to controlled mode to queue owner-selected outcomes",
        ));
    }
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let role: String = tx.query_row("SELECT role FROM users WHERE id=?1", [actor], |r| r.get(0))?;
    if role != "owner" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Owner access required"));
    }
    // Freeze any in-progress round first: configuration never alters accepted bets.
    advance(&tx, milliseconds())?;
    tx.execute(
        "UPDATE gambling_config SET mode=?1,paused=?2,luck_percent=COALESCE(?3,luck_percent) WHERE id=1",
        params![input.mode, input.paused, input.luck_percent],
    )?;
    tx.execute("DELETE FROM gambling_crash_queue", [])?;
    for multiplier in expanded {
        tx.execute(
            "INSERT INTO gambling_crash_queue(multiplier) VALUES(?1)",
            [multiplier],
        )?;
    }
    record(&tx, actor, "gambling_config", &json!(input))?;
    let mut result = admin_view(&tx, actor, milliseconds())?;
    result["ok"] = json!(true);
    tx.commit()?;
    Ok(axum::Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};

    #[test]
    fn crash_ceiling_migration_preserves_bets_queue_and_sequence() {
        let (_dir, app) = fixture();
        account(&app, "migration-player", false);
        let db = app.db.lock().unwrap();
        db.execute_batch("DROP TABLE gambling_crash_queue; DROP TABLE gambling_crash_rounds;
          CREATE TABLE gambling_crash_queue(id INTEGER PRIMARY KEY AUTOINCREMENT,multiplier INTEGER NOT NULL CHECK(multiplier BETWEEN 100 AND 10000));
          CREATE TABLE gambling_crash_rounds(id INTEGER PRIMARY KEY AUTOINCREMENT,created_ms INTEGER NOT NULL,start_ms INTEGER NOT NULL,crash_ms INTEGER NOT NULL,multiplier INTEGER NOT NULL CHECK(multiplier BETWEEN 100 AND 10000),mode TEXT NOT NULL CHECK(mode IN ('random','controlled')),settled INTEGER NOT NULL DEFAULT 0);
          INSERT INTO gambling_crash_queue VALUES(1,5000);
          INSERT INTO gambling_crash_rounds VALUES(42,0,1000,5000,5000,'controlled',0);
          INSERT INTO gambling_crash_rounds VALUES(1000,0,1000,5000,5000,'controlled',1);
          DELETE FROM gambling_crash_rounds WHERE id=1000;
          INSERT INTO gambling_crash_bets(round_id,user_id,stake,auto_multiplier) VALUES(42,1,25,200);") .unwrap();
        let foreign_keys: bool = db
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        migrate_crash_ceiling(&db).unwrap();
        migrate_crash_ceiling(&db).unwrap();
        assert_eq!(
            db.pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
                .unwrap(),
            foreign_keys
        );
        assert_eq!(bet_view(&db, 42, 1).unwrap()["stake"], 25);
        assert_eq!(
            db.query_row(
                "SELECT multiplier FROM gambling_crash_queue WHERE id=1",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            5000
        );
        db.execute("INSERT INTO gambling_crash_rounds(created_ms,start_ms,crash_ms,multiplier,mode) VALUES(0,1000,70000,100000,'controlled')",[]).unwrap();
        assert!(db.last_insert_rowid() > 1000);
        db.execute(
            "INSERT INTO gambling_crash_queue(multiplier) VALUES(100000)",
            [],
        )
        .unwrap();
        assert!(
            db.execute(
                "INSERT INTO gambling_crash_queue(multiplier) VALUES(100001)",
                []
            )
            .is_err()
        );
        assert_eq!(centi(1000.0).unwrap(), 100000);
        assert!(centi(1000.01).is_err());
        assert_eq!(
            db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
    }

    fn balance(app: &App, actor: i64, amount: i64) {
        let db = app.db.lock().unwrap();
        wallet(&db, actor).unwrap();
        db.execute(
            "UPDATE bot_wallets SET balance=?1 WHERE user_id=?2",
            params![amount, actor],
        )
        .unwrap();
    }

    fn round(db: &Connection, start: i64, multiplier: i64) -> Round {
        let crash = at_multiplier(start, multiplier);
        db.execute("INSERT INTO gambling_crash_rounds(created_ms,start_ms,crash_ms,multiplier,mode) VALUES(?1,?2,?3,?4,'controlled')",params![start-BETTING_MS,start,crash,multiplier]).unwrap();
        Round {
            id: db.last_insert_rowid(),
            start,
            crash,
            multiplier,
            mode: "controlled".into(),
        }
    }

    pub(super) fn hand(
        db: &Connection,
        actor: i64,
        player: &[u8],
        dealer: &[u8],
        next: &[u8],
    ) -> String {
        let id = Uuid::new_v4().to_string();
        db.execute(
            "INSERT INTO gambling_blackjack VALUES(?1,?2,100,?3,0,?4,?5,'playing',0,?6)",
            params![
                id,
                actor,
                json!(next).to_string(),
                json!(player).to_string(),
                json!(dealer).to_string(),
                now()
            ],
        )
        .unwrap();
        debit(db, actor, 100).unwrap();
        id
    }

    async fn bound_call(
        app: Shared,
        path: &str,
        body: Value,
        token: Option<&str>,
        member: &str,
    ) -> Response {
        use tower::ServiceExt;
        let mut request = axum::http::Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .header("x-canna-member", member);
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        router(app)
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn all_member_mutations_refuse_changed_or_invalid_identity_before_writes() {
        let (_dir, app) = fixture();
        let first = account(&app, "identity-first", false);
        let second = account(&app, "identity-second", false);
        balance(&app, 1, 1000);
        balance(&app, 2, 1000);
        let paths = [
            (
                "/api/v1/gambling/arcade/play",
                json!({"request_id":"bound-arcade","game":"slots","stake":25}),
            ),
            (
                "/api/v1/gambling/crash/bet",
                json!({"request_id":"bound-bet","round_id":1,"stake":25,"auto_cashout":null}),
            ),
            (
                "/api/v1/gambling/crash/cashout",
                json!({"request_id":"bound-cash","round_id":1}),
            ),
            (
                "/api/v1/gambling/blackjack/deal",
                json!({"request_id":"bound-deal","stake":25}),
            ),
            (
                "/api/v1/gambling/blackjack/action",
                json!({"request_id":"bound-hit","hand_id":"missing","action":"hit"}),
            ),
            (
                "/api/v1/gambling/cases/open",
                json!({"request_id":"bound-case","case_id":"avatar-frames"}),
            ),
            (
                "/api/v1/gambling/cosmetics/equip",
                json!({"frame":null,"banner":null}),
            ),
            (
                "/api/v1/gambling/daily",
                json!({"request_id":"bound-daily"}),
            ),
        ];
        for (path, body) in &paths {
            for expected in ["1", "", "-2", " 2", "2,1", "9223372036854775808"] {
                let response =
                    bound_call(app.clone(), path, body.clone(), Some(&second), expected).await;
                assert_eq!(
                    response.status(),
                    StatusCode::CONFLICT,
                    "{path} accepted {expected:?}"
                );
                assert!(
                    value(response).await["error"]
                        .as_str()
                        .unwrap()
                        .contains("reload")
                );
            }
            assert_eq!(
                bound_call(app.clone(), path, body.clone(), None, "1")
                    .await
                    .status(),
                StatusCode::UNAUTHORIZED
            );
        }
        {
            let db = app.db.lock().unwrap();
            for id in [1, 2] {
                assert_eq!(wallet(&db, id).unwrap().0, 1000);
            }
            for table in [
                "gambling_requests",
                "gambling_crash_rounds",
                "gambling_blackjack",
                "gambling_cosmetics",
                "gambling_equipped",
            ] {
                let count: i64 = db
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                    .unwrap();
                assert_eq!(count, 0, "identity refusal wrote {table}");
            }
        }
        // Matching identity keeps exact lost-reply replay semantics.
        let input = json!({"request_id":"same-member-retry","case_id":"avatar-frames"});
        let result = value(
            bound_call(
                app.clone(),
                "/api/v1/gambling/cases/open",
                input.clone(),
                Some(&first),
                "1",
            )
            .await,
        )
        .await;
        let retry = value(
            bound_call(
                app.clone(),
                "/api/v1/gambling/cases/open",
                input.clone(),
                Some(&first),
                "1",
            )
            .await,
        )
        .await;
        assert_eq!(result, retry);
        assert_eq!(
            bound_call(
                app.clone(),
                "/api/v1/gambling/cases/open",
                input,
                Some(&second),
                "1"
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        // Header-less older clients remain supported.
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/daily",
                json!({"request_id":"legacy-daily"}),
                Some(&second)
            )
            .await
            .status(),
            StatusCode::OK
        );
        let db = app.db.lock().unwrap();
        assert_eq!(wallet(&db, 1).unwrap().0, 900);
        assert_eq!(wallet(&db, 2).unwrap().0, 1100);
    }

    #[tokio::test]
    async fn dynamic_state_binds_member_identity_but_static_catalog_is_shared() {
        let (_dir, app) = fixture();
        let first = account(&app, "state-first", false);
        let second = account(&app, "state-second", false);
        let one = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/gambling",
                Value::Null,
                Some(&first),
            )
            .await,
        )
        .await;
        let two = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/gambling",
                Value::Null,
                Some(&second),
            )
            .await,
        )
        .await;
        assert_eq!(one["member_id"], 1);
        assert_eq!(two["member_id"], 2);
        let cursor = format!(
            "/api/v1/gambling?crash_round_id={}&crash_after_user_id=0",
            two["crash"]["id"]
        );
        let page = value(call(app.clone(), "GET", &cursor, Value::Null, Some(&second)).await).await;
        assert_eq!(page["member_id"], 2);
        assert!(page.get("wallet").is_none());
        let catalog = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/gambling/cosmetics/catalog",
                Value::Null,
                Some(&second),
            )
            .await,
        )
        .await;
        assert!(catalog.get("member_id").is_none());
        let mut headers = HeaderMap::new();
        headers.insert("authorization", format!("Bearer {second}").parse().unwrap());
        headers.append("x-canna-member", "2".parse().unwrap());
        headers.append("x-canna-member", "2".parse().unwrap());
        assert_eq!(
            mutation_actor(&app, &headers).unwrap_err().0,
            StatusCode::CONFLICT
        );
    }

    #[tokio::test]
    async fn catalog_cache_still_requires_current_membership_and_never_starts_games() {
        let (_dir, app) = fixture();
        let member = account(&app, "catalog-member", false);
        let other = account(&app, "catalog-other", false);
        let path = "/api/v1/gambling/cosmetics/catalog";
        assert_eq!(
            call(app.clone(), "GET", path, Value::Null, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let mut first = None;
        for token in [&member, &other, &member] {
            let response = call(app.clone(), "GET", path, Value::Null, Some(token)).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert_eq!(response.headers()["content-type"], "application/json");
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
            let result = value(response).await;
            assert_eq!(result["version"], cached_catalog().unwrap().version);
            assert_eq!(result["catalog"].as_array().unwrap().len(), 1000);
            assert_eq!(result["cases"].as_array().unwrap().len(), 9);
            assert!(result.get("owned").is_none());
            assert!(result.get("wallet").is_none());
            if let Some(previous) = first.as_ref() {
                assert_eq!(&result, previous);
            } else {
                first = Some(result);
            }
        }
        {
            let db = app.db.lock().unwrap();
            for table in ["gambling_crash_rounds", "bot_wallets", "gambling_requests"] {
                let count: i64 = db
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                    .unwrap();
                assert_eq!(count, 0, "catalog read changed {table}");
            }
            db.execute("UPDATE users SET banned=1 WHERE id=1", [])
                .unwrap();
            db.execute("UPDATE sessions SET expires=0 WHERE user_id=2", [])
                .unwrap();
        }
        for token in [&member, &other] {
            assert_eq!(
                call(app.clone(), "GET", path, Value::Null, Some(token))
                    .await
                    .status(),
                StatusCode::UNAUTHORIZED
            );
        }
    }

    #[tokio::test]
    async fn versioned_polls_reduce_payload_keep_game_state_and_refresh_stale_catalogs() {
        let (_dir, app) = fixture();
        let member = account(&app, "compact-gambler", false);
        balance(&app, 1, 1000);
        let legacy = value(
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
        let version = legacy["cosmetics"]["catalog_version"].as_str().unwrap();
        let full_size = serde_json::to_vec(&legacy).unwrap().len();
        assert!(full_size > 600_000);
        assert_eq!(
            legacy["cosmetics"]["catalog"].as_array().unwrap().len(),
            1000
        );
        assert!(legacy["cases"].is_array());
        let compact_path = format!("/api/v1/gambling?catalog_version={version}");
        for _ in 0..12 {
            let compact = value(
                call(
                    app.clone(),
                    "GET",
                    &compact_path,
                    Value::Null,
                    Some(&member),
                )
                .await,
            )
            .await;
            assert!(compact["cosmetics"].get("catalog").is_none());
            assert!(compact["cases"].is_null());
            assert_eq!(compact["cosmetics"]["catalog_version"], version);
            assert_eq!(compact["cosmetics"]["owned"], legacy["cosmetics"]["owned"]);
            assert_eq!(
                compact["cosmetics"]["equipped"],
                legacy["cosmetics"]["equipped"]
            );
            assert_eq!(compact["wallet"], legacy["wallet"]);
            assert_eq!(compact["crash"]["id"], legacy["crash"]["id"]);
            assert!(serde_json::to_vec(&compact).unwrap().len() < full_size / 100);
        }
        let stale_path = format!("/api/v1/gambling?catalog_version={}", "0".repeat(64));
        let stale =
            value(call(app.clone(), "GET", &stale_path, Value::Null, Some(&member)).await).await;
        assert_eq!(stale["cosmetics"]["catalog_version"], version);
        assert_eq!(
            stale["cosmetics"]["catalog"],
            legacy["cosmetics"]["catalog"]
        );
        assert_eq!(stale["cases"], legacy["cases"]);
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/gambling?catalog_version=bad",
                Value::Null,
                Some(&member)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );

        let input = json!({"request_id":"compact-crate","case_id":"avatar-frames"});
        let drop = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/cases/open",
                input.clone(),
                Some(&member),
            )
            .await,
        )
        .await;
        let repeated = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/cases/open",
                input,
                Some(&member),
            )
            .await,
        )
        .await;
        assert_eq!(drop, repeated);
        let equipped = call(
            app.clone(),
            "POST",
            "/api/v1/gambling/cosmetics/equip",
            json!({"frame":drop["item"]["id"],"banner":null}),
            Some(&member),
        )
        .await;
        assert_eq!(equipped.status(), StatusCode::OK);
        let compact = value(
            call(
                app.clone(),
                "GET",
                &compact_path,
                Value::Null,
                Some(&member),
            )
            .await,
        )
        .await;
        assert_eq!(compact["wallet"]["balance"], 900);
        assert_eq!(
            compact["cosmetics"]["owned"],
            json!([{"id":drop["item"]["id"],"count":1}])
        );
        assert_eq!(
            compact["cosmetics"]["equipped"]["frame"],
            drop["item"]["id"]
        );
        assert!(compact["cosmetics"].get("catalog").is_none());
        let fresh_legacy =
            value(call(app, "GET", "/api/v1/gambling", Value::Null, Some(&member)).await).await;
        assert_eq!(fresh_legacy["wallet"], compact["wallet"]);
        assert_eq!(
            fresh_legacy["cosmetics"]["owned"],
            compact["cosmetics"]["owned"]
        );
        assert_eq!(
            fresh_legacy["cosmetics"]["equipped"],
            compact["cosmetics"]["equipped"]
        );
    }

    #[test]
    fn catalog_fingerprint_changes_with_embedded_metadata_and_validates_before_caching() {
        let source = include_str!("../web/cosmetics/catalog.json");
        let current = build_catalog(source).unwrap();
        let mut changed: Value = serde_json::from_str(source).unwrap();
        changed["items"][0]["name"] = json!("A new catalog label");
        let newer = build_catalog(&changed.to_string()).unwrap();
        assert_ne!(newer.version, current.version);
        let response: Value = serde_json::from_slice(&newer.response).unwrap();
        assert_eq!(response["version"], newer.version);
        assert_eq!(response["catalog"][0]["name"], "A new catalog label");
        changed["items"][0]["collection"] = json!("client-created-pool");
        assert!(build_catalog(&changed.to_string()).is_err());
    }

    #[tokio::test]
    async fn game_endpoints_are_private_and_crash_controls_require_current_owner() {
        let (_dir, app) = fixture();
        let member = account(&app, "gambler", false);
        let owner = account(&app, "owner", true);
        for path in ["/api/v1/gambling", "/api/v1/admin/gambling"] {
            assert_eq!(
                call(app.clone(), "GET", path, Value::Null, None)
                    .await
                    .status(),
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/admin/gambling",
                Value::Null,
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE users SET role='admin' WHERE id=1", [])
            .unwrap();
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/admin/gambling",
                Value::Null,
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let result = value(
            call(
                app,
                "GET",
                "/api/v1/admin/gambling",
                Value::Null,
                Some(&owner),
            )
            .await,
        )
        .await;
        assert!(
            result["crash"]["planned_crash_multiplier"]
                .as_f64()
                .is_some()
        );
    }

    #[tokio::test]
    async fn public_crash_never_leaks_future_outcomes_or_queue() {
        let (_dir, app) = fixture();
        let member = account(&app, "gambler", false);
        {
            let db = app.db.lock().unwrap();
            round(&db, milliseconds() + BETTING_MS, 5000);
            db.execute(
                "INSERT INTO gambling_crash_queue(multiplier) VALUES(5000)",
                [],
            )
            .unwrap();
        }
        let result =
            value(call(app, "GET", "/api/v1/gambling", Value::Null, Some(&member)).await).await;
        assert_eq!(result["crash"]["mode"], "controlled");
        assert_eq!(result["crash"]["phase"], "betting");
        assert_eq!(result["crash"]["owner_visible"], true);
        for key in [
            "planned_crash_multiplier",
            "planned_crash_at_ms",
            "crashed_at_ms",
            "crash_multiplier",
            "queue",
        ] {
            assert!(result["crash"].get(key).is_none(), "leaked {key}");
            assert!(result.get(key).is_none());
        }
        assert_eq!(result["crash"]["history"], json!([]));
    }

    #[tokio::test]
    async fn crash_bet_and_cashout_requests_are_exactly_once_and_user_scoped() {
        let (_dir, app) = fixture();
        let a = account(&app, "alice", false);
        let b = account(&app, "bob", false);
        balance(&app, 1, 1000);
        balance(&app, 2, 1000);
        let r = {
            let db = app.db.lock().unwrap();
            round(&db, milliseconds() + BETTING_MS, 1000)
        };
        let bet = json!({"request_id":"bet-once","round_id":r.id,"stake":100});
        let first = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/crash/bet",
                bet.clone(),
                Some(&a),
            )
            .await,
        )
        .await;
        let retry = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/crash/bet",
                bet.clone(),
                Some(&a),
            )
            .await,
        )
        .await;
        assert_eq!(first, retry);
        assert_eq!(first["balance"], 900);
        let changed = json!({"request_id":"bet-once","round_id":r.id,"stake":101});
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/crash/bet",
                changed,
                Some(&a)
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/crash/bet",
                bet,
                Some(&b)
            )
            .await
            .status(),
            StatusCode::OK
        );
        {
            let db = app.db.lock().unwrap();
            let start = milliseconds() - 3000;
            db.execute(
                "UPDATE gambling_crash_rounds SET start_ms=?1,crash_ms=?2 WHERE id=?3",
                params![start, at_multiplier(start, 1000), r.id],
            )
            .unwrap();
        }
        let cashout = json!({"request_id":"cash-once","round_id":r.id});
        let first = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/crash/cashout",
                cashout.clone(),
                Some(&a),
            )
            .await,
        )
        .await;
        let retry = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/crash/cashout",
                cashout,
                Some(&a),
            )
            .await,
        )
        .await;
        assert_eq!(first, retry);
        assert_eq!(first["bet"]["status"], "won");
        let db = app.db.lock().unwrap();
        assert_eq!(
            wallet(&db, 1).unwrap().0,
            900 + first["bet"]["payout"].as_i64().unwrap()
        );
        assert_eq!(wallet(&db, 2).unwrap().0, 900);
        assert_eq!(bet_view(&db, r.id, 2).unwrap()["status"], "pending");
    }

    #[test]
    fn legacy_wager_tables_preserve_games_indexes_and_foreign_keys() {
        let (_dir, app) = fixture();
        account(&app, "old-wager", false);
        account(&app, "large-wager", false);
        balance(&app, 2, 3_000_000);
        let db = app.db.lock().unwrap();
        let r = round(&db, 100_000, 200);
        for (name, old_limit) in [
            ("gambling_crash_bets", 1000000),
            ("gambling_blackjack", 2000000),
        ] {
            let sql: String = db
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE name=?1",
                    [name],
                    |row| row.get(0),
                )
                .unwrap();
            db.execute_batch(&format!(
                "DROP TABLE {name}; {}",
                sql.replace(
                    "stake BETWEEN 1 AND 9007199254740991",
                    &format!("stake BETWEEN 1 AND {old_limit}")
                )
            ))
            .unwrap();
        }
        db.execute_batch("CREATE UNIQUE INDEX gambling_one_active_hand ON gambling_blackjack(user_id) WHERE status='playing'; INSERT INTO gambling_blackjack VALUES('old-shoe',1,1000000,'[0,1,2,3]',2,'[0,1]','[2,3]','playing',0,123);").unwrap();
        db.execute(
            "INSERT INTO gambling_crash_bets(round_id,user_id,stake) VALUES(?1,1,1000000)",
            [r.id],
        )
        .unwrap();
        let before = bet_view(&db, r.id, 1).unwrap();
        let keys: bool = db
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        initialize(&db).unwrap();
        initialize(&db).unwrap();
        assert_eq!(bet_view(&db, r.id, 1).unwrap(), before);
        assert_eq!(
            db.query_row(
                "SELECT deck FROM gambling_blackjack WHERE id='old-shoe'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "[0,1,2,3]"
        );
        assert_eq!(
            db.pragma_query_value(None, "foreign_keys", |row| row.get::<_, bool>(0))
                .unwrap(),
            keys
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            0
        );
        assert!(db.execute("INSERT INTO gambling_blackjack SELECT 'duplicate',user_id,stake,deck,cursor,player,dealer,status,payout,created FROM gambling_blackjack WHERE id='old-shoe'",[]).is_err());
        debit(&db, 2, 2_000_000).unwrap();
        db.execute(
            "INSERT INTO gambling_crash_bets(round_id,user_id,stake) VALUES(?1,2,2000000)",
            [r.id],
        )
        .unwrap();
        assert_eq!(bet_view(&db, r.id, 2).unwrap()["stake"], 2_000_000);
        db.execute("INSERT INTO gambling_blackjack VALUES('large-shoe',2,2500000,'[0,1,2,3]',2,'[0,1]','[2,3]','playing',0,123)",[]).unwrap();
        assert_eq!(
            db.query_row("SELECT luck_percent FROM gambling_config", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            100
        );
    }

    #[test]
    fn maximum_balance_crash_return_does_not_overflow_or_credit_twice() {
        let (_dir, app) = fixture();
        account(&app, "big-cap", false);
        balance(&app, 1, MAX_STAKE);
        let db = app.db.lock().unwrap();
        db.execute("UPDATE gambling_config SET paused=1", [])
            .unwrap();
        let r = round(&db, 100_000, MAX_MULTIPLIER);
        debit(&db, 1, MAX_STAKE).unwrap();
        db.execute("INSERT INTO gambling_crash_bets(round_id,user_id,stake,auto_multiplier) VALUES(?1,1,?2,?3)",params![r.id,MAX_STAKE,MAX_MULTIPLIER]).unwrap();
        advance(&db, r.crash + 1000).unwrap();
        assert_eq!(wallet(&db, 1).unwrap().0, MAX_STAKE);
        assert_eq!(bet_view(&db, r.id, 1).unwrap()["payout"], MAX_STAKE);
        advance(&db, r.crash + 2000).unwrap();
        assert_eq!(wallet(&db, 1).unwrap().0, MAX_STAKE);
    }

    #[test]
    fn luck_distribution_is_monotone_restores_baseline_and_caps() {
        for i in 0..10000 {
            let u = i as f64 / 10000.0;
            let baseline = (97.0 / (1.0 - u))
                .floor()
                .clamp(100.0, MAX_MULTIPLIER as f64) as i64;
            assert_eq!(multiplier_for_sample(u, 100), baseline);
            let mut previous = 100;
            for luck in [25, 100, 200, 500, 2500, 10000] {
                let value = multiplier_for_sample(u, luck);
                assert!((100..=MAX_MULTIPLIER).contains(&value));
                assert!(value >= previous);
                previous = value;
            }
        }
        assert_eq!(multiplier_for_sample(0.0, 10000), 9700);
        assert_eq!(multiplier_for_sample(0.99999, 10000), MAX_MULTIPLIER);
    }

    #[tokio::test]
    async fn luck_controls_are_owner_only_bounded_and_preserve_existing_round() {
        let (_dir, app) = fixture();
        let owner = account(&app, "luck-owner", true);
        let member = account(&app, "luck-member", false);
        let original = {
            let db = app.db.lock().unwrap();
            round(&db, milliseconds() + BETTING_MS, 200)
        };
        let config = json!({"mode":"random","paused":false,"queue":[],"luck_percent":10000});
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/gambling",
                config.clone(),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let reply = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/gambling",
                config,
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(reply["luck_percent"], 10000);
        assert_eq!(reply["planned_crash_multiplier"], 2.0);
        for invalid in [24, 10001] {
            assert_eq!(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/admin/gambling",
                    json!({"mode":"random","paused":false,"queue":[],"luck_percent":invalid}),
                    Some(&owner)
                )
                .await
                .status(),
                StatusCode::BAD_REQUEST
            );
        }
        // Older clients omit the field without resetting the selected boost.
        let legacy = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/gambling",
                json!({"mode":"random","paused":false,"queue":[]}),
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(legacy["luck_percent"], 10000);
        let db = app.db.lock().unwrap();
        assert_eq!(latest_round(&db).unwrap().unwrap().crash, original.crash);
        let next = advance(&db, original.crash + INTERMISSION_MS)
            .unwrap()
            .unwrap();
        assert!(next.multiplier >= 9700);
        assert!(
            crash_view(&db, Some(&next), 2, next.start, false, 0)
                .unwrap()
                .get("luck_percent")
                .is_none()
        );
    }

    #[test]
    fn crash_maximum_auto_target_wins_at_cap_and_after_delayed_poll_once() {
        let (_dir, app) = fixture();
        account(&app, "cap-auto", false);
        account(&app, "cap-manual", false);
        balance(&app, 1, 1000);
        balance(&app, 2, 1000);
        let db = app.db.lock().unwrap();
        db.execute("UPDATE gambling_config SET paused=1", [])
            .unwrap();
        let r = round(&db, 100_000, MAX_MULTIPLIER);
        for (actor, auto) in [(1, Some(MAX_MULTIPLIER)), (2, None)] {
            debit(&db, actor, 25).unwrap();
            db.execute("INSERT INTO gambling_crash_bets(round_id,user_id,stake,auto_multiplier) VALUES(?1,?2,25,?3)", params![r.id,actor,auto]).unwrap();
        }
        advance(&db, r.crash - 1).unwrap();
        assert_eq!(bet_view(&db, r.id, 1).unwrap()["status"], "pending");
        // No request at the deadline: a later server update still pays the target.
        advance(&db, r.crash + 2000).unwrap();
        let bet = bet_view(&db, r.id, 1).unwrap();
        assert_eq!(bet["status"], "won");
        assert_eq!(bet["cashout_multiplier"], 1000.0);
        assert_eq!(bet["cashout_at_ms"], r.crash);
        assert_eq!(bet["payout"], 25_000);
        assert_eq!(wallet(&db, 1).unwrap().0, 25_975);
        assert_eq!(bet_view(&db, r.id, 2).unwrap()["status"], "lost");
        advance(&db, r.crash + INTERMISSION_MS + 1).unwrap();
        assert_eq!(wallet(&db, 1).unwrap().0, 25_975);
        // A 1000x target is still lost when the round stops below the ceiling.
        let below = round(&db, 300_000, MAX_MULTIPLIER - 100);
        debit(&db, 1, 25).unwrap();
        db.execute("INSERT INTO gambling_crash_bets(round_id,user_id,stake,auto_multiplier) VALUES(?1,1,25,?2)", params![below.id,MAX_MULTIPLIER]).unwrap();
        advance(&db, below.crash + 1000).unwrap();
        assert_eq!(bet_view(&db, below.id, 1).unwrap()["status"], "lost");
        assert_eq!(wallet(&db, 1).unwrap().0, 25_950);
    }
    #[test]
    fn crash_clock_boundary_and_auto_cashout_settle_once() {
        let (_dir, app) = fixture();
        account(&app, "automatic", false);
        account(&app, "late", false);
        balance(&app, 1, 1000);
        balance(&app, 2, 1000);
        let db = app.db.lock().unwrap();
        db.execute("UPDATE gambling_config SET paused=1", [])
            .unwrap();
        let r = round(&db, 100_000, 300);
        for (actor, auto) in [(1, Some(200)), (2, Some(300))] {
            debit(&db, actor, 100).unwrap();
            db.execute("INSERT INTO gambling_crash_bets(round_id,user_id,stake,auto_multiplier) VALUES(?1,?2,100,?3)",params![r.id,actor,auto]).unwrap();
        }
        advance(&db, at_multiplier(r.start, 200) - 1).unwrap();
        assert_eq!(wallet(&db, 1).unwrap().0, 900);
        advance(&db, at_multiplier(r.start, 200)).unwrap();
        assert_eq!(wallet(&db, 1).unwrap().0, 1100);
        advance(&db, r.crash - 1).unwrap();
        assert_eq!(bet_view(&db, r.id, 2).unwrap()["status"], "pending");
        advance(&db, r.crash).unwrap();
        assert_eq!(bet_view(&db, r.id, 2).unwrap()["status"], "lost");
        advance(&db, r.crash + INTERMISSION_MS + 1).unwrap();
        assert_eq!(wallet(&db, 1).unwrap().0, 1100);
        assert_eq!(wallet(&db, 2).unwrap().0, 900);
        assert!(latest_round(&db).unwrap().is_some());
    }

    #[test]
    fn crash_participants_persist_through_crash_and_pause_then_clear_at_next_round() {
        let (_dir, app) = fixture();
        for name in ["manual", "automatic", "lost", "hidden"] {
            account(&app, name, false);
        }
        for actor in 1..=4 {
            balance(&app, actor, 1000);
        }
        let db = app.db.lock().unwrap();
        db.execute("UPDATE gambling_config SET paused=1", [])
            .unwrap();
        let r = round(&db, 100_000, 300);
        for (actor, auto) in [(1, None), (2, Some(200)), (3, Some(300)), (4, None)] {
            debit(&db, actor, 100).unwrap();
            db.execute("INSERT INTO gambling_crash_bets(round_id,user_id,stake,auto_multiplier) VALUES(?1,?2,100,?3)",params![r.id,actor,auto]).unwrap();
        }
        db.execute(
            "UPDATE users SET banned=1,email='private@example.test' WHERE id=4",
            [],
        )
        .unwrap();
        let before = crash_view(&db, Some(&r), 3, r.start - 1, false, 0).unwrap();
        assert_eq!(before["participant_count"], 4);
        assert!(
            before["participants"]
                .as_array()
                .unwrap()
                .iter()
                .all(|p| p["status"] == "pending")
        );
        assert!(before.get("planned_crash_multiplier").is_none());
        assert!(before.get("planned_crash_at_ms").is_none());
        assert_eq!(before["participants"][0]["username"], "manual");
        assert_eq!(before["participants"][0]["profile_url"], "/members/1");
        let hidden = &before["participants"][3];
        assert_eq!(hidden["display_name"], "Unavailable member");
        assert!(hidden["username"].is_null() && hidden["profile_url"].is_null());
        for p in before["participants"].as_array().unwrap() {
            assert_eq!(p.as_object().unwrap().len(), 9);
            for private in [
                "email",
                "wallet",
                "balance",
                "auto_cashout",
                "auto_multiplier",
                "payout",
                "password",
                "token",
                "planned_crash_at_ms",
            ] {
                assert!(p.get(private).is_none(), "participant leaked {private}");
            }
        }
        let manual_at = r.start + 1850;
        settle_bet(&db, r.id, 1, Some(120), manual_at).unwrap();
        // A delayed poll still records the automatic threshold crossing, not poll time.
        advance(&db, r.crash).unwrap();
        let settled = crash_view(&db, Some(&r), 3, r.crash, false, 0).unwrap();
        assert_eq!(settled["participants"][0]["cashout_multiplier"], 1.2);
        assert_eq!(settled["participants"][0]["cashout_at_ms"], manual_at);
        assert_eq!(settled["participants"][0]["cashout_elapsed_ms"], 1850);
        assert_eq!(settled["participants"][1]["status"], "won");
        assert_eq!(
            settled["participants"][1]["cashout_at_ms"],
            at_multiplier(r.start, 200)
        );
        assert_eq!(
            settled["participants"][1]["cashout_elapsed_ms"],
            at_multiplier(0, 200)
        );
        for index in [2, 3] {
            assert_eq!(settled["participants"][index]["status"], "lost");
            assert!(settled["participants"][index]["cashout_at_ms"].is_null());
        }
        let time = r.crash + INTERMISSION_MS + 1;
        assert!(advance(&db, time).unwrap().is_none());
        let paused = crash_view(&db, None, 3, time, false, 0).unwrap();
        assert_eq!(paused["id"], r.id);
        assert_eq!(paused["phase"], "crashed");
        assert_eq!(paused["paused"], true);
        assert_eq!(paused["participants"], settled["participants"]);
        db.execute("UPDATE gambling_config SET paused=0", [])
            .unwrap();
        let next = advance(&db, time).unwrap().unwrap();
        let fresh = crash_view(&db, Some(&next), 3, time, false, 0).unwrap();
        assert_ne!(fresh["id"], r.id);
        assert_eq!(fresh["participants"], json!([]));
        assert_eq!(fresh["participant_count"], 0);
        assert_eq!(
            participant_view(&db, &r, 0).unwrap()["participants"],
            settled["participants"]
        );
        assert_eq!(wallet(&db, 1).unwrap().0, 1020);
        assert_eq!(wallet(&db, 2).unwrap().0, 1100);
    }

    #[tokio::test]
    async fn crash_participant_pages_are_complete_bounded_and_pinned_to_current_round() {
        let (_dir, app) = fixture();
        let viewer = account(&app, "viewer", false);
        let r = {
            let mut db = app.db.lock().unwrap();
            let tx = db.transaction().unwrap();
            let r = round(&tx, milliseconds() + 60_000, 5000);
            for index in 0..205 {
                tx.execute("INSERT INTO users(username,password,verified,role) VALUES(?1,'fixture',1,'member')",[format!("participant-{index}")]).unwrap();
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO gambling_crash_bets(round_id,user_id,stake) VALUES(?1,?2,?3)",
                    params![r.id, id, index + 1],
                )
                .unwrap();
            }
            tx.commit().unwrap();
            r
        };
        let page_path = format!(
            "/api/v1/gambling?crash_round_id={}&crash_after_user_id=0",
            r.id
        );
        let page_only =
            value(call(app.clone(), "GET", &page_path, Value::Null, Some(&viewer)).await).await;
        assert_eq!(page_only.as_object().unwrap().len(), 3);
        assert_eq!(page_only["member_id"], 1);
        assert!(page_only["server_time_ms"].as_i64().is_some());
        assert_eq!(page_only["crash"]["participant_count"], 205);
        assert_eq!(
            page_only["crash"]["participants"].as_array().unwrap().len(),
            200
        );
        for omitted in [
            "wallet",
            "balance",
            "blackjack",
            "cosmetics",
            "catalog",
            "cases",
        ] {
            assert!(
                page_only.get(omitted).is_none(),
                "paging repeated {omitted}"
            );
        }
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM bot_wallets WHERE user_id=1",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        let first = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/gambling",
                Value::Null,
                Some(&viewer),
            )
            .await,
        )
        .await;
        assert!(first["wallet"].is_object());
        assert!(first["cosmetics"]["catalog"].is_array());
        assert!(first["cases"].is_array());
        let first = &first["crash"];
        assert_eq!(first["participant_count"], 205);
        assert_eq!(first["participants"].as_array().unwrap().len(), 200);
        assert_eq!(first["participant_has_more"], true);
        let after = first["participant_next_after_user_id"].as_i64().unwrap();
        let path = format!(
            "/api/v1/gambling?crash_round_id={}&crash_after_user_id={after}",
            r.id
        );
        let second = value(call(app.clone(), "GET", &path, Value::Null, Some(&viewer)).await).await;
        assert_eq!(second.as_object().unwrap().len(), 3);
        assert_eq!(second["member_id"], 1);
        let second = &second["crash"];
        assert_eq!(second["participant_count"], 205);
        assert_eq!(second["participants"].as_array().unwrap().len(), 5);
        assert_eq!(second["participant_has_more"], false);
        assert!(second["participant_next_after_user_id"].is_null());
        let ids = first["participants"]
            .as_array()
            .unwrap()
            .iter()
            .chain(second["participants"].as_array().unwrap())
            .map(|p| p["user_id"].as_i64().unwrap())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(ids.len(), 205);
        for path in [
            "/api/v1/gambling?crash_after_user_id=1".to_owned(),
            format!(
                "/api/v1/gambling?crash_round_id={}&crash_after_user_id=-1",
                r.id
            ),
        ] {
            assert_eq!(
                call(app.clone(), "GET", &path, Value::Null, Some(&viewer))
                    .await
                    .status(),
                StatusCode::BAD_REQUEST
            );
        }
        let stale = format!(
            "/api/v1/gambling?crash_round_id={}&crash_after_user_id={after}",
            r.id + 1
        );
        assert_eq!(
            call(app.clone(), "GET", &stale, Value::Null, Some(&viewer))
                .await
                .status(),
            StatusCode::CONFLICT
        );
    }

    #[test]
    fn crash_cashout_clock_migration_is_idempotent_without_fabricating_history() {
        let (_dir, app) = fixture();
        account(&app, "legacy-winner", false);
        let db = app.db.lock().unwrap();
        let r = round(&db, 100_000, 300);
        db.execute_batch("DROP TABLE gambling_crash_bets; CREATE TABLE gambling_crash_bets(round_id INTEGER,user_id INTEGER,stake INTEGER,auto_multiplier INTEGER,status TEXT,payout INTEGER,cashout_multiplier INTEGER,PRIMARY KEY(round_id,user_id));").unwrap();
        db.execute(
            "INSERT INTO gambling_crash_bets VALUES(?1,1,100,NULL,'won',200,200)",
            [r.id],
        )
        .unwrap();
        initialize(&db).unwrap();
        initialize(&db).unwrap();
        let bet = bet_view(&db, r.id, 1).unwrap();
        assert_eq!(bet["status"], "won");
        assert_eq!(bet["cashout_multiplier"], 2.0);
        assert!(bet["cashout_at_ms"].is_null());
        let participants = participant_view(&db, &r, 0).unwrap();
        assert!(participants["participants"][0]["cashout_at_ms"].is_null());
        assert!(participants["participants"][0]["cashout_elapsed_ms"].is_null());
    }

    #[tokio::test]
    async fn crash_cashout_transaction_rolls_back_wallet_and_participant_clock_together() {
        let (_dir, app) = fixture();
        let auth = account(&app, "atomic-cashout", false);
        balance(&app, 1, 1000);
        let r = {
            let db = app.db.lock().unwrap();
            let r = round(&db, milliseconds() - 3000, 1000);
            debit(&db, 1, 100).unwrap();
            db.execute(
                "INSERT INTO gambling_crash_bets(round_id,user_id,stake) VALUES(?1,1,100)",
                [r.id],
            )
            .unwrap();
            db.execute_batch("CREATE TRIGGER fail_cashout BEFORE UPDATE ON gambling_crash_bets WHEN NEW.status='won' BEGIN SELECT RAISE(ABORT,'fixture cashout failure'); END;").unwrap();
            r
        };
        let input = json!({"request_id":"atomic-cashout","round_id":r.id});
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/crash/cashout",
                input.clone(),
                Some(&auth)
            )
            .await
            .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        {
            let db = app.db.lock().unwrap();
            assert_eq!(wallet(&db, 1).unwrap().0, 900);
            let bet = bet_view(&db, r.id, 1).unwrap();
            assert_eq!(bet["status"], "pending");
            assert!(bet["cashout_at_ms"].is_null());
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM gambling_requests WHERE request_id='atomic-cashout'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            db.execute_batch("DROP TRIGGER fail_cashout;").unwrap();
        }
        let success = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/crash/cashout",
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
                "/api/v1/gambling/crash/cashout",
                input,
                Some(&auth),
            )
            .await,
        )
        .await;
        assert_eq!(success, replay);
        assert_eq!(success["bet"]["status"], "won");
        assert!(success["bet"]["cashout_at_ms"].as_i64().unwrap() >= r.start);
        let db = app.db.lock().unwrap();
        let row = &participant_view(&db, &r, 0).unwrap()["participants"][0];
        assert_eq!(row["cashout_at_ms"], success["bet"]["cashout_at_ms"]);
    }

    #[tokio::test]
    async fn crash_stale_cashouts_never_pay_or_join_the_new_round_participants() {
        let (_dir, app) = fixture();
        let auth = account(&app, "late-cashout", false);
        balance(&app, 1, 1000);
        let old = {
            let db = app.db.lock().unwrap();
            let r = round(&db, milliseconds() - 30_000, 200);
            debit(&db, 1, 100).unwrap();
            db.execute(
                "INSERT INTO gambling_crash_bets(round_id,user_id,stake) VALUES(?1,1,100)",
                [r.id],
            )
            .unwrap();
            r
        };
        let state = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/gambling",
                Value::Null,
                Some(&auth),
            )
            .await,
        )
        .await;
        let next = state["crash"]["id"].as_i64().unwrap();
        assert_ne!(next, old.id);
        assert_eq!(state["crash"]["participants"], json!([]));
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/crash/bet",
                json!({"request_id":"new-round-bet","round_id":next,"stake":50}),
                Some(&auth)
            )
            .await
            .status(),
            StatusCode::OK
        );
        let input = json!({"request_id":"stale-cashout","round_id":old.id});
        let stale = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/crash/cashout",
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
                "/api/v1/gambling/crash/cashout",
                input,
                Some(&auth),
            )
            .await,
        )
        .await;
        assert_eq!(stale, replay);
        assert_eq!(stale["bet"]["status"], "lost");
        assert_eq!(stale["balance"], 850);
        let state = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/gambling",
                Value::Null,
                Some(&auth),
            )
            .await,
        )
        .await;
        assert_eq!(state["crash"]["id"], next);
        assert_eq!(state["crash"]["participant_count"], 1);
        assert_eq!(state["crash"]["participants"][0]["stake"], 50);
        assert_eq!(state["crash"]["participants"][0]["status"], "pending");
        assert!(state["crash"]["participants"][0]["cashout_at_ms"].is_null());
    }

    #[tokio::test]
    async fn crash_pause_rejects_new_bets_but_preserves_paid_cashouts_and_replay() {
        let (_dir, app) = fixture();
        let owner = account(&app, "owner", true);
        let existing = account(&app, "existing", false);
        let newcomer = account(&app, "newcomer", false);
        balance(&app, 2, 1000);
        balance(&app, 3, 1000);
        let r = {
            let db = app.db.lock().unwrap();
            round(&db, milliseconds() + BETTING_MS, 1000)
        };
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/crash/bet",
                json!({"request_id":"paid-before-pause","round_id":r.id,"stake":100}),
                Some(&existing)
            )
            .await
            .status(),
            StatusCode::OK
        );
        let paused = value(call(app.clone(), "POST", "/api/v1/admin/gambling", json!({"mode":"controlled","paused":true,"queue":[{"min_multiplier":50.0,"max_multiplier":50.0,"rounds":2}]}), Some(&owner)).await).await;
        assert_eq!(paused["paused"], true);
        assert_eq!(paused["queue"].as_array().unwrap().len(), 2);
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/crash/bet",
                json!({"request_id":"denied-after-pause","round_id":r.id,"stake":100}),
                Some(&newcomer)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        {
            let db = app.db.lock().unwrap();
            let start = milliseconds() - 3000;
            db.execute(
                "UPDATE gambling_crash_rounds SET start_ms=?1,crash_ms=?2 WHERE id=?3",
                params![start, at_multiplier(start, 1000), r.id],
            )
            .unwrap();
        }
        let input = json!({"request_id":"paid-cashout-while-paused","round_id":r.id});
        let first = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/crash/cashout",
                input.clone(),
                Some(&existing),
            )
            .await,
        )
        .await;
        let retry = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/crash/cashout",
                input,
                Some(&existing),
            )
            .await,
        )
        .await;
        assert_eq!(first, retry);
        assert_eq!(first["bet"]["status"], "won");
        let db = app.db.lock().unwrap();
        assert_eq!(
            wallet(&db, 2).unwrap().0,
            900 + first["bet"]["payout"].as_i64().unwrap()
        );
        assert_eq!(wallet(&db, 3).unwrap().0, 1000);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM gambling_crash_bets WHERE user_id=3",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        let current = latest_round(&db).unwrap().unwrap();
        assert!(
            advance(&db, current.crash + INTERMISSION_MS)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM gambling_crash_queue", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn queued_controlled_rounds_are_bounded_and_never_rewrite_active_round() {
        let (_dir, app) = fixture();
        let owner = account(&app, "owner", true);
        let original = {
            let db = app.db.lock().unwrap();
            round(&db, milliseconds() + BETTING_MS, 200)
        };
        let config = json!({"mode":"controlled","paused":false,"queue":[{"min_multiplier":50.0,"max_multiplier":50.0,"rounds":3}]});
        let response = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/gambling",
                config,
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(response["queue"].as_array().unwrap().len(), 3);
        assert_eq!(response["planned_crash_multiplier"], 2.0);
        let excessive = json!({"mode":"controlled","paused":false,"queue":[{"min_multiplier":1.0,"max_multiplier":1000.0,"rounds":21}]});
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/gambling",
                excessive,
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        let db = app.db.lock().unwrap();
        let next = advance(&db, original.crash + INTERMISSION_MS)
            .unwrap()
            .unwrap();
        assert_eq!(next.multiplier, 5000);
        assert_eq!(next.mode, "controlled");
        assert_eq!(
            db.query_row("SELECT count(*) FROM gambling_crash_queue", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
        let before = latest_round(&db).unwrap().unwrap();
        db.execute("UPDATE gambling_config SET mode='random'", [])
            .unwrap();
        assert_eq!(
            advance(&db, before.start).unwrap().unwrap().multiplier,
            5000
        );
    }

    #[test]
    fn blackjack_totals_soft_aces_and_natural_settlement_are_exact() {
        assert_eq!(total(&[0, 13, 9]), 12);
        assert_eq!(total(&[0, 5]), 17);
        let (_dir, app) = fixture();
        account(&app, "natural", false);
        balance(&app, 1, 1000);
        let db = app.db.lock().unwrap();
        let id = hand(&db, 1, &[0, 9], &[8, 7], &[]);
        let mut hand = load_hand(&db, &id, 1).unwrap();
        finish_hand(&db, &mut hand, true).unwrap();
        save_hand(&db, &hand).unwrap();
        assert_eq!(hand.status, "blackjack");
        assert_eq!(hand.payout, 250);
        finish_hand(&db, &mut hand, true).unwrap();
        assert_eq!(wallet(&db, 1).unwrap().0, 1150);
        assert_eq!(hand_view(&hand)["dealer_total"], 17);
    }

    #[test]
    fn stored_blackjack_cursor_is_checked_before_native_conversion_and_draw() {
        let (_dir, app) = fixture();
        account(&app, "cursor", false);
        balance(&app, 1, 1000);
        let db = app.db.lock().unwrap();
        let id = hand(&db, 1, &[4, 5], &[8, 7], &[9]);
        for cursor in [-1, 2, 53, i64::MAX] {
            db.execute(
                "UPDATE gambling_blackjack SET cursor=?1 WHERE id=?2",
                params![cursor, id],
            )
            .unwrap();
            assert!(
                load_hand(&db, &id, 1).is_err(),
                "accepted invalid cursor {cursor}"
            );
        }
        db.execute("UPDATE gambling_blackjack SET cursor=1 WHERE id=?1", [&id])
            .unwrap();
        let mut exhausted = load_hand(&db, &id, 1).unwrap();
        assert_eq!(exhausted.cursor, 1);
        assert!(draw(&mut exhausted, false).is_err());
        db.execute("UPDATE gambling_blackjack SET cursor=0 WHERE id=?1", [&id])
            .unwrap();
        let mut valid = load_hand(&db, &id, 1).unwrap();
        draw(&mut valid, false).unwrap();
        assert_eq!(valid.player, vec![4, 5, 9]);
        assert_eq!(valid.cursor, 1);
    }

    #[tokio::test]
    async fn blackjack_hidden_cards_disconnect_resume_and_double_cannot_repeat() {
        let (_dir, app) = fixture();
        let a = account(&app, "alice", false);
        let b = account(&app, "bob", false);
        balance(&app, 1, 1000);
        balance(&app, 2, 1000);
        let id = {
            let db = app.db.lock().unwrap();
            hand(&db, 1, &[4, 5], &[8, 7], &[9])
        };
        let current = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/gambling",
                Value::Null,
                Some(&a),
            )
            .await,
        )
        .await;
        assert_eq!(current["blackjack"]["id"], id);
        assert_eq!(current["blackjack"]["dealer"][1], json!({"hidden":true}));
        assert!(current["blackjack"].get("dealer_total").is_none());
        assert!(current["blackjack"].get("deck").is_none());
        let action = json!({"request_id":"double-once","hand_id":id,"action":"double"});
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/blackjack/action",
                action.clone(),
                Some(&b)
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        let first = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/blackjack/action",
                action.clone(),
                Some(&a),
            )
            .await,
        )
        .await;
        let duplicate = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/blackjack/action",
                action,
                Some(&a),
            )
            .await,
        )
        .await;
        assert_eq!(first, duplicate);
        assert_eq!(first["blackjack"]["stake"], 200);
        assert_eq!(first["blackjack"]["status"], "won");
        assert_eq!(first["balance"], 1200);
        let second = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/blackjack/action",
                json!({"request_id":"double-again","hand_id":id,"action":"double"}),
                Some(&a),
            )
            .await,
        )
        .await;
        assert_eq!(second["balance"], 1200);
        assert_eq!(wallet(&app.db.lock().unwrap(), 2).unwrap().0, 1000);
    }

    #[tokio::test]
    async fn blackjack_deal_has_one_finite_unique_shoe_and_duplicate_deal_keeps_it() {
        let (_dir, app) = fixture();
        let auth = account(&app, "player", false);
        balance(&app, 1, 1000);
        let input = json!({"request_id":"deal-unique","stake":100});
        let first = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/blackjack/deal",
                input.clone(),
                Some(&auth),
            )
            .await,
        )
        .await;
        let retry = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/blackjack/deal",
                input,
                Some(&auth),
            )
            .await,
        )
        .await;
        assert_eq!(first, retry);
        let db = app.db.lock().unwrap();
        let h = load_hand(&db, first["blackjack"]["id"].as_str().unwrap(), 1).unwrap();
        assert_eq!(h.deck.len(), 52);
        assert_eq!(
            h.deck
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            52
        );
        assert_eq!(h.cursor, 4);
    }

    #[tokio::test]
    async fn approved_mw2_artwork_restores_saved_selection_and_crate_without_changing_ids() {
        let (_dir, app) = fixture();
        let auth = account(&app, "artwork-collector", false);
        balance(&app, 1, 1000);
        let catalog = cached_catalog().unwrap();
        let cards: Vec<_> = catalog.value["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["collection"] == "mw2")
            .collect();
        assert_eq!(cards.len(), 398);
        assert!(cards.iter().all(|item| item["paused"] != true));
        let a = cards[0]["id"].as_str().unwrap();
        let b = cards[1]["id"].as_str().unwrap();
        assert!(case_pool(&catalog.value, "mw2-calling-cards").unwrap().1 > 0);
        {
            let db = app.db.lock().unwrap();
            for id in [a, b, "frame-mint-halo"] {
                db.execute("INSERT INTO gambling_cosmetics VALUES(1,?1,2)", [id])
                    .unwrap();
            }
            db.execute(
                "INSERT INTO gambling_equipped(user_id,banner) VALUES(1,?1)",
                [a],
            )
            .unwrap();
            // A previously stored paused selection becomes visible with the
            // same identity, without an ownership/equipment migration.
            assert_eq!(equipped(&db, 1).unwrap()["banner"]["id"], a);
        }
        let input = json!({"request_id":"resumed-case","case_id":"mw2-calling-cards"});
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
        let retry = value(
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
        assert_eq!(first, retry);
        assert_eq!(first["item"]["collection"], "mw2");
        let response = call(
            app.clone(),
            "POST",
            "/api/v1/gambling/cosmetics/equip",
            json!({"frame":"frame-mint-halo","banner":b}),
            Some(&auth),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        {
            let db = app.db.lock().unwrap();
            assert_eq!(wallet(&db, 1).unwrap().0, 900);
            assert_eq!(
                db.query_row(
                    "SELECT sum(count) FROM gambling_cosmetics WHERE user_id=1",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                7
            );
            let profile = equipped(&db, 1).unwrap();
            assert_eq!(profile["banner"]["id"], b);
            assert_eq!(profile["frame"]["id"], "frame-mint-halo");
        }
        // Keep the collection-pause mechanism covered independently of the
        // currently approved catalog. A pause must retain entries and weights.
        let mut source: Value =
            serde_json::from_str(include_str!("../web/cosmetics/catalog.json")).unwrap();
        source["paused_collections"] = json!(["mw2"]);
        let paused = build_catalog(&source.to_string()).unwrap();
        assert_eq!(case_pool(&paused.value, "mw2-calling-cards").unwrap().1, 0);
        assert!(
            case_pool(&paused.value, "canna-case")
                .unwrap()
                .0
                .iter()
                .all(|item| item["collection"] != "mw2")
        );
        assert_ne!(paused.version, catalog.version);
        assert_eq!(
            paused.value["items"].as_array().unwrap().len(),
            source["items"].as_array().unwrap().len()
        );
    }

    #[tokio::test]
    async fn cases_show_full_odds_duplicate_request_awards_once_and_equip_checks_ownership() {
        let (_dir, app) = fixture();
        let a = account(&app, "collector", false);
        let b = account(&app, "other", false);
        balance(&app, 1, 1000);
        let overview = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/gambling",
                Value::Null,
                Some(&a),
            )
            .await,
        )
        .await;
        let sum: f64 = overview["cases"][0]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["odds_percent"].as_f64().unwrap())
            .sum();
        assert!((sum - 100.0).abs() < 0.000_001);
        let input = json!({"request_id":"open-once","case_id":"canna-case"});
        let first = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/cases/open",
                input.clone(),
                Some(&a),
            )
            .await,
        )
        .await;
        let retry = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/cases/open",
                input,
                Some(&a),
            )
            .await,
        )
        .await;
        assert_eq!(first, retry);
        assert_eq!(first["count"], 1);
        let mut equip = json!({"frame":null,"banner":null});
        equip[first["item"]["kind"].as_str().unwrap()] = first["item"]["id"].clone();
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/cosmetics/equip",
                equip.clone(),
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
                "/api/v1/gambling/cosmetics/equip",
                equip,
                Some(&a)
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
                Some(&a),
            )
            .await,
        )
        .await;
        assert_eq!(
            profile["cosmetics"][first["item"]["kind"].as_str().unwrap()]["id"],
            first["item"]["id"]
        );
        let metadata = equipped(&app.db.lock().unwrap(), 1).unwrap();
        assert_eq!(
            metadata[first["item"]["kind"].as_str().unwrap()]["id"],
            first["item"]["id"]
        );
    }

    #[test]
    fn separate_crate_pools_never_cross_collections_and_show_exact_odds() {
        let catalog = json!({"case":{"price":100},"items":[
            {"id":"bo2-a","collection":"bo2","kind":"banner","weight":2},
            {"id":"bo2-b","collection":"bo2","kind":"banner","weight":1},
            {"id":"mw2-a","collection":"mw2","kind":"banner","weight":3},
            {"id":"mw2-b","collection":"mw2","kind":"banner","weight":1},
            {"id":"frame-a","collection":"frames","kind":"frame","weight":2},
            {"id":"canna-a","collection":"canna","kind":"banner","weight":1},
            {"id":"bo2-zero","collection":"bo2","kind":"banner","weight":0},
            {"id":"wrong-kind","collection":"bo2","kind":"frame","weight":1},
            {"id":"emblem-a","collection":"mw2-emblems","kind":"emblem","weight":2},
            {"id":"effect-a","collection":"username-effects","kind":"name_effect","weight":1}
        ]});
        let cases = cases_view(&catalog).unwrap();
        assert_eq!(cases.as_array().unwrap().len(), 9);
        for case in cases.as_array().unwrap() {
            let id = case["id"].as_str().unwrap();
            let (pool, sum) = case_pool(&catalog, id).unwrap();
            assert_eq!(case["cost"], arcade::default_case_cost(id));
            if sum == 0 {
                assert_eq!(case["available"], false);
                continue;
            }
            assert_eq!(case["available"], true);
            assert_eq!(case["item_count"].as_u64().unwrap() as usize, pool.len());
            let odds: f64 = case["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["odds_percent"].as_f64().unwrap())
                .sum();
            assert!((odds - 100.0).abs() < 0.000_001);
            let mut counts = std::collections::HashMap::<String, u64>::new();
            for roll in 0..sum {
                let item = select_case_item(&catalog, id, roll).unwrap();
                if !case["collection"].is_null() {
                    assert_eq!(item["collection"], case["collection"]);
                }
                if case["kind"] != "all" {
                    assert_eq!(item["kind"], case["kind"]);
                }
                *counts
                    .entry(item["id"].as_str().unwrap().to_owned())
                    .or_default() += 1;
            }
            for item in pool {
                assert_eq!(
                    counts[item["id"].as_str().unwrap()],
                    item["weight"].as_u64().unwrap()
                );
            }
            assert!(select_case_item(&catalog, id, sum).is_err());
        }
        // The compatibility endpoint keeps its original mixed collection.
        let (legacy, _) = case_pool(&catalog, "canna-case").unwrap();
        assert!(legacy.iter().any(|i| i["collection"] == "canna"));
        assert!(legacy.iter().any(|i| i["collection"] == "frames"));
    }

    #[test]
    fn empty_unknown_or_invalid_crate_pools_fail_without_random_range_panics() {
        let empty = json!({"case":{"price":100},"items":[{"id":"frame-only","collection":"frames","kind":"frame","weight":1}]});
        let cases = cases_view(&empty).unwrap();
        assert_eq!(cases[0]["available"], false);
        assert_eq!(cases[0]["item_count"], 0);
        assert_eq!(cases[0]["items"], json!([]));
        assert!(select_case_item(&empty, "bo2-calling-cards", 0).is_err());
        assert!(case_pool(&empty, "client-chosen-pool").is_err());
        let excessive =
            json!({"items":[{"id":"bad","collection":"frames","kind":"frame","weight":u64::MAX}]});
        assert!(case_pool(&excessive, "avatar-frames").is_err());
        let invalid =
            json!({"items":[{"id":"bad","collection":"frames","kind":"frame","weight":-1}]});
        assert!(case_pool(&invalid, "avatar-frames").is_err());
    }

    #[tokio::test]
    async fn separate_crate_requests_are_server_selected_exactly_once_and_preserve_equipment() {
        let (_dir, app) = fixture();
        let auth = account(&app, "crate-collector", false);
        balance(&app, 1, 1000);
        {
            let db = app.db.lock().unwrap();
            db.execute(
                "INSERT INTO gambling_cosmetics VALUES(1,'frame-mint-halo',1)",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO gambling_equipped(user_id,frame) VALUES(1,'frame-mint-halo')",
                [],
            )
            .unwrap();
        }
        for (index, (id, collection, kind)) in [
            ("bo2-calling-cards", "bo2", "banner"),
            ("avatar-frames", "frames", "frame"),
        ]
        .into_iter()
        .enumerate()
        {
            let input = json!({"request_id":format!("crate-{index}"),"case_id":id});
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
            assert_eq!(first["case_id"], id);
            assert_eq!(first["item"]["collection"], collection);
            assert_eq!(first["item"]["kind"], kind);
            assert_eq!(first["balance"], 1000 - ((index + 1) * 100) as i64);
            let db = app.db.lock().unwrap();
            assert_eq!(equipped(&db, 1).unwrap()["frame"]["id"], "frame-mint-halo");
            assert!(equipped(&db, 1).unwrap()["banner"].is_null());
            assert_eq!(
                db.query_row(
                    "SELECT count FROM gambling_cosmetics WHERE user_id=1 AND item_id=?1",
                    [first["item"]["id"].as_str().unwrap()],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                first["count"].as_i64().unwrap()
            );
        }
        for (input, expected) in [
            (
                json!({"request_id":"bad-case","case_id":"unknown"}),
                StatusCode::BAD_REQUEST,
            ),
            (
                json!({"request_id":"client-loot","case_id":"avatar-frames","item_id":"frame-canna-leaf"}),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
        ] {
            assert_eq!(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/gambling/cases/open",
                    input,
                    Some(&auth)
                )
                .await
                .status(),
                expected
            );
        }
        let db = app.db.lock().unwrap();
        assert_eq!(wallet(&db, 1).unwrap().0, 800);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM gambling_requests WHERE user_id=1",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        assert_eq!(equipped(&db, 1).unwrap()["frame"]["id"], "frame-mint-halo");
    }

    #[tokio::test]
    async fn daily_is_shared_with_cannabot_and_failure_rolls_back_entire_game() {
        let (_dir, app) = fixture();
        let auth = account(&app, "daily", false);
        let input = json!({"request_id":"daily-once"});
        let first = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/daily",
                input.clone(),
                Some(&auth),
            )
            .await,
        )
        .await;
        let retry = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/daily",
                input,
                Some(&auth),
            )
            .await,
        )
        .await;
        assert_eq!(first, retry);
        assert_eq!(first["balance"], 100);
        {
            let db = app.db.lock().unwrap();
            assert!(
                cannabot::run(&db, 1, "/daily")
                    .unwrap()
                    .unwrap()
                    .contains("already claimed")
            );
        }
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/blackjack/deal",
                json!({"request_id":"impossible","stake":101}),
                Some(&auth)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        let db = app.db.lock().unwrap();
        assert_eq!(wallet(&db, 1).unwrap().0, 100);
        assert_eq!(
            db.query_row("SELECT count(*) FROM gambling_blackjack", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM gambling_requests WHERE request_id='impossible'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn supported_balance_precision_caps_credit_without_overflow() {
        let (_dir, app) = fixture();
        account(&app, "large", false);
        balance(&app, 1, cannabot::MAX_KASH - 10);
        let db = app.db.lock().unwrap();
        assert_eq!(
            credit(
                &db,
                1,
                whole_return(MAX_STAKE as i128 * MAX_MULTIPLIER as i128 / 100),
                MAX_STAKE
            )
            .unwrap(),
            10
        );
        assert_eq!(wallet(&db, 1).unwrap().0, cannabot::MAX_KASH);
        assert_eq!(credit(&db, 1, 100, 0).unwrap(), 0);
        assert!(centi(f64::NAN).is_err());
        assert!(centi(f64::INFINITY).is_err());
        assert!(centi(1000.01).is_err());
        assert!(centi(0.99).is_err());
    }

    #[tokio::test]
    async fn new_game_daily_limit_never_blocks_finishing_a_paid_hand() {
        let (_dir, app) = fixture();
        let auth = account(&app, "quota", false);
        balance(&app, 1, 1000);
        let id = {
            let db = app.db.lock().unwrap();
            let id = hand(&db, 1, &[9, 8], &[8, 7], &[]);
            for n in 0..200 {
                db.execute("INSERT INTO gambling_requests(user_id,request_id,fingerprint,response,created,kind) VALUES(1,?1,'fixture','{}',?2,'cosmetic_case')",params![format!("past-{n}"),now()]).unwrap();
            }
            id
        };
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/cases/open",
                json!({"request_id":"over-quota","case_id":"canna-case"}),
                Some(&auth)
            )
            .await
            .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        let result = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/blackjack/action",
                json!({"request_id":"finish-existing","hand_id":id,"action":"stand"}),
                Some(&auth),
            )
            .await,
        )
        .await;
        assert_eq!(result["blackjack"]["status"], "won");
        assert_eq!(result["balance"], 1100);
        assert_eq!(wallet(&app.db.lock().unwrap(), 1).unwrap().0, 1100);
    }

    #[tokio::test]
    async fn double_failure_rolls_back_additional_stake_and_hand() {
        let (_dir, app) = fixture();
        let auth = account(&app, "rollback", false);
        balance(&app, 1, 1000);
        let id = {
            let db = app.db.lock().unwrap();
            hand(&db, 1, &[4, 5], &[8, 7], &[])
        };
        let response = call(
            app.clone(),
            "POST",
            "/api/v1/gambling/blackjack/action",
            json!({"request_id":"empty-shoe","hand_id":id,"action":"double"}),
            Some(&auth),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let db = app.db.lock().unwrap();
        assert_eq!(wallet(&db, 1).unwrap().0, 900);
        let current = load_hand(&db, &id, 1).unwrap();
        assert_eq!(current.stake, 100);
        assert_eq!(current.status, "playing");
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM gambling_requests WHERE request_id='empty-shoe'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}
