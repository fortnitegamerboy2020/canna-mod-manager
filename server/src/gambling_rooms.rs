//! Multiplayer tables hold stakes before play. Only stored server outcomes
//! settle a table, and an expired waiting table refunds its members.
use super::*;

#[derive(Serialize, Deserialize, Default)]
struct Table {
    deck: Vec<u8>,
    cursor: usize,
    dealer: Vec<u8>,
    seats: Vec<Seat>,
    #[serde(default)]
    result: Value,
}
#[derive(Serialize, Deserialize)]
struct Seat {
    user: i64,
    stake: i64,
    choice: String,
    number: Option<i64>,
    cards: Vec<u8>,
    done: bool,
    payout: i64,
}
struct Room {
    id: String,
    host: i64,
    game: String,
    stake: i64,
    status: String,
    public: bool,
    invite_hash: String,
    capacity: i64,
    factor: i64,
    expires: i64,
    table: Table,
}
pub(super) fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS gambling_rooms(id TEXT PRIMARY KEY,host INTEGER NOT NULL REFERENCES users(id),game TEXT NOT NULL CHECK(game IN ('coinflip','blackjack','roulette')),stake INTEGER NOT NULL CHECK(stake BETWEEN 1 AND 9007199254740991),status TEXT NOT NULL CHECK(status IN ('waiting','playing','settled','cancelled','expired')),public INTEGER NOT NULL,invite_hash TEXT NOT NULL,capacity INTEGER NOT NULL CHECK(capacity BETWEEN 2 AND 8),factor INTEGER NOT NULL CHECK(factor BETWEEN 0 AND 200),expires INTEGER NOT NULL,created INTEGER NOT NULL,table_json TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS gambling_rooms_open ON gambling_rooms(status,expires);
        CREATE TABLE IF NOT EXISTS gambling_room_members(room_id TEXT NOT NULL REFERENCES gambling_rooms(id),user_id INTEGER NOT NULL REFERENCES users(id),PRIMARY KEY(room_id,user_id));")
}
fn load(db: &Connection, id: &str) -> ApiResult<Room> {
    if !identifier(id) {
        return Err(bad("Invalid table identity"));
    }
    let (host,game,stake,status,public,invite_hash,capacity,factor,expires,data):(i64,String,i64,String,bool,String,i64,i64,i64,String)=db.query_row("SELECT host,game,stake,status,public,invite_hash,capacity,factor,expires,table_json FROM gambling_rooms WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?))).optional()?.ok_or(ApiError(StatusCode::NOT_FOUND,"Table not found"))?;
    let table = serde_json::from_str(&data).map_err(|_| bad("Stored table is unavailable"))?;
    Ok(Room {
        id: id.into(),
        host,
        game,
        stake,
        status,
        public,
        invite_hash,
        capacity,
        factor,
        expires,
        table,
    })
}
fn save(db: &Connection, room: &Room) -> ApiResult<()> {
    let data = serde_json::to_string(&room.table).map_err(|_| bad("Table could not be saved"))?;
    db.execute(
        "UPDATE gambling_rooms SET status=?1,expires=?2,table_json=?3 WHERE id=?4",
        params![room.status, room.expires, data, room.id],
    )?;
    Ok(())
}
fn member(room: &Room, actor: i64) -> bool {
    room.table.seats.iter().any(|s| s.user == actor)
}
fn visible(room: &Room, actor: i64) -> ApiResult<()> {
    if room.public || member(room, actor) {
        Ok(())
    } else {
        Err(ApiError(StatusCode::NOT_FOUND, "Table not found"))
    }
}
fn view(db: &Connection, room: &Room, actor: i64) -> ApiResult<Value> {
    let finished = room.status == "settled";
    let seats=room.table.seats.iter().map(|s|{let username:String=db.query_row("SELECT username FROM users WHERE id=?1",[s.user],|r|r.get(0))?;Ok(json!({"user_id":s.user,"username":username,"stake":s.stake,"choice":s.choice,"number":s.number,"cards":s.cards.iter().map(|c|card_view(*c)).collect::<Vec<_>>(),"total":total(&s.cards),"done":s.done,"payout":s.payout}))}).collect::<ApiResult<Vec<_>>>()?;
    let dealer = room
        .table
        .dealer
        .iter()
        .enumerate()
        .map(|(i, c)| {
            if i > 0 && !finished {
                json!({"hidden":true})
            } else {
                card_view(*c)
            }
        })
        .collect::<Vec<_>>();
    Ok(
        json!({"id":room.id,"host_id":room.host,"game":room.game,"stake":room.stake,"status":room.status,"public":room.public,"capacity":room.capacity,"expires":room.expires,"seats":seats,"dealer":dealer,"dealer_total":if finished {Some(total(&room.table.dealer))}else{None},"result":room.table.result,"joined":member(room,actor)}),
    )
}
fn card(table: &mut Table) -> ApiResult<u8> {
    let c = table
        .deck
        .get(table.cursor)
        .copied()
        .filter(|c| *c < 52)
        .ok_or_else(|| bad("Stored table shoe exhausted"))?;
    table.cursor += 1;
    Ok(c)
}
fn finish_blackjack(db: &Connection, room: &mut Room) -> ApiResult<()> {
    if room.status != "playing" || room.table.seats.iter().any(|s| !s.done) {
        return Ok(());
    }
    while total(&room.table.dealer) < 17 {
        let c = card(&mut room.table)?;
        room.table.dealer.push(c);
    }
    let dealer = total(&room.table.dealer);
    let dealer_natural = dealer == 21 && room.table.dealer.len() == 2;
    for seat in &mut room.table.seats {
        let player = total(&seat.cards);
        let natural = player == 21 && seat.cards.len() == 2;
        let payout = if player > 21 || dealer_natural && !natural {
            0
        } else if natural && !dealer_natural {
            whole_return(i128::from(seat.stake) * 5 / 2)
        } else if player == dealer {
            seat.stake
        } else if dealer > 21 || player > dealer {
            whole_return(i128::from(seat.stake) * 2)
        } else {
            0
        };
        // The selected table payout factor is frozen when the host creates it.
        seat.payout = credit(
            db,
            seat.user,
            whole_return(i128::from(payout) * i128::from(room.factor) / 100),
            seat.stake,
        )?;
    }
    room.status = "settled".into();
    room.table.result = json!({"dealer_total":dealer});
    save(db, room)
}
fn settle(db: &Connection, room: &mut Room) -> ApiResult<()> {
    match room.game.as_str() {
        "coinflip" => {
            if room.table.seats.len() != 2 {
                return Err(bad("Coinflip requires two players"));
            }
            let pot = room
                .stake
                .checked_mul(2)
                .filter(|s| *s <= MAX_STAKE)
                .ok_or_else(|| bad("The combined stake exceeds whole-Kash precision"))?;
            for seat in &room.table.seats {
                if wallet(db, seat.user)?.0 > MAX_STAKE - pot {
                    return Err(bad(
                        "A player's wallet is full; spend Kash before starting this table",
                    ));
                }
            }
            let index = rand::thread_rng().gen_range(0..2);
            let winner = &mut room.table.seats[index];
            market::transfer_credit(db, winner.user, pot)?;
            winner.payout = pot;
            room.table.result = json!({"side":winner.choice,"winner_id":winner.user});
            room.status = "settled".into();
        }
        "roulette" => {
            let number = rand::thread_rng().gen_range(0..37_i64);
            let red = matches!(
                number,
                1 | 3 | 5 | 7 | 9 | 12 | 14 | 16 | 18 | 19 | 21 | 23 | 25 | 27 | 30 | 32 | 34 | 36
            );
            for seat in &mut room.table.seats {
                let win = arcade::roulette_win(number, &seat.choice, seat.number);
                let factor = if seat.choice == "number" { 36 } else { 2 };
                let payout = if win {
                    whole_return(i128::from(seat.stake) * factor * i128::from(room.factor) / 100)
                } else {
                    0
                };
                seat.payout = credit(db, seat.user, payout, seat.stake)?;
            }
            room.table.result = json!({"number":number,"color":if number==0{"green"}else if red{"red"}else{"black"}});
            room.status = "settled".into();
        }
        "blackjack" => {
            room.table.deck = (0..6).flat_map(|_| 0..52_u8).collect();
            room.table.deck.shuffle(&mut rand::thread_rng());
            room.table.cursor = 0;
            for i in 0..room.table.seats.len() {
                for _ in 0..2 {
                    let c = card(&mut room.table)?;
                    room.table.seats[i].cards.push(c);
                }
                room.table.seats[i].done = total(&room.table.seats[i].cards) == 21;
            }
            for _ in 0..2 {
                let c = card(&mut room.table)?;
                room.table.dealer.push(c);
            }
            room.status = "playing".into();
            room.expires = now() + 180;
            if total(&room.table.dealer) == 21 {
                for s in &mut room.table.seats {
                    s.done = true;
                }
            }
            finish_blackjack(db, room)?;
        }
        _ => return Err(bad("Unknown table game")),
    }
    save(db, room)
}
fn advance(db: &Connection) -> ApiResult<()> {
    let ids=db.prepare("SELECT id FROM gambling_rooms WHERE status IN ('waiting','playing') AND expires<=?1 ORDER BY expires LIMIT 1000")?.query_map([now()],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
    for id in ids {
        db.execute_batch("SAVEPOINT expire_table;")?;
        let result = (|| -> ApiResult<()> {
            let mut room = load(db, &id)?;
            if room.status == "waiting" {
                for s in &room.table.seats {
                    market::transfer_credit(db, s.user, s.stake)?;
                }
                room.status = "expired".into();
                save(db, &room)
            } else {
                for s in &mut room.table.seats {
                    s.done = true;
                }
                finish_blackjack(db, &mut room)
            }
        })();
        if result.is_err() {
            db.execute_batch("ROLLBACK TO expire_table;")?;
        }
        db.execute_batch("RELEASE expire_table;")?;
    }
    Ok(())
}
pub async fn rooms(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    let actor = app.auth(&headers)?.0;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    advance(&tx)?;
    let ids=tx.prepare("SELECT r.id FROM gambling_rooms r WHERE (r.public=1 AND r.status='waiting') OR EXISTS(SELECT 1 FROM gambling_room_members m WHERE m.room_id=r.id AND m.user_id=?1) AND r.created>?2 ORDER BY r.created DESC,r.rowid DESC LIMIT 100")?.query_map(params![actor,now()-86400],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
    let result = ids
        .iter()
        .map(|id| view(&tx, &load(&tx, id)?, actor))
        .collect::<ApiResult<Vec<_>>>()?;
    tx.commit()?;
    Ok(axum::Json(json!({"member_id":actor,"rooms":result})))
}
pub async fn room(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<RoomQuery>,
) -> ApiResult<axum::Json<Value>> {
    let actor = app.auth(&headers)?.0;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    advance(&tx)?;
    let room = load(&tx, &id)?;
    if visible(&room, actor).is_err()
        && query
            .invite
            .as_deref()
            .is_none_or(|invite| digest(invite) != room.invite_hash)
    {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "Table invitation is invalid",
        ));
    }
    let value = view(&tx, &room, actor)?;
    tx.commit()?;
    Ok(axum::Json(json!({"member_id":actor,"room":value})))
}
#[derive(Deserialize)]
pub struct RoomQuery {
    #[serde(default)]
    invite: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomInput {
    request_id: String,
    action: String,
    #[serde(default)]
    room_id: Option<String>,
    #[serde(default)]
    game: Option<String>,
    #[serde(default)]
    stake: Option<i64>,
    #[serde(default)]
    choice: Option<String>,
    #[serde(default)]
    number: Option<i64>,
    #[serde(default)]
    public: Option<bool>,
    #[serde(default)]
    capacity: Option<i64>,
    #[serde(default)]
    invite: Option<String>,
}
fn choice(game: &str, value: Option<&str>, number: Option<i64>) -> ApiResult<String> {
    let choice = value.unwrap_or(if game == "coinflip" {
        "heads"
    } else if game == "roulette" {
        "red"
    } else {
        "stand"
    });
    let valid = match game {
        "coinflip" => matches!(choice, "heads" | "tails") && number.is_none(),
        "blackjack" => choice == "stand" && number.is_none(),
        "roulette" => {
            matches!(choice, "red" | "black" | "even" | "odd" | "low" | "high") && number.is_none()
                || choice == "number" && number.is_some_and(|n| (0..=36).contains(&n))
        }
        _ => false,
    };
    if !valid {
        return Err(bad("Choose a valid bet for this game"));
    }
    Ok(choice.into())
}
fn capacity(db: &Connection, actor: i64) -> ApiResult<()> {
    let active:i64=db.query_row("SELECT count(*) FROM gambling_room_members m JOIN gambling_rooms r ON r.id=m.room_id WHERE m.user_id=?1 AND r.status IN ('waiting','playing')",[actor],|r|r.get(0))?;
    if active >= 3 {
        return Err(bad("Finish or leave one of your active tables first"));
    }
    Ok(())
}
pub async fn room_action(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<RoomInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = mutation_actor(&app, &headers)?;
    app.limits.check(format!("table-action:{actor}"), 60)?;
    let kind = if matches!(input.action.as_str(), "create" | "join") {
        "online_room"
    } else {
        "online_action"
    };
    once(
        &app,
        actor,
        &input.request_id,
        kind,
        &json!({"action":input.action,"room":input.room_id,"game":input.game,"stake":input.stake,"choice":input.choice,"number":input.number,"public":input.public,"capacity":input.capacity,"invite":input.invite}),
        |db| {
            advance(db)?;
            if input.action == "create" {
                if input.room_id.is_some() || input.invite.is_some() {
                    return Err(bad("Unexpected new-table identity"));
                }
                capacity(db, actor)?;
                let all: i64 = db.query_row(
                    "SELECT count(*) FROM gambling_rooms WHERE status IN ('waiting','playing')",
                    [],
                    |r| r.get(0),
                )?;
                if all >= 1000 {
                    return Err(bad("Table capacity reached"));
                }
                let game = input
                    .game
                    .as_deref()
                    .filter(|g| matches!(*g, "blackjack" | "roulette" | "coinflip"))
                    .ok_or_else(|| bad("Choose blackjack, roulette or coinflip"))?;
                let stake = input.stake.ok_or_else(|| bad("Choose a stake"))?;
                let factor = if game == "coinflip" {
                    if arcade::paused(db, "coinflip")? {
                        return Err(bad("New wagering is paused"));
                    }
                    100
                } else {
                    arcade::new_game(db, game, stake)?
                };
                if game == "coinflip" && stake > MAX_STAKE / 2 {
                    return Err(bad(
                        "A combined coinflip stake must fit whole-Kash precision",
                    ));
                }
                let capacity = if game == "coinflip" {
                    2
                } else {
                    input.capacity.unwrap_or(6)
                };
                if !(2..=8).contains(&capacity) {
                    return Err(bad("Choose two to eight seats"));
                }
                let choice = choice(game, input.choice.as_deref(), input.number)?;
                debit(db, actor, stake)?;
                let id = Uuid::new_v4().to_string();
                let invite = Uuid::new_v4().to_string();
                let table = Table {
                    seats: vec![Seat {
                        user: actor,
                        stake,
                        choice,
                        number: input.number,
                        cards: Vec::new(),
                        done: false,
                        payout: 0,
                    }],
                    ..Table::default()
                };
                let room = Room {
                    id: id.clone(),
                    host: actor,
                    game: game.into(),
                    stake,
                    status: "waiting".into(),
                    public: input.public.unwrap_or(true),
                    invite_hash: digest(&invite),
                    capacity,
                    factor,
                    expires: now() + 600,
                    table,
                };
                db.execute(
                    "INSERT INTO gambling_rooms VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                    params![
                        room.id,
                        actor,
                        room.game,
                        stake,
                        room.status,
                        room.public,
                        room.invite_hash,
                        capacity,
                        factor,
                        room.expires,
                        now(),
                        serde_json::to_string(&room.table).unwrap()
                    ],
                )?;
                db.execute(
                    "INSERT INTO gambling_room_members VALUES(?1,?2)",
                    params![id, actor],
                )?;
                return Ok(json!({"room":view(db,&room,actor)?,"invite":invite}));
            }
            if input.game.is_some()
                || input.stake.is_some()
                || input.public.is_some()
                || input.capacity.is_some()
            {
                return Err(bad("Table terms are fixed by the host"));
            }
            let id = input
                .room_id
                .as_deref()
                .ok_or_else(|| bad("Choose a table"))?;
            let mut room = load(db, id)?;
            if input.action == "join" {
                if room.status != "waiting" || room.expires <= now() {
                    return Err(ApiError(
                        StatusCode::CONFLICT,
                        "This table is no longer accepting players",
                    ));
                }
                capacity(db, actor)?;
                if !room.public
                    && input
                        .invite
                        .as_deref()
                        .is_none_or(|invite| digest(invite) != room.invite_hash)
                {
                    return Err(ApiError(
                        StatusCode::NOT_FOUND,
                        "Table invitation is invalid",
                    ));
                }
                if member(&room, actor) || room.table.seats.len() >= room.capacity as usize {
                    return Err(bad("You already joined or the table is full"));
                }
                if room.game == "coinflip" {
                    if arcade::paused(db, "coinflip")? {
                        return Err(bad("New wagering is paused"));
                    }
                } else {
                    arcade::new_game(db, &room.game, room.stake)?;
                }
                let selected = if room.game == "coinflip" {
                    if input.number.is_some() || input.choice.is_some() {
                        return Err(bad("The joining player takes the opposite coin side"));
                    }
                    if room.table.seats[0].choice == "heads" {
                        "tails".into()
                    } else {
                        "heads".into()
                    }
                } else {
                    choice(&room.game, input.choice.as_deref(), input.number)?
                };
                debit(db, actor, room.stake)?;
                room.table.seats.push(Seat {
                    user: actor,
                    stake: room.stake,
                    choice: selected,
                    number: input.number,
                    cards: Vec::new(),
                    done: false,
                    payout: 0,
                });
                db.execute(
                    "INSERT INTO gambling_room_members VALUES(?1,?2)",
                    params![id, actor],
                )?;
                save(db, &room)?;
            } else {
                if input.choice.is_some() || input.number.is_some() || input.invite.is_some() {
                    return Err(bad("Unexpected action terms"));
                }
                visible(&room, actor)?;
                if !member(&room, actor) {
                    return Err(ApiError(StatusCode::FORBIDDEN, "Join this table first"));
                }
                match input.action.as_str() {
                    "rotate_invite" => {
                        if actor != room.host || room.status != "waiting" {
                            return Err(ApiError(
                                StatusCode::FORBIDDEN,
                                "Only the host of a waiting table can replace its invitation",
                            ));
                        }
                        let invite = Uuid::new_v4().to_string();
                        db.execute(
                            "UPDATE gambling_rooms SET invite_hash=?1 WHERE id=?2",
                            params![digest(&invite), id],
                        )?;
                        return Ok(json!({"room":view(db,&room,actor)?,"invite":invite}));
                    }
                    "start" => {
                        if actor != room.host {
                            return Err(ApiError(
                                StatusCode::FORBIDDEN,
                                "Only the host can start this table",
                            ));
                        }
                        if room.status != "waiting"
                            || room.table.seats.len() < 2
                            || room.expires <= now()
                        {
                            return Err(bad("Start a waiting table with at least two players"));
                        }
                        settle(db, &mut room)?;
                    }
                    "leave" | "cancel" => {
                        if room.status != "waiting" {
                            return Err(bad("A started table must finish before leaving"));
                        }
                        if input.action == "cancel" && room.host != actor {
                            return Err(ApiError(
                                StatusCode::FORBIDDEN,
                                "Only the host can cancel this table",
                            ));
                        }
                        if room.host == actor {
                            for s in &room.table.seats {
                                market::transfer_credit(db, s.user, s.stake)?;
                            }
                            room.status = "cancelled".into();
                        } else {
                            let index = room
                                .table
                                .seats
                                .iter()
                                .position(|s| s.user == actor)
                                .unwrap();
                            let seat = room.table.seats.remove(index);
                            market::transfer_credit(db, actor, seat.stake)?;
                            db.execute(
                                "DELETE FROM gambling_room_members WHERE room_id=?1 AND user_id=?2",
                                params![id, actor],
                            )?;
                        }
                        save(db, &room)?;
                    }
                    "hit" | "stand" | "double" => {
                        if room.game != "blackjack" || room.status != "playing" {
                            return Err(bad("There is no active blackjack hand at this table"));
                        }
                        let index = room
                            .table
                            .seats
                            .iter()
                            .position(|s| s.user == actor)
                            .unwrap();
                        if room.table.seats[index].done {
                            return Err(bad("Your hand is already finished"));
                        }
                        if input.action == "double" {
                            let seat = &room.table.seats[index];
                            if seat.cards.len() != 2 || seat.stake > MAX_STAKE / 2 {
                                return Err(bad(
                                    "Double is available only on your first two cards within whole-Kash precision",
                                ));
                            }
                            debit(db, actor, seat.stake)?;
                            room.table.seats[index].stake *= 2;
                        }
                        if input.action != "stand" {
                            let c = card(&mut room.table)?;
                            room.table.seats[index].cards.push(c);
                        }
                        if input.action != "hit" || total(&room.table.seats[index].cards) >= 21 {
                            room.table.seats[index].done = true;
                        }
                        finish_blackjack(db, &mut room)?;
                        save(db, &room)?;
                    }
                    _ => {
                        return Err(bad(
                            "Choose create, join, start, leave, cancel, hit, stand or double",
                        ));
                    }
                }
            }
            if !member(&room, actor) && !room.public {
                return Ok(json!({"left":true,"room":{"id":room.id,"status":"left"}}));
            }
            Ok(json!({"room":view(db,&room,actor)?}))
        },
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoinInput {
    request_id: String,
    stake: i64,
    choice: String,
}
pub async fn solo_coinflip(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<CoinInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = mutation_actor(&app, &headers)?;
    once(
        &app,
        actor,
        &input.request_id,
        "coinflip",
        &json!({"stake":input.stake,"choice":input.choice}),
        |db| {
            if arcade::paused(db, "coinflip")? {
                return Err(bad("New wagering is paused"));
            }
            choice("coinflip", Some(&input.choice), None)?;
            debit(db, actor, input.stake)?;
            let side = if rand::thread_rng().gen_bool(0.5) {
                "heads"
            } else {
                "tails"
            };
            let payout = credit(
                db,
                actor,
                if side == input.choice {
                    whole_return(i128::from(input.stake) * 2)
                } else {
                    0
                },
                input.stake,
            )?;
            Ok(
                json!({"game":"coinflip","stake":input.stake,"payout":payout,"result":{"side":side,"won":side==input.choice}}),
            )
        },
    )
}
pub fn start_cleanup(app: Shared) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let mut db = app.db.lock().unwrap();
            if let Ok(tx) = db.transaction_with_behavior(TransactionBehavior::Immediate)
                && advance(&tx).is_ok()
                && cosmetic_games::expire(&tx).is_ok()
            {
                let _ = tx.commit();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    fn balances(app: &App) {
        let db = app.db.lock().unwrap();
        for id in 1..=3 {
            wallet(&db, id).unwrap();
        }
        db.execute("UPDATE bot_wallets SET balance=1000", [])
            .unwrap();
    }
    #[tokio::test]
    async fn coinflip_escrow_settles_once_and_private_invites_do_not_expose_shoes() {
        let (_dir, app) = fixture();
        let host = account(&app, "host", false);
        let guest = account(&app, "guest", false);
        let other = account(&app, "other", true);
        balances(&app);
        let created=value(call(app.clone(),"POST","/api/v1/gambling/rooms/action",json!({"request_id":"create","action":"create","game":"coinflip","stake":200,"public":false}),Some(&host)).await).await;
        let id = created["room"]["id"].as_str().unwrap();
        let path = format!("/api/v1/gambling/rooms/{id}");
        assert_eq!(
            call(app.clone(), "GET", &path, Value::Null, Some(&other))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        let join =
            json!({"request_id":"join","action":"join","room_id":id,"invite":created["invite"]});
        value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/rooms/action",
                join,
                Some(&guest),
            )
            .await,
        )
        .await;
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/rooms/action",
                json!({"request_id":"start-other","action":"start","room_id":id}),
                Some(&guest)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let start = json!({"request_id":"start","action":"start","room_id":id});
        let result = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/rooms/action",
                start.clone(),
                Some(&host),
            )
            .await,
        )
        .await;
        assert_eq!(result["room"]["status"], "settled");
        assert_eq!(
            result,
            value(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/gambling/rooms/action",
                    start,
                    Some(&host)
                )
                .await
            )
            .await
        );
        let db = app.db.lock().unwrap();
        assert_eq!(wallet(&db, 1).unwrap().0 + wallet(&db, 2).unwrap().0, 2000);
        assert_eq!(db.query_row("SELECT sum(json_extract(value,'$.payout')) FROM gambling_rooms,json_each(table_json,'$.seats')",[],|r|r.get::<_,i64>(0)).unwrap(),400);
    }
    #[tokio::test]
    async fn blackjack_hidden_shoe_timeout_and_waiting_refunds() {
        let (_dir, app) = fixture();
        let host = account(&app, "host", false);
        let guest = account(&app, "guest", false);
        account(&app, "other", false);
        balances(&app);
        let created = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/rooms/action",
                json!({"request_id":"bj","action":"create","game":"blackjack","stake":100}),
                Some(&host),
            )
            .await,
        )
        .await;
        let id = created["room"]["id"].as_str().unwrap();
        value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/rooms/action",
                json!({"request_id":"join","action":"join","room_id":id}),
                Some(&guest),
            )
            .await,
        )
        .await;
        let result = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/rooms/action",
                json!({"request_id":"start","action":"start","room_id":id}),
                Some(&host),
            )
            .await,
        )
        .await;
        assert!(result["room"].get("deck").is_none());
        assert!(result["room"].get("cursor").is_none());
        if result["room"]["status"] == "playing" {
            assert_eq!(result["room"]["dealer"][1]["hidden"], true);
            assert!(result["room"]["dealer_total"].is_null());
            {
                let db = app.db.lock().unwrap();
                db.execute("UPDATE gambling_rooms SET expires=0 WHERE id=?1", [id])
                    .unwrap();
            }
            let settled = value(
                call(
                    app.clone(),
                    "GET",
                    &format!("/api/v1/gambling/rooms/{id}"),
                    Value::Null,
                    Some(&guest),
                )
                .await,
            )
            .await;
            assert_eq!(settled["room"]["status"], "settled");
            assert!(settled["room"]["dealer"][1].get("hidden").is_none());
        }
        let before = wallet(&app.db.lock().unwrap(), 1).unwrap().0;
        let created = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/gambling/rooms/action",
                json!({"request_id":"expire","action":"create","game":"roulette","stake":10}),
                Some(&host),
            )
            .await,
        )
        .await;
        {
            let db = app.db.lock().unwrap();
            db.execute(
                "UPDATE gambling_rooms SET expires=0 WHERE id=?1",
                [created["room"]["id"].as_str().unwrap()],
            )
            .unwrap();
        }
        value(
            call(
                app.clone(),
                "GET",
                "/api/v1/gambling/rooms",
                Value::Null,
                Some(&host),
            )
            .await,
        )
        .await;
        assert_eq!(wallet(&app.db.lock().unwrap(), 1).unwrap().0, before);
    }
}
