use base64::Engine as _;
use jian_ops_schema::node::PenNode;
use jian_ops_schema::style::PenFill;
use std::collections::HashMap;
use url::Url;

#[path = "resources_css_urls.rs"]
mod css_urls;
pub(crate) use css_urls::rebase_stylesheet_urls;

pub type ResourceFetcher<'a> = dyn Fn(&str) -> Option<Vec<u8>> + 'a;
pub type ImageTransform<'a> = dyn Fn(&[u8]) -> Option<Vec<u8>> + 'a;

const MAX_RESOURCES: usize = 200;
const PLACEHOLDER_GRAY_PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

#[derive(Default)]
pub(crate) struct ResourceBudget {
    count: usize,
    warned: bool,
}

impl ResourceBudget {
    pub(crate) fn take(&mut self, warnings: &mut Vec<String>) -> bool {
        if self.count < MAX_RESOURCES {
            self.count += 1;
            return true;
        }
        if !self.warned {
            warnings.push("resource limit reached (200), remaining resources skipped".to_string());
            self.warned = true;
        }
        false
    }
}

pub fn resolve_url(base: Option<&str>, href: &str) -> Option<String> {
    let href = href.trim();
    if let Ok(url) = Url::parse(href) {
        return matches!(url.scheme(), "http" | "https" | "data").then(|| url.to_string());
    }
    let base = Url::parse(base?.trim()).ok()?;
    if !matches!(base.scheme(), "http" | "https") {
        return None;
    }
    base.join(href).ok().map(Into::into)
}

pub(crate) fn embed_images(
    nodes: &mut [PenNode],
    base_url: Option<&str>,
    fetcher: &ResourceFetcher<'_>,
    transform: Option<&ImageTransform<'_>>,
    budget: &mut ResourceBudget,
    warnings: &mut Vec<String>,
) -> usize {
    let mut cache = HashMap::new();
    nodes
        .iter_mut()
        .map(|node| {
            embed_node_images(
                node, base_url, fetcher, transform, budget, warnings, &mut cache,
            )
        })
        .sum()
}

fn embed_node_images(
    node: &mut PenNode,
    base_url: Option<&str>,
    fetcher: &ResourceFetcher<'_>,
    transform: Option<&ImageTransform<'_>>,
    budget: &mut ResourceBudget,
    warnings: &mut Vec<String>,
    cache: &mut HashMap<String, String>,
) -> usize {
    let mut count = 0;
    let children = match node {
        PenNode::Frame(frame) => {
            count += embed_fills(
                &mut frame.container.fill,
                base_url,
                fetcher,
                transform,
                budget,
                warnings,
                cache,
            );
            frame.children.as_mut()
        }
        PenNode::Group(group) => {
            count += embed_fills(
                &mut group.container.fill,
                base_url,
                fetcher,
                transform,
                budget,
                warnings,
                cache,
            );
            group.children.as_mut()
        }
        PenNode::Rectangle(rectangle) => {
            count += embed_fills(
                &mut rectangle.container.fill,
                base_url,
                fetcher,
                transform,
                budget,
                warnings,
                cache,
            );
            rectangle.children.as_mut()
        }
        PenNode::Ellipse(ellipse) => {
            count += embed_fills(
                &mut ellipse.fill,
                base_url,
                fetcher,
                transform,
                budget,
                warnings,
                cache,
            );
            None
        }
        PenNode::Polygon(polygon) => {
            count += embed_fills(
                &mut polygon.fill,
                base_url,
                fetcher,
                transform,
                budget,
                warnings,
                cache,
            );
            None
        }
        PenNode::Path(path) => {
            count += embed_fills(
                &mut path.fill,
                base_url,
                fetcher,
                transform,
                budget,
                warnings,
                cache,
            );
            None
        }
        PenNode::Text(text) => {
            count += embed_fills(
                &mut text.fill,
                base_url,
                fetcher,
                transform,
                budget,
                warnings,
                cache,
            );
            None
        }
        PenNode::Image(image) => {
            let src = image.src.as_str().to_string();
            if let Some(embedded) =
                embed_url(&src, base_url, fetcher, transform, budget, warnings, cache)
            {
                image.src = embedded.into();
                count += 1;
            }
            None
        }
        PenNode::Ref(reference) => reference.children.as_mut(),
        PenNode::Tabs(tabs) => tabs.children.as_mut(),
        _ => None,
    };
    if let Some(children) = children {
        for child in children {
            count +=
                embed_node_images(child, base_url, fetcher, transform, budget, warnings, cache);
        }
    }
    count
}

