use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::Duration;

use jian_ops_schema::node::PenNode;
use op_editor_core::{EditorCommand, NodeId};
use op_html::{import_html_with_resources, HtmlImportOptions};
use op_mcp::{McpTool, ToolErrorCode, ToolOutcome};
use reqwest::header::{CONTENT_TYPE, LOCATION};

use crate::provider_dial::{client_for, EndpointDialPolicy};
use crate::web_image_search::{read_capped, ImageJobSlot};

const ALLOWLIST_ENV: &str = "OPENPENCIL_WEB_AI_ENDPOINT_ALLOWLIST";
const PAGE_BYTES_CAP: usize = 10 * 1024 * 1024;
const RESOURCE_BYTES_CAP: usize = 4 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_REDIRECTS: usize = 10;

pub(crate) struct ImportHtmlUrl;

impl McpTool for ImportHtmlUrl {
    fn name(&self) -> &str {
        "import_html_url"
    }

    fn call(&self, args: &BTreeMap<String, String>) -> ToolOutcome {
        let Some(raw_url) = args.get("url") else {
            return ToolOutcome::Err(ToolErrorCode::MissingArgument, "url is required".into());
        };
        let x = match parse_opt_i32(args, "x") {
            Ok(value) => value.unwrap_or(0),
            Err(error) => {
                return ToolOutcome::Err(ToolErrorCode::InvalidArgument, format!("x: {error}"));
            }
        };
        let y = match parse_opt_i32(args, "y") {
            Ok(value) => value.unwrap_or(0),
            Err(error) => {
                return ToolOutcome::Err(ToolErrorCode::InvalidArgument, format!("y: {error}"));
            }
        };
        let target_parent = args
            .get("parent")
            .or_else(|| args.get("parent_id"))
            .or_else(|| args.get("target_parent_id"))
            .map(|value| root_or_node_id(value))
            .unwrap_or(NodeId::NONE);
        let page_id = args
            .get("pageId")
            .or_else(|| args.get("page_id"))
            .or_else(|| args.get("page"))
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let allowlist = std::env::var(ALLOWLIST_ENV).ok();
        let initial_url = match screen_import_url_with_allowlist(raw_url, allowlist.as_deref()) {
            Ok(url) => url,
            Err(error) => return ToolOutcome::Err(ToolErrorCode::InvalidArgument, error),
        };
        let Some(_job_slot) = ImageJobSlot::acquire() else {
            return ToolOutcome::Err(
                ToolErrorCode::ToolFailed,
                "too many concurrent import jobs".into(),
            );
        };
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                return ToolOutcome::Err(
                    ToolErrorCode::ToolFailed,
                    format!("failed to start import runtime: {error}"),
                );
            }
        };
        let page = match runtime.block_on(fetch_capped(
            initial_url,
            PAGE_BYTES_CAP,
            allowlist.as_deref(),
        )) {
            Ok(page) => page,
            Err(error) => return ToolOutcome::Err(ToolErrorCode::ToolFailed, error),
        };
        let content_type_is_html = page
            .content_type
            .as_deref()
            .is_some_and(|value| value.to_ascii_lowercase().contains("text/html"));
        let body_looks_html = page.bytes.iter().take(512).any(|byte| *byte == b'<');
        if !content_type_is_html && !body_looks_html {
            return ToolOutcome::Err(ToolErrorCode::ToolFailed, "not an html page".into());
        }

        let html = String::from_utf8_lossy(&page.bytes);
        let fetcher = |resource_url: &str| {
            let resource_url =
                screen_import_url_with_allowlist(resource_url, allowlist.as_deref()).ok()?;
            runtime
                .block_on(fetch_capped(
                    resource_url,
                    RESOURCE_BYTES_CAP,
                    allowlist.as_deref(),
                ))
                .ok()
                .map(|resource| resource.bytes)
        };
        let options = HtmlImportOptions {
            base_url: Some(page.final_url.to_string()),
            ..HtmlImportOptions::default()
        };
        let result = import_html_with_resources(&html, &options, Some(&fetcher), None);
        if result.nodes.is_empty() {
            let detail = result
                .warnings
                .first()
                .map(String::as_str)
                .unwrap_or("input produced no nodes");
            return ToolOutcome::Err(
                ToolErrorCode::InvalidArgument,
                format!("no importable content: {detail}"),
            );
        }
        let mut nodes = result.nodes;
        if x != 0 || y != 0 {
            if let PenNode::Frame(frame) = &mut nodes[0] {
                frame.base.x = Some(x as f64);
                frame.base.y = Some(y as f64);
            }
        }
        let mut output = BTreeMap::new();
        output.insert("wrote".into(), "true".into());
        output.insert("nodeCount".into(), count_nodes(&nodes).to_string());
        output.insert("sourceUrl".into(), page.final_url.to_string());
        if !result.warnings.is_empty() {
            output.insert("warnings".into(), result.warnings.join("\n"));
        }
        ToolOutcome::OkWithCommand(
            output,
            EditorCommand::InsertSubtree {
                nodes,
                parent_id: target_parent,
                page_id,
            },
        )
    }
}

struct FetchedResource {
    bytes: Vec<u8>,
    content_type: Option<String>,
    final_url: reqwest::Url,
}

