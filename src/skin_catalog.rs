use anyhow::{Context, Result};
use base64::Engine;
use scraper::{Html, Selector};
use std::io::Read;

pub struct SkinResult {
    pub title: String,
    pub source: String,
    pub page: String,
    pub bytes: Vec<u8>,
}
pub struct SearchResult {
    pub skins: Vec<SkinResult>,
    pub messages: Vec<String>,
}
struct Listing {
    title: String,
    page: String,
    download: String,
}
fn allowed(raw: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(raw)?;
    anyhow::ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none(),
        "Invalid skin URL"
    );
    anyhow::ensure!(
        matches!(
            url.host_str(),
            Some(
                "www.minecraftskins.net"
                    | "minecraftskins.net"
                    | "skinsmc.org"
                    | "www.skinsmc.org"
                    | "skinsmc.s3.us-east-2.amazonaws.com"
                    | "www.minecraftskins.com"
            )
        ),
        "Unsupported skin host"
    );
    Ok(url)
}
fn get(client: &reqwest::blocking::Client, raw: &str, limit: u64) -> Result<Vec<u8>> {
    let mut url = allowed(raw)?;
    for _ in 0..4 {
        let response = client.get(url.clone()).send()?;
        if response.status().is_redirection() {
            url = allowed(
                url.join(
                    response
                        .headers()
                        .get("location")
                        .context("Missing skin redirect")?
                        .to_str()?,
                )?
                .as_str(),
            )?;
            continue;
        }
        anyhow::ensure!(
            response.status().is_success(),
            "Provider returned HTTP {}",
            response.status()
        );
        let mut bytes = Vec::new();
        response.take(limit + 1).read_to_end(&mut bytes)?;
        anyhow::ensure!(
            bytes.len() as u64 <= limit,
            "Skin response exceeded its size limit"
        );
        return Ok(bytes);
    }
    anyhow::bail!("Too many skin redirects")
}
pub fn search_url(source: &str, query: &str) -> Result<String> {
    let base = match source {
        "MinecraftSkins.net" => "https://www.minecraftskins.net/search/",
        "SkinsMC" => "https://skinsmc.org/search/",
        "Skindex" => "https://www.minecraftskins.com/search/skin/",
        _ => anyhow::bail!("Unknown skin source"),
    };
    let mut url = reqwest::Url::parse(base)?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("Invalid skin search URL"))?
        .pop_if_empty()
        .push(query);
    if source == "Skindex" {
        url.path_segments_mut().unwrap().push("1").push("");
    }
    Ok(url.into())
}
fn listings(source: &str, raw: &str) -> Result<Vec<Listing>> {
    let document = Html::parse_document(raw);
    let mut out = Vec::new();
    let select = |s: &str| Selector::parse(s).map_err(|_| anyhow::anyhow!("Invalid skin selector"));
    match source {
        "MinecraftSkins.net" => {
            for card in document.select(&select(".result .card")?).take(8) {
                let Some(link) = card
                    .select(&select("a.panel-link")?)
                    .next()
                    .and_then(|a| a.value().attr("href"))
                else {
                    continue;
                };
                let url = allowed(
                    reqwest::Url::parse("https://www.minecraftskins.net")?
                        .join(link)?
                        .as_str(),
                )?;
                let slug = url.path().trim_matches('/');
                if slug.is_empty()
                    || !slug
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
                {
                    continue;
                }
                let title = card
                    .select(&select(".card-title")?)
                    .next()
                    .map(|e| e.text().collect::<String>())
                    .unwrap_or_else(|| slug.into());
                out.push(Listing {
                    title: title.trim().chars().take(100).collect(),
                    page: url.to_string(),
                    download: format!("https://www.minecraftskins.net/{slug}/download"),
                });
            }
        }
        "SkinsMC" => {
            for card in document.select(&select("a[href^='/skin/']")?) {
                let Some(image) = card.select(&select("img.renderedskin")?).next() else {
                    continue;
                };
                let Some(encoded) = image
                    .value()
                    .attr("src")
                    .and_then(|s| s.strip_prefix("/skinrender/"))
                else {
                    continue;
                };
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .or_else(|_| {
                        base64::engine::general_purpose::STANDARD_NO_PAD.decode(encoded)
                    })?;
                let download = String::from_utf8(raw)?;
                let url = allowed(&download)?;
                if url.host_str() != Some("skinsmc.s3.us-east-2.amazonaws.com") {
                    continue;
                }
                let title = card
                    .select(&select(".skin-name")?)
                    .next()
                    .map(|e| e.text().collect::<String>())
                    .unwrap_or_else(|| "Minecraft skin".into());
                let page = allowed(
                    reqwest::Url::parse("https://skinsmc.org")?
                        .join(card.value().attr("href").unwrap_or("/"))?
                        .as_str(),
                )?
                .to_string();
                out.push(Listing {
                    title: title.trim().chars().take(100).collect(),
                    page,
                    download,
                });
                if out.len() == 8 {
                    break;
                }
            }
        }
        "Skindex" => {
            for card in document.select(&select(".skin-list li")?).take(8) {
                let Some(link) = card.select(&select("a[href^='/skin/']")?).next() else {
                    continue;
                };
                let page = allowed(
                    reqwest::Url::parse("https://www.minecraftskins.com")?
                        .join(link.value().attr("href").unwrap_or("/"))?
                        .as_str(),
                )?;
                let Some(id) = page
                    .path_segments()
                    .and_then(|mut s| s.nth(1))
                    .filter(|id| id.bytes().all(|b| b.is_ascii_digit()))
                else {
                    continue;
                };
                let title = card
                    .select(&select(".title")?)
                    .next()
                    .map(|e| e.text().collect::<String>())
                    .unwrap_or_else(|| format!("Skin {id}"));
                out.push(Listing {
                    title: title.trim().chars().take(100).collect(),
                    page: page.to_string(),
                    download: format!("https://www.minecraftskins.com/skin/{id}/download/"),
                });
            }
        }
        _ => anyhow::bail!("Unknown skin source"),
    }
    Ok(out)
}
fn provider(source: &str, query: &str) -> Result<Vec<SkinResult>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(std::time::Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Canna-Mod-Manager/0.2.7 skin browser")
        .build()?;
    let bytes = get(&client, &search_url(source, query)?, 3 * 1024 * 1024)?;
    let raw = String::from_utf8(bytes)?;
    let mut out = Vec::new();
    for item in listings(source, &raw)? {
        let Ok(bytes) = get(&client, &item.download, 2 * 1024 * 1024) else {
            continue;
        };
        if crate::skins::validate_png(&bytes).is_err() {
            continue;
        }
        out.push(SkinResult {
            title: item.title,
            source: source.into(),
            page: item.page,
            bytes,
        });
    }
    Ok(out)
}
pub fn search(query: &str, source: &str) -> SearchResult {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > 80 {
        return SearchResult {
            skins: Vec::new(),
            messages: vec!["Enter a search of 1–80 characters.".into()],
        };
    }
    let sources: Vec<_> = ["MinecraftSkins.net", "SkinsMC", "Skindex"]
        .into_iter()
        .filter(|s| source.is_empty() || *s == source)
        .collect();
    std::thread::scope(|scope| {
        let jobs: Vec<_> = sources
            .iter()
            .map(|name| (*name, scope.spawn(move || provider(name, query))))
            .collect();
        let mut result = SearchResult {
            skins: Vec::new(),
            messages: Vec::new(),
        };
        for (name, job) in jobs {
            match job.join() {
                Ok(Ok(mut skins)) => {
                    result
                        .messages
                        .push(format!("{name}: {} skins", skins.len()));
                    result.skins.append(&mut skins);
                }
                Ok(Err(error)) => result.messages.push(format!(
                    "{name}: {error}. You can still search this source in your browser."
                )),
                Err(_) => result.messages.push(format!("{name}: search unavailable")),
            }
        }
        result
    })
}
#[cfg(test)]
mod tests {
    #[test]
    fn providers_cannot_redirect_searches_to_other_hosts() {
        assert!(super::allowed("https://www.minecraftskins.net/diamondrobot/download").is_ok());
        for bad in [
            "http://skinsmc.org/search/robot",
            "https://evil.example/skin.png",
            "https://user@skinsmc.org/skin",
            "https://skinsmc.org:444/skin",
        ] {
            assert!(super::allowed(bad).is_err());
        }
        let url = super::search_url("SkinsMC", "robot/../../?token=x").unwrap();
        assert!(url.starts_with("https://skinsmc.org/search/robot%2F"));
    }
    #[test]
    fn search_listings_keep_attribution_and_validate_download_hosts() {
        let html = r#"<div class="result"><div class="card"><a class="panel-link" href="/diamondrobot"></a><h2 class="card-title">Diamond Robot</h2></div></div>"#;
        let results = super::listings("MinecraftSkins.net", html).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].download,
            "https://www.minecraftskins.net/diamondrobot/download"
        );
    }
    #[test]
    #[ignore = "Live skin-provider search and validated PNG downloads"]
    fn live_multi_source_search() {
        for source in ["MinecraftSkins.net", "SkinsMC"] {
            let results = super::search("robot", source);
            assert!(
                !results.skins.is_empty(),
                "{source} returned no usable skins"
            );
            for item in results.skins {
                crate::skins::validate_png(&item.bytes).unwrap();
            }
        }
    }
}
