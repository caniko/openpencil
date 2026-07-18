//! Browser image-search route for the web daemon.
//!
//! `POST /api/ai/image/search` mirrors the desktop image panel's Search
//! popover backend (`op-host-desktop/src/image_panel_host.rs`: Openverse →
//! two-keyword retry → Wikimedia, thumbnails embedded as `data:` URLs) so
//! the wasm shell can drain its `search_epoch` through the daemon instead
//! of leaving the popover loading forever. Openverse credentials come from
//! the request body (browser-held) or fall back to the daemon's persisted
//! agent settings. Openverse / Wikimedia are product-constant public hosts
//! — the same operator-trust tier as the desktop path — so they dial with
//! a plain client; nothing in this route dials a browser-supplied URL.
//!
//! Unlike the desktop, fetched thumbnails are NOT re-encoded/down-scaled
//! here: `image_downscale` needs skia and this crate must stay GL-free for
//! `op-host-web-server`. The 4 MiB per-image cap still bounds what can be
//! embedded.

use std::time::Duration;

use reqwest::header::CONTENT_TYPE;

/// TS popover requests `count: 5` (desktop parity).
const SEARCH_RESULT_COUNT: usize = 5;
const MAX_EMBEDDED_IMAGE_BYTES: usize = 4 * 1024 * 1024;

/// Design-artifact words that are pure noise against a photo corpus (see
/// the desktop `image_search_session.rs` for the measurement notes).
const IMAGE_SEARCH_ARTIFACT_WORDS: &[&str] = &[
    "album",
    "cover",
    "playlist",
    "artwork",
    "poster",
    "thumbnail",
    "logo",
    "icon",
    "banner",
    "mockup",
    "screenshot",
    "wallpaper",
];

const IMAGE_SEARCH_STOP_WORDS: &[&str] = &[
    "a",
    "an",
    "the",
    "and",
    "or",
    "but",
    "in",
    "on",
    "at",
    "to",
    "for",
    "of",
    "with",
    "by",
    "from",
    "is",
    "are",
    "was",
    "were",
    "be",
    "been",
    "being",
    "have",
    "has",
    "had",
    "do",
    "does",
    "did",
    "will",
    "would",
    "could",
    "should",
    "may",
    "might",
    "shall",
    "can",
    "that",
    "this",
    "these",
    "those",
    "it",
    "its",
    "very",
    "really",
    "just",
    "also",
    "about",
    "above",
    "after",
    "before",
    "between",
    "into",
    "through",
    "during",
    "each",
    "some",
    "such",
    "no",
    "not",
    "only",
    "same",
    "so",
    "than",
    "too",
    "up",
    "out",
    "if",
    "then",
    "once",
    "here",
    "there",
    "when",
    "where",
    "how",
    "all",
    "both",
    "few",
    "more",
    "most",
    "other",
    "any",
    "as",
    "while",
    "using",
    "showing",
    "featuring",
    "looking",
    "style",
    "styled",
    "inspired",
    "based",
];

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct WebOpenverseCredentials {
    pub(crate) client_id: String,
    pub(crate) client_secret: String,
}

impl WebOpenverseCredentials {
    fn from_parts(client_id: &str, client_secret: &str) -> Option<Self> {
        let client_id = client_id.trim();
        let client_secret = client_secret.trim();
        if client_id.is_empty() || client_secret.is_empty() {
            None
        } else {
            Some(Self {
                client_id: client_id.to_string(),
                client_secret: client_secret.to_string(),
            })
        }
    }
}

/// One search hit ready for the JSON reply.
pub(crate) struct WebImageSearchHit {
    pub(crate) id: String,
    pub(crate) thumb_data_url: String,
    pub(crate) attribution: String,
}

pub(crate) struct WebImageSearchOutcome {
    pub(crate) results: Vec<WebImageSearchHit>,
    /// `"openverse"` / `"wikimedia"`, `None` when nothing landed.
    pub(crate) source: Option<&'static str>,
}

