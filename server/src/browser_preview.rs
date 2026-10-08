//! Manual browser fixture, excluded entirely from production builds.
use super::*;

fn review_fixtures(db: &Connection) {
    let source = "using System.IO;\nusing System.Diagnostics;\npublic class PreviewPlugin {\n    public void Awake() { WriteReport(\"user-supplied-directory\"); }\n    public void WriteReport(string directory) {\n        File.WriteAllText(Path.Combine(directory, \"report.png\"), \"inert fixture\");\n    }\n    public void NeverRun() { Process.Start(\"fixture-only\"); }\n}\n";
    let long_source = (1..=12_050)
        .map(|line| {
            if line == 11_005 {
                "public void LaterMethod() { /* late source marker */ }\n".into()
            } else {
                format!("// Inert source fixture line {line}\n")
            }
        })
        .collect::<String>();
    let base = json!({
        "version":"static-7", "coverage_complete":false,
        "note":"Inert browser fixtures; no game code was executed.",
        "files":[
            {"name":"decompiled/Preview.dll/PreviewPlugin.cs","kind":"decompiled","text":source,"origin":"plugins/Preview.dll","language":"C#","decompiler":"ILSpy"},
            {"name":"manifest.json","kind":"source","text":"{\"name\":\"InertPreview\",\"dependencies\":[\"Example-Library-1.0\"]}"},
            {"name":"docs/<img onerror=alert(1)>.md","kind":"source","text":"# Inert preview\nhttps://example.invalid/reference\n"},
            {"name":"src/Large.cs","kind":"source","text":long_source}
        ],
        "inventory":[{"name":"plugins/Preview.dll","size":4096},{"name":"native/Unsupported.dll","size":2048},{"name":"assets/banner.png","size":1024},{"name":"manifest.json","size":64}],
        "findings":[
            {"id":"fixture-write","rule":"filesystem","title":"Diagnostic file write","file":"decompiled/Preview.dll/PreviewPlugin.cs","line":6,"evidence":"File.WriteAllText(Path.Combine(directory, report.png))","accepted":false,"context":{"explanation":"The directory is caller controlled. The extension alone does not establish that a write is safe."}},
            {"id":"fixture-process","rule":"commands","title":"Process launch API","file":"decompiled/Preview.dll/PreviewPlugin.cs","line":8,"evidence":"Process.Start(fixture-only)","accepted":false},
            {"id":"fixture-coverage","rule":"coverage","title":"Native binary has no managed reconstruction","file":"native/Unsupported.dll","line":null,"evidence":"Synthetic native entry, no source preview","accepted":false,"locations":[{"file":"native/Unsupported.dll","line":null}]}
        ],
        "observations":[{"id":"fixture-url","rule":"url-reference","title":"Inert documentation reference","file":"docs/<img onerror=alert(1)>.md","line":2,"evidence":"https://example.invalid/reference"}],
        "engines":{"ilspy":{"status":"complete"},"clamav":{"status":"complete","matches":0}},
        "decompilations":[
            {"input":"plugins/Preview.dll","scope":"binary","language":"C#","tool":"ILSpy","status":"complete","duration_ms":125,"exit_code":0,"generated_files":["decompiled/Preview.dll/PreviewPlugin.cs"],"generated_count":1,"generated_bytes":source.len(),"preview_count":1,"scanned_count":1,"limitations":[]},
            {"input":"native/Unsupported.dll","scope":"binary","language":"native","tool":null,"status":"unsupported","duration_ms":0,"generated_files":[],"generated_count":0,"preview_count":0,"scanned_count":0,"limitations":["Synthetic unsupported native binary; no reconstruction available."]}
        ],
        "limits":{"analysis_seconds":240,"preview_files":500,"preview_text_bytes":8388608}
    });
    for (suffix, name, status, report) in [
        (
            "1",
            "Review workspace · inert browser fixture",
            "complete",
            base,
        ),
        (
            "2",
            "Legacy report · inert browser fixture",
            "complete",
            json!({"version":"static-6","files":[{"name":"Old.cs","kind":"decompiled","text":"public class Old { public void Awake() {} }"}],"findings":[],"inventory":[{"name":"Old.dll","size":1024}]}),
        ),
        (
            "3",
            "Failed analysis · inert browser fixture",
            "failed",
            json!({"version":"static-7","files":[],"findings":[],"engines":{"worker":{"status":"failed","error":"Synthetic permission failure"}}}),
        ),
        (
            "4",
            "Waiting analysis · inert browser fixture",
            "pending",
            json!({"files":[],"findings":[]}),
        ),
    ] {
        let id = format!("11111111-1111-4111-8111-11111111111{suffix}");
        let hash = digest(&id);
        db.execute(
            "INSERT INTO mods VALUES(?1,1,1557740,?2,'fixture','Inert browser-only data',?3,1)",
            params![id, name, hash],
        )
        .unwrap();
        db.execute("INSERT INTO mod_reviews VALUES(?1,0)", [&id])
            .unwrap();
        db.execute(
            "INSERT INTO mod_scans VALUES(?1,?2,?3,?4,?5)",
            params![id, hash, status, report.to_string(), now()],
        )
        .unwrap();
    }
    if let Ok(path) = std::env::var("CANNA_REVIEW_BROWSER_REPORT") {
        let bytes = std::fs::read(path).unwrap();
        assert!(bytes.len() <= 16 * 1024 * 1024);
        let report: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(report.is_object());
        let id = "11111111-1111-4111-8111-111111111115";
        let hash = report["sha256"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| digest(id));
        db.execute("INSERT INTO mods VALUES(?1,1,1557740,'Actual static tools · inert compiled fixture','fixture','Fixture DLL was never executed',?2,1)",params![id,hash]).unwrap();
        db.execute("INSERT INTO mod_reviews VALUES(?1,0)", [id])
            .unwrap();
        db.execute(
            "INSERT INTO mod_scans VALUES(?1,?2,'complete',?3,?4)",
            params![id, hash, report.to_string(), now()],
        )
        .unwrap();
    }
}