fn embed_fills(
    fills: &mut Option<Vec<PenFill>>,
    base_url: Option<&str>,
    fetcher: &ResourceFetcher<'_>,
    transform: Option<&ImageTransform<'_>>,
    budget: &mut ResourceBudget,
    warnings: &mut Vec<String>,
    cache: &mut HashMap<String, String>,
) -> usize {
    let mut count = 0;
    if let Some(fills) = fills {
        for fill in fills {
            if let PenFill::Image(image) = fill {
                let url = image.url.as_str().to_string();
                if let Some(embedded) =
                    embed_url(&url, base_url, fetcher, transform, budget, warnings, cache)
                {
                    image.url = embedded.into();
                    count += 1;
                }
            }
        }
    }
    count
}

fn embed_url(
    url: &str,
    base_url: Option<&str>,
    fetcher: &ResourceFetcher<'_>,
    transform: Option<&ImageTransform<'_>>,
    budget: &mut ResourceBudget,
    warnings: &mut Vec<String>,
    cache: &mut HashMap<String, String>,
) -> Option<String> {
    if url.starts_with("data:") {
        return None;
    }
    let resolved = resolve_url(base_url, url).unwrap_or_else(|| url.to_string());
    if let Some(cached) = cache.get(&resolved) {
        return Some(cached.clone());
    }
    if !budget.take(warnings) {
        return None;
    }
    let embedded = match fetcher(&resolved) {
        Some(bytes) => {
            let transformed = transform.and_then(|rewrite| rewrite(&bytes));
            blob_to_data_url(transformed.as_deref().unwrap_or(&bytes))
        }
        None => {
            warnings.push(format!(
                "image resource unavailable, using placeholder: {resolved}"
            ));
            PLACEHOLDER_GRAY_PNG.to_string()
        }
    };
    cache.insert(resolved, embedded.clone());
    Some(embedded)
}

fn blob_to_data_url(bytes: &[u8]) -> String {
    let mime = match bytes {
        [0xff, 0xd8, ..] => "image/jpeg",
        [0x47, 0x49, ..] => "image/gif",
        [0x52, 0x49, ..] => "image/webp",
        [b'<', ..] if String::from_utf8_lossy(bytes).contains("<svg") => "image/svg+xml",
        _ => "image/png",
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{mime};base64,{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_url_forms() {
        assert_eq!(
            resolve_url(Some("https://a.dev/x/y.html"), "s.css").as_deref(),
            Some("https://a.dev/x/s.css")
        );
        assert_eq!(
            resolve_url(Some("https://a.dev/x/y.html"), "/s.css").as_deref(),
            Some("https://a.dev/s.css")
        );
        assert_eq!(
            resolve_url(Some("https://a.dev/x/y.html"), "../s.css").as_deref(),
            Some("https://a.dev/s.css")
        );
        assert_eq!(
            resolve_url(Some("https://a.dev/x/"), "//cdn.b.io/s.css").as_deref(),
            Some("https://cdn.b.io/s.css")
        );
        assert_eq!(
            resolve_url(None, "https://c.io/s.css").as_deref(),
            Some("https://c.io/s.css")
        );
        assert!(resolve_url(None, "s.css").is_none());
    }
}
