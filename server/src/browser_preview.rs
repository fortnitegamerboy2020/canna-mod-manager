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

async fn localhost_transport(mut request: Request, next: Next) -> Response {
    if request
        .headers()
        .get("origin")
        .and_then(|v| v.to_str().ok())
        == Some("http://localhost:18787")
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

#[tokio::test]
#[ignore = "manual isolated browser preview; requires CANNA_BROWSER_PREVIEW=1"]
async fn browser_preview_fixture() {
    assert_eq!(std::env::var("CANNA_BROWSER_PREVIEW").as_deref(), Ok("1"));
    let (_dir, app) = crate::tests::fixture();
    let owner = crate::tests::account(&app, "PreviewOwner", true);
    let member = crate::tests::account(&app, "PreviewMember", false);
    let admin = crate::tests::account(&app, "PreviewAdmin", false);
    {
        let db = app.db.lock().unwrap();
        db.execute("UPDATE users SET role='admin',admin=1 WHERE id=3", [])
            .unwrap();
        db.execute("INSERT INTO user_roles VALUES(1,'beta')", [])
            .unwrap();
        db.execute(
            "UPDATE users SET email=lower(username)||'@example.test',verified=1",
            [],
        )
        .unwrap();
        review_fixtures(&db);
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
        }
    }
    let outbox_app = app.clone();
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
    preview = preview.layer(middleware::from_fn(localhost_transport));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:18787")
        .await
        .unwrap();
    println!("Isolated browser fixture ready on loopback port18787; no production data");
    axum::serve(listener, preview).await.unwrap();
}