async fn fetch_capped(
    mut url: reqwest::Url,
    cap: usize,
    allowlist: Option<&str>,
) -> Result<FetchedResource, String> {
    for redirect_count in 0..=MAX_REDIRECTS {
        url = screen_import_url_with_allowlist(url.as_str(), allowlist)?;
        let policy = dial_policy(url.as_str(), allowlist);
        let client = client_for(policy, url.as_str()).await?;
        let response = client
            .get(url.clone())
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|error| format!("failed to fetch {url}: {error}"))?;
        if response.status().is_redirection() {
            if redirect_count == MAX_REDIRECTS {
                return Err("too many redirects while fetching html".into());
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| "redirect response is missing a valid Location".to_string())?;
            url = url
                .join(location)
                .map_err(|_| "redirect Location is not a valid URL".to_string())?;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!("failed to fetch {url}: HTTP {}", response.status()));
        }
        let final_url = screen_import_url_with_allowlist(response.url().as_str(), allowlist)?;
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let bytes = read_capped(response, cap)
            .await
            .ok_or_else(|| format!("response from {final_url} exceeds the size cap"))?;
        return Ok(FetchedResource {
            bytes,
            content_type,
            final_url,
        });
    }
    Err("too many redirects while fetching html".into())
}

#[cfg(test)]
fn screen_import_url(url: &str) -> Result<reqwest::Url, String> {
    let allowlist = std::env::var(ALLOWLIST_ENV).ok();
    screen_import_url_with_allowlist(url, allowlist.as_deref())
}

fn screen_import_url_with_allowlist(
    url: &str,
    allowlist: Option<&str>,
) -> Result<reqwest::Url, String> {
    let parsed =
        reqwest::Url::parse(url.trim()).map_err(|_| "import URL is invalid".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("import URL is not allowed".into());
    }
    if crate::web_credentials::base_url_is_explicitly_allowlisted(parsed.as_str(), allowlist) {
        return Ok(parsed);
    }
    let host = parsed
        .host_str()
        .expect("host presence checked above")
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host
        .parse::<IpAddr>()
        .is_ok_and(crate::web_credentials::is_restricted_ip)
        || is_restricted_hostname(&host)
    {
        return Err("import URL is not allowed".into());
    }
    Ok(parsed)
}

fn is_restricted_hostname(host: &str) -> bool {
    host.is_empty()
        || !host.contains('.')
        || [
            "localhost",
            ".localhost",
            ".local",
            ".internal",
            ".home",
            ".lan",
            ".test",
            ".invalid",
        ]
        .iter()
        .any(|suffix| host == *suffix || host.ends_with(suffix))
        || matches!(
            host,
            "metadata.google.internal"
                | "metadata.google"
                | "instance-data"
                | "instance-data.ec2.internal"
        )
}

fn dial_policy(url: &str, allowlist: Option<&str>) -> EndpointDialPolicy {
    if crate::web_credentials::base_url_is_explicitly_allowlisted(url, allowlist) {
        EndpointDialPolicy::Trusted
    } else {
        EndpointDialPolicy::PublicOnly
    }
}

fn parse_opt_i32(args: &BTreeMap<String, String>, key: &str) -> Result<Option<i32>, String> {
    match args.get(key) {
        None => Ok(None),
        Some(value) => value
            .parse::<i32>()
            .map(Some)
            .map_err(|_| format!("expected decimal i32, got {value:?}")),
    }
}

fn root_or_node_id(raw: &str) -> NodeId {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed == "0"
        || trimmed.eq_ignore_ascii_case("root")
        || trimmed.eq_ignore_ascii_case("null")
    {
        NodeId::NONE
    } else {
        NodeId::new(trimmed)
    }
}

fn count_nodes(nodes: &[PenNode]) -> usize {
    nodes
        .iter()
        .map(|node| {
            1 + match node {
                PenNode::Frame(node) => node
                    .children
                    .as_deref()
                    .map(count_nodes)
                    .unwrap_or_default(),
                PenNode::Group(node) => node
                    .children
                    .as_deref()
                    .map(count_nodes)
                    .unwrap_or_default(),
                PenNode::Rectangle(node) => node
                    .children
                    .as_deref()
                    .map(count_nodes)
                    .unwrap_or_default(),
                _ => 0,
            }
        })
        .sum()
}

pub(crate) fn import_html_url_snapshot() -> ImportHtmlUrl {
    ImportHtmlUrl
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_import_url_rejects_restricted_hosts() {
        assert!(screen_import_url("https://example.com/page").is_ok());
        assert!(screen_import_url("https://example.com/p?q=1#f").is_ok());
        assert!(screen_import_url("http://127.0.0.1:3000/").is_err());
        assert!(screen_import_url("http://169.254.169.254/meta").is_err());
        assert!(screen_import_url("http://localhost/x").is_err());
        assert!(screen_import_url("ftp://example.com/x").is_err());
        assert!(screen_import_url("https://user:pw@example.com/").is_err());
    }

    #[test]
    fn missing_url_is_typed_error() {
        let out = ImportHtmlUrl.call(&BTreeMap::new());
        assert!(matches!(out, ToolOutcome::Err(..)));
    }
}
