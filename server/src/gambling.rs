//! Fictional, non-purchasable Kash games. Decisions and wallet changes are persisted
//! together; clients display results but never deal cards or choose payouts.
use super::*;
use rand::{Rng, seq::SliceRandom};
use rusqlite::TransactionBehavior;
use serde::{Deserialize, Serialize};

const MAX_STAKE: i64 = 1_000_000;
const MAX_MULTIPLIER: i64 = 10_000; // hundredths: 100.00x
const BETTING_MS: i64 = 10_000;
const INTERMISSION_MS: i64 = 4_000;
const NOTICE: &str = "Kash is free fictional currency: no purchase, cash-out or transfer. The owner can see Crash outcomes before each round, including random rounds. Controlled rounds are openly marked; this is not a provably-fair game. Blackjack uses a shuffled 52-card shoe, dealer stands on soft 17, no splits or insurance; natural blackjack pays 3:2, rounded down to whole Kash. Balances are capped at 9007199254740991 Kash.";

pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS gambling_config(id INTEGER PRIMARY KEY CHECK(id=1),mode TEXT NOT NULL DEFAULT 'random' CHECK(mode IN ('random','controlled')),paused INTEGER NOT NULL DEFAULT 0);
         INSERT OR IGNORE INTO gambling_config(id) VALUES(1);
         CREATE TABLE IF NOT EXISTS gambling_crash_queue(id INTEGER PRIMARY KEY AUTOINCREMENT,multiplier INTEGER NOT NULL CHECK(multiplier BETWEEN 100 AND 10000));
         CREATE TABLE IF NOT EXISTS gambling_crash_rounds(id INTEGER PRIMARY KEY AUTOINCREMENT,created_ms INTEGER NOT NULL,start_ms INTEGER NOT NULL,crash_ms INTEGER NOT NULL,multiplier INTEGER NOT NULL CHECK(multiplier BETWEEN 100 AND 10000),mode TEXT NOT NULL CHECK(mode IN ('random','controlled')),settled INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE IF NOT EXISTS gambling_crash_bets(round_id INTEGER NOT NULL REFERENCES gambling_crash_rounds(id),user_id INTEGER NOT NULL REFERENCES users(id),stake INTEGER NOT NULL CHECK(stake BETWEEN 1 AND 1000000),auto_multiplier INTEGER,status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','won','lost')),payout INTEGER NOT NULL DEFAULT 0,cashout_multiplier INTEGER,PRIMARY KEY(round_id,user_id));
         CREATE TABLE IF NOT EXISTS gambling_blackjack(id TEXT PRIMARY KEY,user_id INTEGER NOT NULL REFERENCES users(id),stake INTEGER NOT NULL CHECK(stake BETWEEN 1 AND 2000000),deck TEXT NOT NULL,cursor INTEGER NOT NULL,player TEXT NOT NULL,dealer TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'playing',payout INTEGER NOT NULL DEFAULT 0,created INTEGER NOT NULL);
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
        return Err(bad("Choose a stake between 1 and 1000000 whole Kash"));
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
        "SELECT count(*) FROM gambling_requests WHERE user_id=?1 AND created>=?2 AND kind IN ('crash_bet','blackjack_deal','cosmetic_case')",
        params![actor, (now() / 86400) * 86400],
        |r| r.get(0),
    )?;
    if count >= 200 && matches!(kind, "crash_bet" | "blackjack_deal" | "cosmetic_case") {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Today's limit is 200 new games or cases; existing games remain playable",
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

fn random_multiplier() -> i64 {
    // A capped 97%-return distribution; instant 1.00x crashes are possible.
    let sample: f64 = OsRng.gen_range(0.0..1.0);
    (97.0 / (1.0 - sample))
        .floor()
        .clamp(100.0, MAX_MULTIPLIER as f64) as i64
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

fn settle_bet(db: &Connection, round: i64, actor: i64, multiplier: Option<i64>) -> ApiResult<()> {
    let stake: Option<i64> = db.query_row(
        "SELECT stake FROM gambling_crash_bets WHERE round_id=?1 AND user_id=?2 AND status='pending'",
        params![round, actor],
        |r| r.get(0),
    ).optional()?;
    if let Some(stake) = stake {
        let payout = match multiplier {
            Some(multiplier) => credit(db, actor, stake * multiplier / 100, stake)?,
            None => 0,
        };
        db.execute(
            "UPDATE gambling_crash_bets SET status=?1,payout=?2,cashout_multiplier=?3 WHERE round_id=?4 AND user_id=?5 AND status='pending'",
            params![if multiplier.is_some() {"won"} else {"lost"}, payout, multiplier, round, actor],
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
                *auto < round.multiplier && time >= at_multiplier(round.start, *auto)
            }) {
                settle_bet(db, round.id, actor, Some(auto))?;
            } else if time >= round.crash {
                settle_bet(db, round.id, actor, None)?;
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
    let (mode, paused): (String, bool) = db.query_row(
        "SELECT mode,paused FROM gambling_config WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if paused {
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
        random_multiplier()
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
        "SELECT stake,auto_multiplier,status,payout,cashout_multiplier FROM gambling_crash_bets WHERE round_id=?1 AND user_id=?2",
        params![round, actor],
        |r| Ok(json!({"round_id":round,"stake":r.get::<_,i64>(0)?,"auto_cashout":r.get::<_,Option<i64>>(1)?.map(|v|v as f64/100.0),"status":r.get::<_,String>(2)?,"payout":r.get::<_,i64>(3)?,"cashout_multiplier":r.get::<_,Option<i64>>(4)?.map(|v|v as f64/100.0)})),
    ).optional()?.unwrap_or(Value::Null))
}

fn crash_view(
    db: &Connection,
    round: Option<&Round>,
    actor: i64,
    time: i64,
    owner: bool,
) -> ApiResult<Value> {
    let (mode, paused): (String, bool) = db.query_row(
        "SELECT mode,paused FROM gambling_config WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let history = db.prepare("SELECT id,multiplier,mode FROM gambling_crash_rounds WHERE crash_ms<=?1 ORDER BY id DESC LIMIT 20")?
        .query_map([time], |r| Ok(json!({"id":r.get::<_,i64>(0)?,"crash_multiplier":r.get::<_,i64>(1)? as f64/100.0,"mode":r.get::<_,String>(2)?})))?
        .collect::<Result<Vec<_>,_>>()?;
    let mut view = json!({"id":null,"phase":"paused","multiplier":1.0,"mode":mode,"paused":paused,"owner_visible":true,"bet":null,"history":history});
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
    if !multiplier.is_finite() || !(1.0..=100.0).contains(&multiplier) {
        return Err(bad("Crash multipliers must be from 1.00 to 100.00"));
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
    let actor = app.auth(&headers)?.0;
    once(
        &app,
        actor,
        &input.request_id,
        "crash_bet",
        &json!({"round_id":input.round_id,"stake":input.stake,"auto_cashout":input.auto_cashout}),
        |db| {
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
    let actor = app.auth(&headers)?.0;
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
    let mut value = json!({"id":hand.id,"stake":hand.stake,"cards":hand.player.iter().map(|c|card_view(*c)).collect::<Vec<_>>(),"dealer":dealer,"player_total":total(&hand.player),"status":hand.status,"payout":hand.payout,"can_double":playing && hand.player.len()==2});
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
        ("blackjack", hand.stake * 5 / 2)
    } else if natural && dealer == 21 {
        ("lost", 0)
    } else if dealer > 21 || player > dealer {
        ("won", hand.stake * 2)
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
    let actor = app.auth(&headers)?.0;
    once(
        &app,
        actor,
        &input.request_id,
        "blackjack_deal",
        &json!({"stake":input.stake}),
        |db| {
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
    let actor = app.auth(&headers)?.0;
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

fn catalog() -> ApiResult<Value> {
    let catalog: Value = serde_json::from_str(include_str!("../web/cosmetics/catalog.json"))
        .map_err(|_| bad("Cosmetics catalog is unavailable"))?;
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
            || !matches!(item["kind"].as_str(), Some("frame" | "banner"))
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
            .is_some_and(|p| (1..=MAX_STAKE).contains(&p))
    {
        return Err(bad("Cosmetics catalog is unavailable"));
    }
    Ok(catalog)
}

fn cosmetics_view(db: &Connection, actor: i64) -> ApiResult<(Value, Value)> {
    let catalog = catalog()?;
    let items = catalog["items"].as_array().unwrap();
    let sum: u64 = items.iter().map(|i| i["weight"].as_u64().unwrap()).sum();
    let owned = db
        .prepare("SELECT item_id,count FROM gambling_cosmetics WHERE user_id=?1 ORDER BY item_id")?
        .query_map([actor], |r| {
            Ok(json!({"id":r.get::<_,String>(0)?,"count":r.get::<_,i64>(1)?}))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let (frame, banner): (Option<String>, Option<String>) = db
        .query_row(
            "SELECT frame,banner FROM gambling_equipped WHERE user_id=?1",
            [actor],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .unwrap_or_default();
    let cosmetics =
        json!({"catalog":items,"owned":owned,"equipped":{"frame":frame,"banner":banner}});
    let cases = json!([{"id":"canna-case","name":catalog["case"]["name"],"cost":catalog["case"]["price"],"items":items.iter().filter(|i|i["weight"].as_u64().unwrap()>0).map(|i|json!({"id":i["id"],"odds_percent":100.0*i["weight"].as_u64().unwrap() as f64/sum as f64})).collect::<Vec<_>>(),"duplicates":"Duplicates increase your collection count; there is no sale or trade."}]);
    Ok((cosmetics, cases))
}

pub fn equipped(db: &Connection, actor: i64) -> ApiResult<Value> {
    let (frame, banner): (Option<String>, Option<String>) = db
        .query_row(
            "SELECT frame,banner FROM gambling_equipped WHERE user_id=?1",
            [actor],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .unwrap_or_default();
    let catalog = catalog()?;
    let items = catalog["items"].as_array().unwrap();
    let find = |id: Option<String>| {
        id.and_then(|id| items.iter().find(|item| item["id"] == id).cloned())
            .unwrap_or(Value::Null)
    };
    Ok(json!({"frame":find(frame),"banner":find(banner)}))
}

#[derive(Deserialize, Serialize)]
pub struct CaseInput {
    request_id: String,
    case_id: String,
}
pub async fn case_open(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<CaseInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = app.auth(&headers)?.0;
    once(
        &app,
        actor,
        &input.request_id,
        "cosmetic_case",
        &json!({"case_id":input.case_id}),
        |db| {
            if input.case_id != "canna-case" {
                return Err(bad("Cosmetic case not found"));
            }
            let catalog = catalog()?;
            let items = catalog["items"].as_array().unwrap();
            let sum: u64 = items.iter().map(|i| i["weight"].as_u64().unwrap()).sum();
            let mut roll = OsRng.gen_range(0..sum);
            let mut selected = None;
            for item in items {
                let weight = item["weight"].as_u64().unwrap();
                if roll < weight {
                    selected = Some(item.clone());
                    break;
                }
                roll -= weight;
            }
            let item = selected.ok_or_else(|| bad("Cosmetic case is unavailable"))?;
            debit(db, actor, catalog["case"]["price"].as_i64().unwrap())?;
            let id = item["id"].as_str().unwrap();
            db.execute("INSERT INTO gambling_cosmetics VALUES(?1,?2,1) ON CONFLICT(user_id,item_id) DO UPDATE SET count=MIN(1000000,count+1)",params![actor,id])?;
            let count: i64 = db.query_row(
                "SELECT count FROM gambling_cosmetics WHERE user_id=?1 AND item_id=?2",
                params![actor, id],
                |r| r.get(0),
            )?;
            Ok(json!({"item":item,"count":count}))
        },
    )
}

#[derive(Deserialize)]
pub struct EquipInput {
    frame: Option<String>,
    banner: Option<String>,
}
pub async fn equip(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<EquipInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = app.auth(&headers)?.0;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let catalog = catalog()?;
    for (kind, id) in [("frame", &input.frame), ("banner", &input.banner)] {
        if let Some(id) = id {
            if !catalog["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["id"] == *id && i["kind"] == kind)
            {
                return Err(bad("Choose an item of the correct cosmetic kind"));
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
    tx.execute("INSERT INTO gambling_equipped VALUES(?1,?2,?3) ON CONFLICT(user_id) DO UPDATE SET frame=excluded.frame,banner=excluded.banner",params![actor,input.frame,input.banner])?;
    tx.commit()?;
    Ok(axum::Json(
        json!({"ok":true,"equipped":{"frame":input.frame,"banner":input.banner}}),
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
    let actor = app.auth(&headers)?.0;
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

pub async fn overview(
    State(app): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    let actor = app.auth(&headers)?.0;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let time = milliseconds();
    let round = advance(&tx, time)?;
    let (balance, earned, daily) = wallet(&tx, actor)?;
    let hand_id:Option<String>=tx.query_row("SELECT id FROM gambling_blackjack WHERE user_id=?1 ORDER BY created DESC,rowid DESC LIMIT 1",[actor],|r|r.get(0)).optional()?;
    let hand = hand_id
        .map(|id| load_hand(&tx, &id, actor).map(|h| hand_view(&h)))
        .transpose()?
        .unwrap_or(Value::Null);
    let (cosmetics, cases) = cosmetics_view(&tx, actor)?;
    let result = json!({"wallet":{"balance":balance,"earned":earned,"daily_available":daily!=now()/86400},"server_time_ms":time,"notice":NOTICE,"crash":crash_view(&tx,round.as_ref(),actor,time,false)?,"blackjack":hand,"cosmetics":cosmetics,"cases":cases,"limits":{"max_stake":MAX_STAKE,"max_new_games_per_utc_day":200}});
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
    let queue = db
        .prepare("SELECT multiplier FROM gambling_crash_queue ORDER BY id")?
        .query_map([], |r| {
            Ok(json!({"crash_multiplier":r.get::<_,i64>(0)? as f64/100.0}))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let crash = crash_view(db, round.as_ref(), actor, time, true)?;
    Ok(
        json!({"mode":mode,"paused":paused,"queue":queue,"crash":crash,"planned_crash_multiplier":crash["planned_crash_multiplier"],"planned_crash_at_ms":crash["planned_crash_at_ms"],"server_time_ms":time,"notice":NOTICE,"limits":{"queued_rounds":20,"min_multiplier":1.0,"max_multiplier":100.0},"queue_empty_behavior":"Controlled mode remains visibly controlled and draws random rounds when its queue is empty."}),
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
}
pub async fn admin_config(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<ConfigInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = community::owner(&app, &headers)?;
    if !matches!(input.mode.as_str(), "random" | "controlled") || input.queue.len() > 20 {
        return Err(bad(
            "Choose random or controlled mode and at most 20 queued rounds",
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
        "UPDATE gambling_config SET mode=?1,paused=?2 WHERE id=1",
        params![input.mode, input.paused],
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

    fn hand(db: &Connection, actor: i64, player: &[u8], dealer: &[u8], next: &[u8]) -> String {
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
        assert!(
            result["notice"]
                .as_str()
                .unwrap()
                .contains("not a provably-fair")
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
        let excessive = json!({"mode":"controlled","paused":false,"queue":[{"min_multiplier":1.0,"max_multiplier":100.0,"rounds":21}]});
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
            credit(&db, 1, MAX_STAKE * MAX_MULTIPLIER / 100, MAX_STAKE).unwrap(),
            10
        );
        assert_eq!(wallet(&db, 1).unwrap().0, cannabot::MAX_KASH);
        assert_eq!(credit(&db, 1, 100, 0).unwrap(), 0);
        assert!(centi(f64::NAN).is_err());
        assert!(centi(f64::INFINITY).is_err());
        assert!(centi(100.01).is_err());
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
