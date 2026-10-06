use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[derive(Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    pub id: String,
    pub access: String,
    pub refresh: String,
    pub expires: u64,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn account() -> Result<Account> {
    serde_json::from_slice(&crate::credentials::load("minecraft-account")?)
        .context("Sign in to Microsoft first")
}
pub fn sign_out() {
    crate::credentials::remove("minecraft-account");
}
fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}
fn response(r: reqwest::blocking::Response) -> Result<Value> {
    anyhow::ensure!(
        r.status().is_success(),
        "Microsoft / Minecraft authentication failed ({}). Check Canna's application registration and Minecraft ownership.",
        r.status()
    );
    Ok(r.json()?)
}
fn exchange(ms: &Value) -> Result<Account> {
    let client = client()?;
    let msa = ms["access_token"]
        .as_str()
        .context("Microsoft did not return an access token")?;
    let xbox=response(client.post("https://user.auth.xboxlive.com/user/authenticate").json(&json!({"Properties":{"AuthMethod":"RPS","SiteName":"user.auth.xboxlive.com","RpsTicket":format!("d={msa}")},"RelyingParty":"http://auth.xboxlive.com","TokenType":"JWT"})).send()?)?;
    let xsts=response(client.post("https://xsts.auth.xboxlive.com/xsts/authorize").json(&json!({"Properties":{"SandboxId":"RETAIL","UserTokens":[xbox["Token"]]},"RelyingParty":"rp://api.minecraftservices.com/","TokenType":"JWT"})).send()?)?;
    let uhs = xsts["DisplayClaims"]["xui"][0]["uhs"]
        .as_str()
        .context("Xbox account authorization failed")?;
    let xsts_token = xsts["Token"]
        .as_str()
        .context("Xbox authorization token missing")?;
    let mc = response(
        client
            .post("https://api.minecraftservices.com/authentication/login_with_xbox")
            .json(&json!({"identityToken":format!("XBL3.0 x={uhs};{xsts_token}")}))
            .send()?,
    )?;
    let access = mc["access_token"]
        .as_str()
        .context("Minecraft access was not granted")?;
    let entitlements = response(
        client
            .get("https://api.minecraftservices.com/entitlements/mcstore")
            .bearer_auth(access)
            .send()?,
    )?;
    anyhow::ensure!(
        entitlements["items"]
            .as_array()
            .is_some_and(|a| !a.is_empty()),
        "This Microsoft account does not own Minecraft Java Edition"
    );
    let profile = response(
        client
            .get("https://api.minecraftservices.com/minecraft/profile")
            .bearer_auth(access)
            .send()?,
    )?;
    let account = Account {
        name: profile["name"]
            .as_str()
            .context("Minecraft profile missing")?
            .into(),
        id: profile["id"]
            .as_str()
            .context("Minecraft UUID missing")?
            .into(),
        access: access.into(),
        refresh: ms["refresh_token"].as_str().unwrap_or_default().into(),
        expires: now() + mc["expires_in"].as_u64().unwrap_or(3600),
    };
    crate::credentials::save("minecraft-account", &serde_json::to_vec(&account)?)?;
    Ok(account)
}
pub fn sign_in(id: &str, progress: impl Fn(String)) -> Result<String> {
    anyhow::ensure!(
        id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'),
        "Enter Canna's public Microsoft Application (client) ID"
    );
    let client = client()?;
    let code = response(
        client
            .post("https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode")
            .form(&[
                ("client_id", id),
                ("scope", "XboxLive.SignIn XboxLive.offline_access"),
            ])
            .send()?,
    )?;
    let user = code["user_code"]
        .as_str()
        .context("Microsoft did not return a sign-in code")?;
    let device = code["device_code"]
        .as_str()
        .context("Microsoft did not start device sign-in")?;
    progress(format!(
        "Open https://www.microsoft.com/link and enter {user}. Only enter the code for the Canna sign-in you just started."
    ));
    let deadline = now() + code["expires_in"].as_u64().unwrap_or(900).min(900);
    let mut interval = code["interval"].as_u64().unwrap_or(5).max(5);
    while now() < deadline {
        std::thread::sleep(Duration::from_secs(interval));
        let r = client
            .post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
            .form(&[
                ("client_id", id),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device),
            ])
            .send()?;
        let ok = r.status().is_success();
        let result: Value = r.json()?;
        if ok {
            return Ok(format!("Signed in as {}", exchange(&result)?.name));
        }
        match result["error"].as_str() {
            Some("authorization_pending") => {}
            Some("slow_down") => interval += 5,
            _ => {
                anyhow::bail!("Microsoft sign-in was declined or expired. Start again when ready.")
            }
        }
    }
    anyhow::bail!("Microsoft sign-in code expired")
}
pub fn ready(id: &str) -> Result<Account> {
    let account = account()?;
    if account.expires > now() + 120 {
        return Ok(account);
    }
    let refresh = response(
        client()?
            .post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
            .form(&[
                ("client_id", id),
                ("grant_type", "refresh_token"),
                ("refresh_token", account.refresh.as_str()),
                ("scope", "XboxLive.SignIn XboxLive.offline_access"),
            ])
            .send()?,
    )?;
    exchange(&refresh)
}
pub fn apply_skin(id: &str, path: &std::path::Path, slim: bool) -> Result<()> {
    let account = ready(id)?;
    let form = reqwest::blocking::multipart::Form::new()
        .text("variant", if slim { "slim" } else { "classic" })
        .part(
            "file",
            reqwest::blocking::multipart::Part::bytes(std::fs::read(path)?)
                .file_name("skin.png")
                .mime_str("image/png")?,
        );
    let response = client()?
        .post("https://api.minecraftservices.com/minecraft/profile/skins")
        .bearer_auth(&account.access)
        .multipart(form)
        .send()?;
    anyhow::ensure!(
        response.status().is_success(),
        "Minecraft refused the skin change ({})",
        response.status()
    );
    Ok(())
}