fn parse_preview_port(value: Option<&str>) -> Option<u16> {
    match value {
        None => Some(18787),
        Some(value) if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
            value.parse::<u16>().ok().filter(|port| *port != 0)
        }
        Some(_) => None,
    }
}

fn preview_port() -> u16 {
    match std::env::var("CANNA_BROWSER_PREVIEW_PORT") {
        Ok(value) => parse_preview_port(Some(&value)).expect("Preview port must be a nonzero u16"),
        Err(std::env::VarError::NotPresent) => 18787,
        Err(error) => panic!("Invalid preview port: {error}"),
    }
}

async fn localhost_transport(mut request: Request, next: Next) -> Response {
    let local_origin = format!("http://localhost:{}", preview_port());
    if request
        .headers()
        .get("origin")
        .and_then(|v| v.to_str().ok())
        == Some(local_origin.as_str())
    {
        request
            .headers_mut()
            .insert("origin", "https://cannamods.vip".parse().unwrap());
    }
    let mut response = next.run(request).await;
    let cookies: Vec<_> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok().map(|s| s.replace("; Secure", "")))
        .collect();
    response.headers_mut().remove("set-cookie");
    for cookie in cookies {
        response
            .headers_mut()
            .append("set-cookie", cookie.parse().unwrap());
    }
    response
}

fn participant_accounts(db: &Connection) -> rusqlite::Result<()> {
    for index in 0..202 {
        db.execute(
            "INSERT INTO users(username,password,verified,role,email) VALUES(?1,'fixture',1,'member',?2)",
            params![format!("PreviewBettor{index:03}"),format!("preview-bettor-{index}@example.test")],
        )?;
    }
    Ok(())
}

fn round_participants(db: &Connection, time: i64) -> rusqlite::Result<()> {
    let round: Option<(i64, i64)> = db
        .query_row(
            "SELECT id,start_ms FROM gambling_crash_rounds ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((id, start)) = round {
        // Inert display rows only: no request IDs, stake debits or payout credits.
        // No automatic thresholds are set, so normal advance cannot award money.
        db.execute("INSERT OR IGNORE INTO gambling_crash_bets(round_id,user_id,stake) SELECT ?1,id,CASE id WHEN 1 THEN 125 WHEN 2 THEN 250 WHEN 3 THEN 75 ELSE 10+(id%90) END FROM users WHERE id BETWEEN 1 AND 205",[id])?;
        for (actor, multiplier) in [(1, 120_i64), (2, 200)] {
            let cashout_at = start + ((multiplier as f64 / 100.0).ln() * 10_000.0).ceil() as i64;
            if time >= cashout_at {
                db.execute("UPDATE gambling_crash_bets SET status='won',cashout_multiplier=?1,cashout_at_ms=?2 WHERE round_id=?3 AND user_id=?4 AND status='pending'",params![multiplier,cashout_at,id,actor])?;
            }
        }
    }
    Ok(())
}

async fn participant_display_fixture(
    State(app): State<Shared>,
    request: Request,
    next: Next,
) -> Response {
    if request.method() == axum::http::Method::GET
        && matches!(
            request.uri().path(),
            "/api/v1/gambling" | "/api/v1/admin/gambling"
        )
    {
        let mut db = app.db.lock().unwrap();
        let tx = db.transaction().unwrap();
        round_participants(&tx, now() * 1000).unwrap();
        tx.commit().unwrap();
    }
    next.run(request).await
}

