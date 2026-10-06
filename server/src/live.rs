use super::*;
use axum::response::sse::{Event, KeepAlive, Sse};
use std::{collections::HashMap, convert::Infallible, time::Duration};
use tokio::sync::broadcast;

pub struct Live {
    sender: broadcast::Sender<Value>,
    clients: Mutex<HashMap<i64, usize>>,
    gate: Arc<Semaphore>,
}
impl Live {
    pub fn new() -> Self {
        Self {
            sender: broadcast::channel(128).0,
            clients: Mutex::new(HashMap::new()),
            gate: Arc::new(Semaphore::new(64)),
        }
    }
}
struct Subscription {
    app: Shared,
    user: i64,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
impl Drop for Subscription {
    fn drop(&mut self) {
        let mut clients = self.app.live.clients.lock().unwrap();
        if let Some(count) = clients.get_mut(&self.user) {
            *count -= 1;
            if *count == 0 {
                clients.remove(&self.user);
            }
        }
    }
}
pub async fn events(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<Response> {
    let (user, _) = app.auth(&headers)?;
    let permit = app.live.gate.clone().try_acquire_owned().map_err(|_| {
        ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Live connection limit reached",
        )
    })?;
    {
        let mut clients = app.live.clients.lock().unwrap();
        let count = clients.entry(user).or_default();
        if *count >= 4 {
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "Keep at most four live community tabs open",
            ));
        }
        *count += 1;
    }
    let mut receiver = app.live.sender.subscribe();
    let guard = Subscription {
        app: app.clone(),
        user,
        _permit: permit,
    };
    let stream = async_stream::stream! {
        let _guard=guard;
        yield Ok::<Event,Infallible>(Event::default().event("ready").data("{}"));
        let mut check=tokio::time::interval(Duration::from_secs(10));
        check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let change=tokio::select! {
                received=receiver.recv()=>match received {
                    Ok(value)=>Some(value),
                    Err(broadcast::error::RecvError::Lagged(_))=>Some(json!({"kind":"refresh"})),
                    Err(broadcast::error::RecvError::Closed)=>break,
                },
                _=check.tick()=>None,
            };
            if app.auth(&headers).is_err() {
                yield Ok(Event::default().event("auth-expired").data("{}"));break;
            }
            if let Some(change)=change {yield Ok(Event::default().event("change").data(change.to_string()));}
            else {yield Ok(Event::default().comment("session checked"));}
        }
    };
    let mut response = Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response();
    response.headers_mut().insert(
        "cache-control",
        axum::http::HeaderValue::from_static("no-store"),
    );
    Ok(response)
}
pub async fn publish(State(app): State<Shared>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let mutation = matches!(
        request.method().as_str(),
        "POST" | "DELETE" | "PATCH" | "PUT"
    );
    let hint = if !mutation {
        None
    } else if path == "/api/v1/verify-email" {
        Some(("members", "joined"))
    } else if path.starts_with("/api/v1/topics") || path.starts_with("/api/v1/posts/") {
        Some((
            "topics",
            if path == "/api/v1/topics" {
                "created"
            } else {
                "updated"
            },
        ))
    } else if path == "/api/v1/admin/sections/apply" {
        Some(("sections", "updated"))
    } else if path.starts_with("/api/v1/profiles/")
        || path.starts_with("/api/v1/profile-comments/")
        || path.starts_with("/api/v1/admin/users/")
        || path == "/api/v1/admin/transfer-owner"
    {
        Some(("members", "updated"))
    } else if path.starts_with("/api/v1/mods") || path.starts_with("/api/v1/packs") {
        Some(("library", "updated"))
    } else {
        None
    };
    let response = next.run(request).await;
    if response.status().is_success()
        && let Some((kind, action)) = hint
    {
        let _ = app.live.sender.send(json!({"kind":kind,"action":action}));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture};
    use futures_util::StreamExt;
    #[tokio::test]
    async fn updates_are_pushed_only_after_success_and_revoked_sessions_are_closed() {
        let (_dir, app) = fixture();
        let watcher = account(&app, "watcher", false);
        let writer = account(&app, "writer", false);
        assert_eq!(
            call(app.clone(), "GET", "/api/v1/events", Value::Null, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let response = call(
            app.clone(),
            "GET",
            "/api/v1/events",
            Value::Null,
            Some(&watcher),
        )
        .await;
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        let mut stream = response.into_body().into_data_stream();
        let ready = stream.next().await.unwrap().unwrap();
        assert!(String::from_utf8_lossy(&ready).contains("ready"));
        let mut receiver = app.live.sender.subscribe();
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/topics",
                json!({"title":"Bad","body":"x","category":"missing","app_id":1}),
                Some(&writer)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert!(receiver.try_recv().is_err());
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/topics",
                json!({"title":"Live question","body":"Hello","category":"help","app_id":1}),
                Some(&writer)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(receiver.recv().await.unwrap()["kind"], "topics");
        loop {
            let pushed = tokio::time::timeout(Duration::from_secs(1), stream.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if String::from_utf8_lossy(&pushed).contains("topics") {
                break;
            }
        }
        app.db
            .lock()
            .unwrap()
            .execute("DELETE FROM sessions WHERE hash=?1", [digest(&watcher)])
            .unwrap();
        let _ = app.live.sender.send(json!({"kind":"topics"}));
        let pushed = stream.next().await.unwrap().unwrap();
        assert!(String::from_utf8_lossy(&pushed).contains("auth-expired"));
        assert!(stream.next().await.is_none());
        assert!(app.live.clients.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn live_connections_are_limited_and_release_capacity_on_disconnect() {
        let (_dir, app) = fixture();
        let member = account(&app, "member", false);
        let mut responses = Vec::new();
        for _ in 0..4 {
            let response = call(
                app.clone(),
                "GET",
                "/api/v1/events",
                Value::Null,
                Some(&member),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            responses.push(response);
        }
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/events",
                Value::Null,
                Some(&member)
            )
            .await
            .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        drop(responses);
        assert!(app.live.clients.lock().unwrap().is_empty());
        assert_eq!(
            call(app, "GET", "/api/v1/events", Value::Null, Some(&member))
                .await
                .status(),
            StatusCode::OK
        );
    }
    #[tokio::test]
    async fn newly_verified_members_emit_a_join_hint_without_identity_or_email() {
        let (_dir, app) = fixture();
        let _ = account(&app, "watcher", false);
        let _ = account(&app, "newmember", false);
        {
            let db = app.db.lock().unwrap();
            db.execute("UPDATE users SET verified=0 WHERE id=2", [])
                .unwrap();
            db.execute("INSERT INTO codes(challenge,user_id,hash,kind,expires) VALUES('join',2,?1,'verify',?2)",params![email::code_hash("join","123456"),now()+600]).unwrap();
        }
        let mut receiver = app.live.sender.subscribe();
        assert_eq!(
            call(
                app,
                "POST",
                "/api/v1/verify-email",
                json!({"challenge":"join","code":"123456","password":""}),
                None
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            receiver.recv().await.unwrap(),
            json!({"kind":"members","action":"joined"})
        );
    }
}