/// Parse the request body and snapshot the daemon-side credential fallback.
/// Returns `(query, credentials)` or an error message for the 400 reply.
pub(crate) fn parse_search_request(
    body: &str,
    state: &op_editor_core::EditorState,
) -> Result<(String, Option<WebOpenverseCredentials>), String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| "invalid request body".to_string())?;
    let obj = value.as_object().ok_or("invalid request body")?;
    let query = obj
        .get("query")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .ok_or("missing query")?;
    // Browser-held credential wins; the daemon's persisted settings are the
    // fallback (both are optional — anonymous Openverse works, rate-limited).
    let request_credentials = obj
        .get("openverse")
        .and_then(serde_json::Value::as_object)
        .and_then(|cred| {
            WebOpenverseCredentials::from_parts(
                cred.get("client_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
                cred.get("client_secret")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            )
        });
    let credentials = request_credentials.or_else(|| {
        let settings = &state.editor_ui.agent_settings;
        WebOpenverseCredentials::from_parts(
            &settings.openverse_client_id,
            &settings.openverse_client_secret,
        )
    });
    Ok((query.to_string(), credentials))
}

/// JSON reply body for a finished search.
pub(crate) fn search_outcome_to_json(outcome: &WebImageSearchOutcome) -> String {
    let results: Vec<serde_json::Value> = outcome
        .results
        .iter()
        .map(|hit| {
            serde_json::json!({
                "id": hit.id,
                "thumb_data_url": hit.thumb_data_url,
                "attribution": hit.attribution,
            })
        })
        .collect();
    serde_json::json!({
        "ok": true,
        "results": results,
        "source": outcome.source,
    })
    .to_string()
}

/// Run the full search ladder on the calling thread (the connection's own
/// thread — the caller must NOT hold the state lock).
pub(crate) fn run_search_blocking(
    query: &str,
    credentials: Option<&WebOpenverseCredentials>,
) -> WebImageSearchOutcome {
    let empty = WebImageSearchOutcome {
        results: Vec::new(),
        source: None,
    };
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return empty;
    };
    runtime.block_on(run_search(query, credentials))
}

async fn run_search(
    query: &str,
    credentials: Option<&WebOpenverseCredentials>,
) -> WebImageSearchOutcome {
    let empty = WebImageSearchOutcome {
        results: Vec::new(),
        source: None,
    };
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent(concat!("openpencil-web-daemon/", env!("CARGO_PKG_VERSION")))
        .build()
    else {
        return empty;
    };
    // Simplify verbose prompts into keywords (TS simplifySearchQuery).
    let query = simplify_search_query(query);

    // Openverse first; a zero-result answer retries with the first two
    // keywords before falling through to Wikimedia (desktop parity).
    let mut hits = fetch_openverse_list(&client, &query, credentials).await;
    if hits.as_ref().is_some_and(Vec::is_empty) {
        if let Some(truncated) = two_keyword_retry(&query) {
            if let Some(retry) = fetch_openverse_list(&client, &truncated, credentials).await {
                if !retry.is_empty() {
                    hits = Some(retry);
                }
            }
        }
    }
    if let Some(urls) = hits.filter(|h| !h.is_empty()) {
        let results = materialize_thumbs(&client, urls).await;
        if !results.is_empty() {
            return WebImageSearchOutcome {
                results,
                source: Some("openverse"),
            };
        }
    }
    let mut wiki = fetch_wikimedia_list(&client, &query).await;
    if wiki.is_empty() {
        if let Some(truncated) = two_keyword_retry(&query) {
            wiki = fetch_wikimedia_list(&client, &truncated).await;
        }
    }
    let results = materialize_thumbs(&client, wiki).await;
    let source = (!results.is_empty()).then_some("wikimedia");
    WebImageSearchOutcome { results, source }
}

fn two_keyword_retry(query: &str) -> Option<String> {
    let words: Vec<&str> = query.split_whitespace().filter(|w| !w.is_empty()).collect();
    (words.len() > 2).then(|| words[..2].join(" "))
}

pub(crate) struct RawHit {
    id: String,
    thumb_url: String,
    attribution: String,
}