#[tokio::test]
#[ignore = "manual isolated browser preview; requires CANNA_BROWSER_PREVIEW=1"]
async fn browser_preview_fixture() {
    assert_eq!(std::env::var("CANNA_BROWSER_PREVIEW").as_deref(), Ok("1"));
    let (_dir, app) = crate::tests::fixture();
    let owner = crate::tests::account(&app, "PreviewOwner", true);
    let member = crate::tests::account(&app, "PreviewMember", false);
    let admin = crate::tests::account(&app, "PreviewAdmin", false);
    {
        let mut db = app.db.lock().unwrap();
        let tx = db.transaction().unwrap();
        let db = &tx;
        db.execute("UPDATE users SET role='admin',admin=1 WHERE id=3", [])
            .unwrap();
        db.execute("INSERT INTO user_roles VALUES(1,'beta')", [])
            .unwrap();
        db.execute(
            "UPDATE users SET email=lower(username)||'@example.test',verified=1",
            [],
        )
        .unwrap();
        review_fixtures(db);
        // Browser-only flight and participant demo: upcoming rounds reach 50x.
        // These synthetic display rows never debit stakes or credit payouts.
        db.execute("UPDATE gambling_config SET mode='controlled',paused=0", [])
            .unwrap();
        for _ in 0..20 {
            db.execute(
                "INSERT INTO gambling_crash_queue(multiplier) VALUES(5000)",
                [],
            )
            .unwrap();
        }
        participant_accounts(db).unwrap();
        let start = now() * 1000 - 5000;
        let crash = start + (50_f64.ln() * 10_000.0).ceil() as i64;
        db.execute("INSERT INTO gambling_crash_rounds(created_ms,start_ms,crash_ms,multiplier,mode) VALUES(?1,?2,?3,5000,'controlled')",params![start-10_000,start,crash]).unwrap();
        round_participants(db, now() * 1000).unwrap();
        let catalog: Value =
            serde_json::from_str(include_str!("../web/cosmetics/catalog.json")).unwrap();
        let items = catalog["items"].as_array().unwrap();
        let animated = items
            .iter()
            .find(|item| {
                item["collection"] == "bo2" && item["kind"] == "banner" && item["animated"] == true
            })
            .expect("Browser QA needs an owned animated BO2 calling card");
        assert!(
            animated["poster_asset"]
                .as_str()
                .is_some_and(|asset| asset.starts_with("/api/v1/cosmetics/assets/"))
        );
        assert_ne!(animated["poster_asset"], animated["asset"]);
        let frame = items
            .iter()
            .find(|item| item["collection"] == "frames" && item["kind"] == "frame")
            .unwrap();
        let mw2 = items
            .iter()
            .find(|item| item["collection"] == "mw2" && item["kind"] == "banner")
            .unwrap();
        for id in 1..=3 {
            db.execute("INSERT INTO bot_wallets(user_id,balance) VALUES(?1,10000) ON CONFLICT(user_id) DO UPDATE SET balance=10000",[id]).unwrap();
            db.execute(
                "INSERT INTO gambling_cosmetics VALUES(?1,'frame-canna-leaf',1)",
                [id],
            )
            .unwrap();
            db.execute(
                "INSERT INTO gambling_cosmetics VALUES(?1,'banner-canna-night',1)",
                [id],
            )
            .unwrap();
            db.execute(
                "INSERT INTO gambling_cosmetics VALUES(?1,'mw2-blunttrauma-a89c363e',1)",
                [id],
            )
            .unwrap();
            for item in [animated, frame, mw2] {
                db.execute(
                    "INSERT OR IGNORE INTO gambling_cosmetics VALUES(?1,?2,1)",
                    params![id, item["id"].as_str().unwrap()],
                )
                .unwrap();
            }
        }
        tx.commit().unwrap();
    }
    let outbox_app = app.clone();
    let fixture_app = app.clone();
    let mut preview = router(app);
    for (name, session, destination) in [
        (
            "review",
            owner.clone(),
            "/review/mods/11111111-1111-4111-8111-111111111111",
        ),
        (
            "tools",
            owner.clone(),
            "/review/mods/11111111-1111-4111-8111-111111111115",
        ),
        ("owner", owner, "/admin"),
        ("member", member, "/gambling"),
        ("admin", admin, "/admin"),
    ] {
        preview=preview.route(&format!("/__preview/{name}"),get(move || { let session=session.clone();async move {
            ([("set-cookie",format!("canna_session={session}; Path=/; HttpOnly; SameSite=Strict; Max-Age=3600"))],axum::response::Redirect::to(destination))
        }}));
    }
    preview = preview.route(
        "/__preview/mail",
        get(move || {
            let app = outbox_app.clone();
            async move {
                let messages = match &app.mail {
                    email::Mailer::Test(outbox) => outbox.lock().unwrap().clone(),
                    _ => Vec::new(),
                };
                axum::Json(json!({"fixture_only":true,"messages":messages}))
            }
        }),
    );
    preview = preview.layer(middleware::from_fn_with_state(
        fixture_app,
        participant_display_fixture,
    ));
    preview = preview.layer(middleware::from_fn(localhost_transport));
    let port = preview_port();
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    println!("Isolated browser fixture ready on loopback port{port}; no production data");
    axum::serve(listener, preview).await.unwrap();
}

