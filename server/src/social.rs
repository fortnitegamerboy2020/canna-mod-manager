//! Private friendships and authenticated, encrypted-at-rest direct messages.
//! The service decrypts only for the two participants, never through staff APIs.
use super::*;
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use rand::{RngCore, rngs::OsRng};
use rusqlite::TransactionBehavior;

pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS friendships(low INTEGER NOT NULL REFERENCES users(id),high INTEGER NOT NULL REFERENCES users(id),requester INTEGER NOT NULL REFERENCES users(id),status TEXT NOT NULL CHECK(status IN ('pending','accepted')),created INTEGER NOT NULL,PRIMARY KEY(low,high),CHECK(low<high));
        CREATE TABLE IF NOT EXISTS social_blocks(actor INTEGER NOT NULL REFERENCES users(id),target INTEGER NOT NULL REFERENCES users(id),created INTEGER NOT NULL,PRIMARY KEY(actor,target),CHECK(actor!=target));
        CREATE TABLE IF NOT EXISTS direct_messages(seq INTEGER PRIMARY KEY AUTOINCREMENT,id TEXT UNIQUE NOT NULL,sender INTEGER NOT NULL REFERENCES users(id),recipient INTEGER NOT NULL REFERENCES users(id),created INTEGER NOT NULL,nonce BLOB NOT NULL,ciphertext BLOB NOT NULL,CHECK(sender!=recipient));
        CREATE INDEX IF NOT EXISTS dm_pair ON direct_messages(sender,recipient,seq);
        CREATE INDEX IF NOT EXISTS dm_recipient ON direct_messages(recipient,seq);
        CREATE TABLE IF NOT EXISTS dm_reads(actor INTEGER NOT NULL REFERENCES users(id),peer INTEGER NOT NULL REFERENCES users(id),last_seq INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(actor,peer));
        CREATE TABLE IF NOT EXISTS social_requests(actor INTEGER NOT NULL REFERENCES users(id),request_id TEXT NOT NULL,fingerprint TEXT NOT NULL,response TEXT NOT NULL,created INTEGER NOT NULL,PRIMARY KEY(actor,request_id));")
}
fn member(db: &Connection, id: i64) -> ApiResult<()> {
    if id <= 0
        || !db.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND verified=1 AND banned=0)",
            [id],
            |r| r.get::<_, bool>(0),
        )?
    {
        return Err(ApiError(StatusCode::NOT_FOUND, "Member not found"));
    }
    Ok(())
}
fn blocked(db: &Connection, a: i64, b: i64) -> ApiResult<bool> {
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM social_blocks WHERE actor=?1 AND target=?2 OR actor=?2 AND target=?1)",params![a,b],|r|r.get(0))?)
}
fn friendship(db: &Connection, a: i64, b: i64) -> ApiResult<Option<(String, i64)>> {
    Ok(db
        .query_row(
            "SELECT status,requester FROM friendships WHERE low=?1 AND high=?2",
            params![a.min(b), a.max(b)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}
fn status(db: &Connection, actor: i64, target: i64) -> ApiResult<&'static str> {
    if actor == target {
        return Ok("self");
    }
    member(db, target)?;
    if blocked(db, actor, target)? {
        return Ok("blocked");
    }
    Ok(match friendship(db, actor, target)?.as_ref() {
        Some((s, _)) if s == "accepted" => "friends",
        Some((_, who)) if *who == actor => "outgoing",
        Some(_) => "incoming",
        None => "none",
    })
}
fn once(
    app: &App,
    actor: i64,
    request: &str,
    payload: &Value,
    action: impl FnOnce(&Connection) -> ApiResult<Value>,
) -> ApiResult<axum::Json<Value>> {
    if request.is_empty()
        || request.len() > 80
        || !request
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    {
        return Err(bad("Supply a valid request identity"));
    }
    let fingerprint = digest(&payload.to_string());
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing: Option<(String, String)> = tx
        .query_row(
            "SELECT fingerprint,response FROM social_requests WHERE actor=?1 AND request_id=?2",
            params![actor, request],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((stored, response)) = existing {
        if stored != fingerprint {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "That request identity was already used",
            ));
        }
        return Ok(axum::Json(
            serde_json::from_str(&response).map_err(|_| bad("Stored receipt is unavailable"))?,
        ));
    }
    let result = action(&tx)?;
    tx.execute(
        "INSERT INTO social_requests VALUES(?1,?2,?3,?4,?5)",
        params![actor, request, fingerprint, result.to_string(), now()],
    )?;
    tx.commit()?;
    app.live.hint("notifications");
    Ok(axum::Json(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FriendInput {
    request_id: String,
    target: i64,
    action: String,
}
pub async fn friend_action(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<FriendInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = gambling::mutation_actor(&app, &headers)?;
    app.limits.check(format!("social-action:{actor}"), 60)?;
    once(
        &app,
        actor,
        &input.request_id,
        &json!({"type":"friend","target":input.target,"action":input.action}),
        |db| {
            let target = input.target;
            member(db, target)?;
            if actor == target {
                return Err(bad("Choose another member"));
            }
            let pair = params![actor.min(target), actor.max(target)];
            match input.action.as_str() {
                "request" => {
                    if blocked(db, actor, target)? {
                        return Err(bad("A friendship is unavailable for this member"));
                    }
                    if friendship(db, actor, target)?.is_some() {
                        return Err(ApiError(
                            StatusCode::CONFLICT,
                            "A friendship or request already exists",
                        ));
                    }
                    for user in [actor, target] {
                        let total: i64 = db.query_row(
                            "SELECT count(*) FROM friendships WHERE low=?1 OR high=?1",
                            [user],
                            |r| r.get(0),
                        )?;
                        let pending:i64=db.query_row("SELECT count(*) FROM friendships WHERE (low=?1 OR high=?1) AND status='pending'",[user],|r|r.get(0))?;
                        if total >= 1000 || pending >= 50 {
                            return Err(bad("Friend request capacity reached"));
                        }
                    }
                    db.execute(
                        "INSERT INTO friendships VALUES(?1,?2,?3,'pending',?4)",
                        params![actor.min(target), actor.max(target), actor, now()],
                    )?;
                    notifications::notify(
                        db,
                        target,
                        "friend-request",
                        "You have a new friend request.",
                        "/messages",
                        &format!(
                            "friend:{actor}:{target}:{request}",
                            request = input.request_id
                        ),
                    )?;
                }
                "accept" => {
                    if blocked(db, actor, target)?
                        || friendship(db, actor, target)? != Some(("pending".into(), target))
                    {
                        return Err(bad("There is no incoming request to accept"));
                    }
                    db.execute(
                        "UPDATE friendships SET status='accepted' WHERE low=?1 AND high=?2",
                        pair,
                    )?;
                }
                "remove" => {
                    db.execute("DELETE FROM friendships WHERE low=?1 AND high=?2", pair)?;
                }
                "block" => {
                    db.execute(
                        "INSERT OR IGNORE INTO social_blocks VALUES(?1,?2,?3)",
                        params![actor, target, now()],
                    )?;
                    db.execute("DELETE FROM friendships WHERE low=?1 AND high=?2", pair)?;
                }
                "unblock" => {
                    db.execute(
                        "DELETE FROM social_blocks WHERE actor=?1 AND target=?2",
                        params![actor, target],
                    )?;
                }
                _ => return Err(bad("Choose request, accept, remove, block or unblock")),
            }
            Ok(
                json!({"ok":true,"member_id":actor,"target":target,"status":status(db,actor,target)?}),
            )
        },
    )
}
pub async fn relationship(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(target): Path<i64>,
) -> ApiResult<axum::Json<Value>> {
    let actor = app.auth(&headers)?.0;
    Ok(axum::Json(
        json!({"member_id":actor,"target":target,"status":status(&app.db.lock().unwrap(),actor,target)?}),
    ))
}
pub async fn overview(
    State(app): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    let actor = app.auth(&headers)?.0;
    let db = app.db.lock().unwrap();
    let friends=db.prepare("SELECT u.id,u.username,f.status,f.requester FROM friendships f JOIN users u ON u.id=CASE WHEN f.low=?1 THEN f.high ELSE f.low END WHERE (f.low=?1 OR f.high=?1) AND u.verified=1 AND u.banned=0 ORDER BY f.status,u.username LIMIT 1000")?.query_map([actor],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"username":r.get::<_,String>(1)?,"status":r.get::<_,String>(2)?,"requester":r.get::<_,i64>(3)?})))?.collect::<Result<Vec<_>,_>>()?;
    let blocks=db.prepare("SELECT u.id,u.username FROM social_blocks b JOIN users u ON u.id=b.target WHERE b.actor=?1 ORDER BY b.created DESC LIMIT 1000")?.query_map([actor],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"username":r.get::<_,String>(1)?})))?.collect::<Result<Vec<_>,_>>()?;
    let conversations=db.prepare("SELECT u.id,u.username,max(d.created),sum(CASE WHEN d.recipient=?1 AND d.seq>coalesce(r.last_seq,0) THEN 1 ELSE 0 END) FROM direct_messages d JOIN users u ON u.id=CASE WHEN d.sender=?1 THEN d.recipient ELSE d.sender END LEFT JOIN dm_reads r ON r.actor=?1 AND r.peer=u.id WHERE d.sender=?1 OR d.recipient=?1 GROUP BY u.id ORDER BY max(d.seq) DESC LIMIT 100")?.query_map([actor],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"username":r.get::<_,String>(1)?,"updated":r.get::<_,i64>(2)?,"unread":r.get::<_,i64>(3)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(axum::Json(
        json!({"member_id":actor,"friends":friends,"blocks":blocks,"conversations":conversations}),
    ))
}
fn cipher(app: &App) -> XChaCha20Poly1305 {
    let mut hash = Sha256::new();
    hash.update(b"Canna private messages v1\0");
    hash.update(app.upload_key.as_ref());
    let key: Zeroizing<[u8; 32]> = Zeroizing::new(hash.finalize().into());
    XChaCha20Poly1305::new(key.as_ref().into())
}
fn aad(id: &str, sender: i64, recipient: i64, created: i64) -> String {
    format!("canna-dm-v1:{id}:{sender}:{recipient}:{created}")
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendInput {
    request_id: String,
    target: i64,
    body: String,
    #[serde(default)]
    pack_id: Option<String>,
}
pub async fn send(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<SendInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = gambling::mutation_actor(&app, &headers)?;
    app.limits.check(format!("dm-send:{actor}"), 30)?;
    if input.body.chars().count() > 4000
        || input.body.len() > 16000
        || input.body.contains('\0')
        || input.body.trim().is_empty() && input.pack_id.is_none()
    {
        return Err(bad(
            "Write a message of up to 4,000 characters or attach a shared pack",
        ));
    }
    once(
        &app,
        actor,
        &input.request_id,
        &json!({"type":"dm","target":input.target,"body":input.body,"pack_id":input.pack_id}),
        |db| {
            let target = input.target;
            member(db, target)?;
            if target == actor || status(db, actor, target)? != "friends" {
                return Err(ApiError(
                    StatusCode::FORBIDDEN,
                    "Private messages are available between accepted friends",
                ));
            }
            if let Some(id) = input.pack_id.as_deref() {
                let info = shared_packs::message_embed(db, id, actor)?;
                if info["ready"] != true {
                    return Err(bad(
                        "This pack has unavailable or unapproved mods; share a ready pack",
                    ));
                }
            }
            let count: i64 = db.query_row(
                "SELECT count(*) FROM direct_messages WHERE sender=?1 AND created>?2",
                params![actor, now() - 86400],
                |r| r.get(0),
            )?;
            if count >= 1000 {
                return Err(ApiError(
                    StatusCode::TOO_MANY_REQUESTS,
                    "Daily private message limit reached",
                ));
            }
            let id = Uuid::new_v4().to_string();
            let created = now();
            let mut nonce = [0u8; 24];
            OsRng.fill_bytes(&mut nonce);
            let plaintext = Zeroizing::new(
                serde_json::to_vec(&json!({"body":input.body.trim(),"pack_id":input.pack_id}))
                    .map_err(|_| bad("Message could not be prepared"))?,
            );
            let ciphertext = cipher(&app)
                .encrypt(
                    XNonce::from_slice(&nonce),
                    Payload {
                        msg: &plaintext,
                        aad: aad(&id, actor, target, created).as_bytes(),
                    },
                )
                .map_err(|_| bad("Message encryption failed"))?;
            db.execute("INSERT INTO direct_messages(id,sender,recipient,created,nonce,ciphertext) VALUES(?1,?2,?3,?4,?5,?6)",params![id,actor,target,created,nonce.as_slice(),ciphertext])?;
            // Notification records contain no message excerpt or attached pack.
            notifications::notify(
                db,
                target,
                "private-message",
                "You have a new private message.",
                &format!("/messages/{actor}"),
                &format!("dm:{id}"),
            )?;
            let seq = db.last_insert_rowid();
            Ok(json!({"ok":true,"member_id":actor,"id":id,"seq":seq,"created":created}))
        },
    )
}
#[derive(Deserialize)]
pub struct MessageQuery {
    #[serde(default)]
    after: i64,
    #[serde(default)]
    before: i64,
}
pub async fn messages(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(peer): Path<i64>,
    Query(query): Query<MessageQuery>,
) -> ApiResult<axum::Json<Value>> {
    let actor = app.auth(&headers)?.0;
    if peer <= 0
        || peer == actor
        || query.after < 0
        || query.before < 0
        || query.after > 0 && query.before > 0
    {
        return Err(bad("Invalid conversation"));
    }
    let db = app.db.lock().unwrap();
    member(&db, peer)?;
    // There is no target message ID API: the authenticated actor is always one
    // end of the pair, including for Owner accounts.
    let order = if query.after == 0 { "DESC" } else { "ASC" };
    let rows=db.prepare(&format!("SELECT seq,id,sender,recipient,created,nonce,ciphertext FROM direct_messages WHERE ((sender=?1 AND recipient=?2) OR (sender=?2 AND recipient=?1)) AND seq>?3 AND (?4=0 OR seq<?4) ORDER BY seq {order} LIMIT 101"))?.query_map(params![actor,peer,query.after,query.before],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?,r.get::<_,Vec<u8>>(5)?,r.get::<_,Vec<u8>>(6)?)))?.collect::<Result<Vec<_>,_>>()?;
    let has_more = rows.len() > 100;
    let mut rows = rows.into_iter().take(100).collect::<Vec<_>>();
    if query.after == 0 {
        rows.reverse();
    }
    let mut messages = Vec::new();
    for (seq, id, sender, recipient, created, nonce, ciphertext) in rows {
        if nonce.len() != 24 {
            return Err(bad("Stored message integrity check failed"));
        }
        let bytes = Zeroizing::new(
            cipher(&app)
                .decrypt(
                    XNonce::from_slice(&nonce),
                    Payload {
                        msg: &ciphertext,
                        aad: aad(&id, sender, recipient, created).as_bytes(),
                    },
                )
                .map_err(|_| bad("Stored message integrity check failed"))?,
        );
        let content: Value =
            serde_json::from_slice(&bytes).map_err(|_| bad("Stored message could not be read"))?;
        let pack = content["pack_id"].as_str().map(|id| {
            shared_packs::message_embed(&db, id, actor).unwrap_or_else(
                |_| json!({"id":id,"available":false,"name":"Shared pack unavailable"}),
            )
        });
        messages.push(json!({"seq":seq,"id":id,"sender":sender,"created":created,"body":content["body"],"pack":pack}));
    }
    let next = messages
        .last()
        .and_then(|m| m["seq"].as_i64())
        .unwrap_or(query.after);
    Ok(axum::Json(
        json!({"member_id":actor,"peer":peer,"messages":messages,"next":next,"has_more":has_more,"can_send":status(&db,actor,peer)?=="friends"}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadInput {
    peer: i64,
    seq: i64,
}
pub async fn mark_read(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<ReadInput>,
) -> ApiResult<axum::Json<Value>> {
    let actor = gambling::mutation_actor(&app, &headers)?;
    let db = app.db.lock().unwrap();
    if input.peer<=0 || input.seq<=0 || !db.query_row("SELECT EXISTS(SELECT 1 FROM direct_messages WHERE seq=?1 AND ((sender=?2 AND recipient=?3) OR (sender=?3 AND recipient=?2)))",params![input.seq,actor,input.peer],|r|r.get::<_,bool>(0))? {return Err(bad("Invalid conversation cursor"));}
    db.execute("INSERT INTO dm_reads VALUES(?1,?2,?3) ON CONFLICT(actor,peer) DO UPDATE SET last_seq=MAX(last_seq,excluded.last_seq)",params![actor,input.peer,input.seq])?;
    Ok(axum::Json(json!({"ok":true,"member_id":actor})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    #[tokio::test]
    async fn messages_require_friendship_hide_plaintext_and_bind_ciphertext_to_pair() {
        let (_dir, app) = fixture();
        let a = account(&app, "alice", false);
        let b = account(&app, "bob", false);
        let owner = account(&app, "owner", true);
        let send = json!({"request_id":"message","target":2,"body":"private secret message"});
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/social/messages",
                send.clone(),
                Some(&a)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        value(
            call(
                app.clone(),
                "POST",
                "/api/v1/social/friends",
                json!({"request_id":"request","target":2,"action":"request"}),
                Some(&a),
            )
            .await,
        )
        .await;
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/social/friends",
                json!({"request_id":"self-accept","target":2,"action":"accept"}),
                Some(&a)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        value(
            call(
                app.clone(),
                "POST",
                "/api/v1/social/friends",
                json!({"request_id":"accept","target":1,"action":"accept"}),
                Some(&b),
            )
            .await,
        )
        .await;
        let result = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/social/messages",
                send.clone(),
                Some(&a),
            )
            .await,
        )
        .await;
        assert_eq!(
            result,
            value(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/social/messages",
                    send,
                    Some(&a)
                )
                .await
            )
            .await
        );
        let read = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/social/messages/1",
                Value::Null,
                Some(&b),
            )
            .await,
        )
        .await;
        assert_eq!(read["messages"][0]["body"], "private secret message");
        let third = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/social/messages/1",
                Value::Null,
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(third["messages"], json!([]));
        {
            let db = app.db.lock().unwrap();
            let (nonce, encrypted): (Vec<u8>, Vec<u8>) = db
                .query_row("SELECT nonce,ciphertext FROM direct_messages", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .unwrap();
            assert_eq!(nonce.len(), 24);
            assert!(!encrypted.windows(7).any(|w| w == b"private"));
            let receipts: String = db
                .query_row(
                    "SELECT group_concat(response) FROM social_requests",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(!receipts.contains("secret"));
            db.execute("UPDATE direct_messages SET recipient=3", [])
                .unwrap();
        }
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/social/messages/1",
                Value::Null,
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
    #[tokio::test]
    async fn blocks_remove_friendship_and_stop_new_messages_but_keep_participant_history() {
        let (_dir, app) = fixture();
        let a = account(&app, "alice", false);
        let b = account(&app, "bob", false);
        {
            let db = app.db.lock().unwrap();
            db.execute("INSERT INTO friendships VALUES(1,2,1,'accepted',0)", [])
                .unwrap();
        }
        value(
            call(
                app.clone(),
                "POST",
                "/api/v1/social/messages",
                json!({"request_id":"hi","target":2,"body":"hello"}),
                Some(&a),
            )
            .await,
        )
        .await;
        value(
            call(
                app.clone(),
                "POST",
                "/api/v1/social/friends",
                json!({"request_id":"block","target":1,"action":"block"}),
                Some(&b),
            )
            .await,
        )
        .await;
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/social/messages",
                json!({"request_id":"blocked","target":2,"body":"cannot send"}),
                Some(&a)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let read = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/social/messages/2",
                Value::Null,
                Some(&a),
            )
            .await,
        )
        .await;
        assert_eq!(read["can_send"], false);
        assert_eq!(read["messages"].as_array().unwrap().len(), 1);
        assert_eq!(
            call(app.clone(), "GET", "/api/v1/social", Value::Null, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
}