/// `None` = request-level failure (429 / network), `Some([])` = the
/// catalogue answered with zero hits (the ladder distinguishes the two).
async fn fetch_openverse_list(
    client: &reqwest::Client,
    query: &str,
    credentials: Option<&WebOpenverseCredentials>,
) -> Option<Vec<RawHit>> {
    let url = reqwest::Url::parse_with_params(
        "https://api.openverse.org/v1/images/",
        &[
            ("q", query),
            ("page_size", &SEARCH_RESULT_COUNT.to_string()),
        ],
    )
    .ok()?;
    let mut request = client.get(url);
    if let Some(credentials) = credentials {
        if let Some(token) = fetch_openverse_token(client, credentials).await {
            request = request.bearer_auth(token);
        }
    }
    let resp = request.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    Some(parse_openverse_results(&json))
}

pub(crate) fn parse_openverse_results(json: &serde_json::Value) -> Vec<RawHit> {
    let Some(results) = json.get("results").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    results
        .iter()
        .filter_map(|r| {
            let thumb = r
                .get("thumbnail")
                .and_then(serde_json::Value::as_str)
                .or_else(|| r.get("url").and_then(serde_json::Value::as_str))?;
            let license = format!(
                "{} {}",
                r.get("license")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
                r.get("license_version")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            );
            Some(RawHit {
                id: r
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                thumb_url: thumb.to_string(),
                attribution: r
                    .get("attribution")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| license.trim().to_string()),
            })
        })
        .take(SEARCH_RESULT_COUNT)
        .collect()
}

async fn fetch_wikimedia_list(client: &reqwest::Client, query: &str) -> Vec<RawHit> {
    let Ok(url) = reqwest::Url::parse_with_params(
        "https://commons.wikimedia.org/w/api.php",
        &[
            ("action", "query"),
            ("generator", "search"),
            ("gsrsearch", query),
            ("gsrnamespace", "6"),
            ("gsrlimit", &SEARCH_RESULT_COUNT.to_string()),
            ("prop", "imageinfo"),
            ("iiprop", "url|size|mime|extmetadata"),
            ("iiurlwidth", "800"),
            ("format", "json"),
            ("origin", "*"),
        ],
    ) else {
        return Vec::new();
    };
    let Ok(resp) = client.get(url).send().await else {
        return Vec::new();
    };
    if !resp.status().is_success() {
        return Vec::new();
    }
    let Ok(json) = resp.json::<serde_json::Value>().await else {
        return Vec::new();
    };
    parse_wikimedia_results(&json)
}