#[test]
fn preview_port_requires_strict_nonzero_u16_and_defaults_to_existing_port() {
    assert_eq!(parse_preview_port(None), Some(18787));
    assert_eq!(parse_preview_port(Some("18888")), Some(18888));
    assert_eq!(parse_preview_port(Some("65535")), Some(65535));
    for invalid in [
        "", "0", "65536", "-1", "+18888", " 18888", "18888 ", "18x88",
    ] {
        assert_eq!(parse_preview_port(Some(invalid)), None);
    }
}

#[test]
fn participant_display_seed_has_no_wallet_or_role_side_effects() {
    let (_dir, app) = crate::tests::fixture();
    for name in ["OwnerSample", "MemberSample", "AdminSample"] {
        crate::tests::account(&app, name, false);
    }
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction().unwrap();
    participant_accounts(&tx).unwrap();
    tx.execute(
        "INSERT INTO bot_wallets(user_id,balance) VALUES(1,10000)",
        [],
    )
    .unwrap();
    let roles_before: i64 = tx
        .query_row("SELECT count(*) FROM user_roles", [], |r| r.get(0))
        .unwrap();
    tx.execute("INSERT INTO gambling_crash_rounds(created_ms,start_ms,crash_ms,multiplier,mode) VALUES(90000,100000,139121,5000,'controlled')",[]).unwrap();
    round_participants(&tx, 105000).unwrap();
    round_participants(&tx, 108000).unwrap();
    let count: i64 = tx
        .query_row("SELECT count(*) FROM gambling_crash_bets", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 205);
    let manual: (String,i64,i64) = tx.query_row("SELECT status,cashout_multiplier,cashout_at_ms FROM gambling_crash_bets WHERE user_id=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(manual, ("won".into(), 120, 101824));
    let second: (String,i64,i64) = tx.query_row("SELECT status,cashout_multiplier,cashout_at_ms FROM gambling_crash_bets WHERE user_id=2",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(second, ("won".into(), 200, 106932));
    assert_eq!(tx.query_row("SELECT count(*) FROM gambling_crash_bets WHERE status='pending' AND auto_multiplier IS NULL",[],|r|r.get::<_,i64>(0)).unwrap(),203);
    assert_eq!(
        tx.query_row("SELECT SUM(payout) FROM gambling_crash_bets", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        tx.query_row("SELECT balance FROM bot_wallets WHERE user_id=1", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        10000
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM user_roles", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        roles_before
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM gambling_requests", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn animated_cosmetic_profile_preserves_animation_and_poster_metadata() {
    let (_dir, app) = crate::tests::fixture();
    let auth = crate::tests::account(&app, "AnimatedPreviewCollector", false);
    let catalog: Value =
        serde_json::from_str(include_str!("../web/cosmetics/catalog.json")).unwrap();
    let animated = catalog["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["collection"] == "bo2" && item["animated"] == true && item["kind"] == "banner"
        })
        .expect("Animated BO2 fixture catalog entry");
    let id = animated["id"].as_str().unwrap();
    {
        let db = app.db.lock().unwrap();
        db.execute("INSERT INTO gambling_cosmetics VALUES(1,?1,1)", [id])
            .unwrap();
        db.execute(
            "INSERT INTO gambling_equipped(user_id,banner) VALUES(1,?1)",
            [id],
        )
        .unwrap();
    }
    let profile = crate::tests::value(
        crate::tests::call(
            app.clone(),
            "GET",
            "/api/v1/profiles/1",
            Value::Null,
            Some(&auth),
        )
        .await,
    )
    .await;
    let banner = &profile["cosmetics"]["banner"];
    assert_eq!(banner, animated);
    assert_eq!(banner["animated"], true);
    assert!(banner["frame_count"].as_u64().unwrap() > 1);
    let poster = banner["poster_asset"].as_str().unwrap();
    assert_ne!(banner["asset"], banner["poster_asset"]);
    assert_eq!(
        crate::tests::call(app, "GET", poster, Value::Null, Some(&auth))
            .await
            .status(),
        StatusCode::OK
    );
}