pub(crate) fn parse_wikimedia_results(json: &serde_json::Value) -> Vec<RawHit> {
    let Some(pages) = json
        .get("query")
        .and_then(|q| q.get("pages"))
        .and_then(serde_json::Value::as_object)
    else {
        return Vec::new();
    };
    pages
        .values()
        .filter_map(|page| {
            let info = page.get("imageinfo")?.as_array()?.first()?;
            let thumb = info
                .get("thumburl")
                .and_then(serde_json::Value::as_str)
                .or_else(|| info.get("url").and_then(serde_json::Value::as_str))?;
            Some(RawHit {
                id: page
                    .get("pageid")
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                thumb_url: thumb.to_string(),
                attribution: info
                    .get("extmetadata")
                    .and_then(|m| m.get("LicenseShortName"))
                    .and_then(|l| l.get("value"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .take(SEARCH_RESULT_COUNT)
        .collect()
}

/// Download each hit's thumbnail into a `data:` URL. Hits whose thumbnails
/// fail to download are dropped.
async fn materialize_thumbs(client: &reqwest::Client, hits: Vec<RawHit>) -> Vec<WebImageSearchHit> {
    let mut out = Vec::with_capacity(hits.len());
    for hit in hits {
        if let Some(data_url) = fetch_image_data_url(client, &hit.thumb_url).await {
            out.push(WebImageSearchHit {
                id: hit.id,
                thumb_data_url: data_url,
                attribution: hit.attribution,
            });
        }
    }
    out
}

/// Simplify a verbose prompt into provider keywords (desktop parity).
pub(crate) fn simplify_search_query(prompt: &str) -> String {
    let mut normalized = String::with_capacity(prompt.len());
    for ch in prompt.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() || ch.is_ascii_whitespace() || ch == '-' {
            normalized.push(ch);
        } else {
            normalized.push(' ');
        }
    }
    let keywords: Vec<&str> = normalized
        .split_whitespace()
        .filter(|word| word.len() > 2 && !IMAGE_SEARCH_STOP_WORDS.contains(word))
        .take(6)
        .collect();
    // Drop artifact words ONLY when aesthetic words remain — "logo" alone
    // must not become an empty query.
    let non_artifact: Vec<&str> = keywords
        .iter()
        .copied()
        .filter(|word| !IMAGE_SEARCH_ARTIFACT_WORDS.contains(word))
        .collect();
    let keywords: Vec<&str> = if non_artifact.is_empty() {
        keywords
    } else {
        non_artifact
    }
    .into_iter()
    .take(4)
    .collect();
    if keywords.is_empty() {
        prompt.chars().take(30).collect()
    } else {
        keywords.join(" ")
    }
}

pub(crate) async fn fetch_openverse_token(
    client: &reqwest::Client,
    credentials: &WebOpenverseCredentials,
) -> Option<String> {
    let resp = client
        .post("https://api.openverse.org/v1/auth_tokens/token/")
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", credentials.client_id.as_str()),
            ("client_secret", credentials.client_secret.as_str()),
        ])
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    json.get("access_token")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

/// Download `url` and embed it as a `data:` URL, subject to the 4 MiB cap.
pub(crate) async fn fetch_image_data_url(client: &reqwest::Client, url: &str) -> Option<String> {
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    if resp
        .content_length()
        .is_some_and(|len| len > MAX_EMBEDDED_IMAGE_BYTES as u64)
    {
        return None;
    }
    let header_mime = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(normalize_image_mime_header);
    let bytes = resp.bytes().await.ok()?;
    if bytes.is_empty() || bytes.len() > MAX_EMBEDDED_IMAGE_BYTES {
        return None;
    }
    let mime = header_mime.or_else(|| sniff_image_mime(&bytes).map(str::to_string))?;
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine as _;
    Some(format!("data:{mime};base64,{}", B64.encode(&bytes)))
}

fn normalize_image_mime_header(value: &str) -> Option<String> {
    let mime = value.split(';').next()?.trim().to_ascii_lowercase();
    if mime == "image/jpg" {
        return Some("image/jpeg".to_string());
    }
    if mime.starts_with("image/") && mime != "image/svg+xml" {
        Some(mime)
    } else {
        None
    }
}

fn sniff_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1A\n") {
        return Some("image/png");
    }
    if bytes.starts_with(b"\xFF\xD8\xFF") {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplify_search_query_mirrors_the_desktop_adapter() {
        assert_eq!(
            simplify_search_query("A beautiful sunset over the mountains"),
            "beautiful sunset over mountains"
        );
        // Artifact words drop only when aesthetic words remain.
        assert_eq!(
            simplify_search_query("synthwave album cover neon"),
            "synthwave neon"
        );
        assert_eq!(simplify_search_query("logo"), "logo");
        // Empty keyword set falls back to a 30-char prefix.
        assert_eq!(simplify_search_query("の"), "の");
    }

    #[test]
    fn parse_search_request_reads_query_and_prefers_request_credentials() {
        let mut state = op_editor_core::EditorState::default();
        state.editor_ui.agent_settings.openverse_client_id = "persisted-id".into();
        state.editor_ui.agent_settings.openverse_client_secret = "persisted-secret".into();
        let (query, cred) = parse_search_request(
            r#"{"query":"cat","openverse":{"client_id":"req-id","client_secret":"req-secret"}}"#,
            &state,
        )
        .expect("parses");
        assert_eq!(query, "cat");
        assert_eq!(cred.expect("cred").client_id, "req-id");
        // No request credential → daemon-persisted fallback.
        let (_, cred) = parse_search_request(r#"{"query":"cat"}"#, &state).expect("parses");
        assert_eq!(cred.expect("cred").client_id, "persisted-id");
        // Neither → anonymous.
        let empty = op_editor_core::EditorState::default();
        let (_, cred) = parse_search_request(r#"{"query":"cat"}"#, &empty).expect("parses");
        assert!(cred.is_none());
    }

    #[test]
    fn parse_search_request_rejects_bad_bodies() {
        let state = op_editor_core::EditorState::default();
        assert!(parse_search_request("", &state).is_err());
        assert!(parse_search_request("{}", &state).is_err());
        assert!(parse_search_request(r#"{"query":"  "}"#, &state).is_err());
    }

    #[test]
    fn parse_openverse_results_maps_thumbnail_license_and_cap() {
        let json = serde_json::json!({
            "results": [
                {"id": "a", "thumbnail": "https://x/a.jpg", "attribution": "By A"},
                {"id": "b", "url": "https://x/b.jpg", "license": "cc0", "license_version": "1.0"},
                {"id": "c"},
                {"id": "d", "thumbnail": "https://x/d.jpg"},
                {"id": "e", "thumbnail": "https://x/e.jpg"},
                {"id": "f", "thumbnail": "https://x/f.jpg"},
                {"id": "g", "thumbnail": "https://x/g.jpg"}
            ]
        });
        let hits = parse_openverse_results(&json);
        assert_eq!(hits.len(), SEARCH_RESULT_COUNT); // "c" dropped, capped at 5
        assert_eq!(hits[0].id, "a");
        assert_eq!(hits[0].attribution, "By A");
        assert_eq!(hits[1].thumb_url, "https://x/b.jpg");
        assert_eq!(hits[1].attribution, "cc0 1.0");
    }

    #[test]
    fn parse_wikimedia_results_maps_thumburl_and_license() {
        let json = serde_json::json!({
            "query": {"pages": {
                "1": {"pageid": 1, "imageinfo": [{
                    "thumburl": "https://c/w1.jpg",
                    "extmetadata": {"LicenseShortName": {"value": "CC BY-SA 4.0"}}
                }]},
                "2": {"pageid": 2, "imageinfo": [{"url": "https://c/w2.jpg"}]},
                "3": {"pageid": 3}
            }}
        });
        let mut hits = parse_wikimedia_results(&json);
        hits.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].thumb_url, "https://c/w1.jpg");
        assert_eq!(hits[0].attribution, "CC BY-SA 4.0");
        assert_eq!(hits[1].thumb_url, "https://c/w2.jpg");
    }

    #[test]
    fn search_outcome_json_shape() {
        let outcome = WebImageSearchOutcome {
            results: vec![WebImageSearchHit {
                id: "a".into(),
                thumb_data_url: "data:image/png;base64,AA==".into(),
                attribution: "By A".into(),
            }],
            source: Some("openverse"),
        };
        let json: serde_json::Value =
            serde_json::from_str(&search_outcome_to_json(&outcome)).expect("valid json");
        assert_eq!(json["ok"], true);
        assert_eq!(json["source"], "openverse");
        assert_eq!(json["results"][0]["id"], "a");
        assert_eq!(
            json["results"][0]["thumb_data_url"],
            "data:image/png;base64,AA=="
        );
        let empty = WebImageSearchOutcome {
            results: Vec::new(),
            source: None,
        };
        let json: serde_json::Value =
            serde_json::from_str(&search_outcome_to_json(&empty)).expect("valid json");
        assert!(json["source"].is_null());
    }

    #[test]
    fn sniff_image_mime_recognizes_the_embeddable_formats() {
        assert_eq!(sniff_image_mime(b"\x89PNG\r\n\x1A\nxx"), Some("image/png"));
        assert_eq!(sniff_image_mime(b"\xFF\xD8\xFFxx"), Some("image/jpeg"));
        assert_eq!(sniff_image_mime(b"GIF89a"), Some("image/gif"));
        assert_eq!(
            sniff_image_mime(b"RIFF\0\0\0\0WEBPVP8 "),
            Some("image/webp")
        );
        assert_eq!(sniff_image_mime(b"<svg>"), None);
        assert_eq!(
            normalize_image_mime_header("image/jpg"),
            Some("image/jpeg".into())
        );
        assert_eq!(normalize_image_mime_header("image/svg+xml"), None);
        assert_eq!(normalize_image_mime_header("text/html"), None);
    }
}
