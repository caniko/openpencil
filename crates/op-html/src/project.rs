use std::collections::{HashMap, HashSet};
use std::fmt;

use url::Url;

use crate::resources::ImageTransform;
use crate::{
    import_html_with_resources, wrap_imported_document, HtmlDocumentResult, HtmlImportOptions,
    HtmlImportResult,
};

pub const MAX_PROJECT_FILES: usize = 16_384;
pub const VIRTUAL_PROJECT_ORIGIN: &str = "https://openpencil.local/";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlProjectFile {
    pub relative_path: String,
    pub bytes: Vec<u8>,
}

impl HtmlProjectFile {
    pub fn new(relative_path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            relative_path: relative_path.into(),
            bytes: bytes.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HtmlProjectError {
    EmptyProject,
    InvalidPath { path: String, reason: String },
    DuplicatePath(String),
    TooManyFiles { count: usize, limit: usize },
    NoHtmlEntry,
    HtmlIsNotUtf8(String),
    InvalidZip(String),
    UnsupportedZipEntry { path: String, reason: String },
}

impl fmt::Display for HtmlProjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyProject => formatter.write_str("HTML project contains no files"),
            Self::InvalidPath { path, reason } => {
                write!(formatter, "invalid project path `{path}`: {reason}")
            }
            Self::DuplicatePath(path) => write!(formatter, "duplicate project path `{path}`"),
            Self::TooManyFiles { count, limit } => {
                write!(
                    formatter,
                    "HTML project contains {count} files; limit is {limit}"
                )
            }
            Self::NoHtmlEntry => formatter.write_str("HTML project contains no .html or .htm file"),
            Self::HtmlIsNotUtf8(path) => {
                write!(formatter, "HTML entry `{path}` is not valid UTF-8")
            }
            Self::InvalidZip(detail) => write!(formatter, "invalid HTML ZIP archive: {detail}"),
            Self::UnsupportedZipEntry { path, reason } => {
                write!(formatter, "unsupported ZIP entry `{path}`: {reason}")
            }
        }
    }
}

impl std::error::Error for HtmlProjectError {}

pub fn select_html_entry(files: &[HtmlProjectFile]) -> Result<String, HtmlProjectError> {
    let project = PreparedProject::new(files)?;
    Ok(project.files[project.entry_index()?].relative_path.clone())
}

pub fn import_html_project(
    files: &[HtmlProjectFile],
    options: &HtmlImportOptions,
) -> Result<HtmlImportResult, HtmlProjectError> {
    import_html_project_with_transform(files, options, None)
}

pub fn import_html_project_with_transform(
    files: &[HtmlProjectFile],
    options: &HtmlImportOptions,
    transform: Option<&ImageTransform<'_>>,
) -> Result<HtmlImportResult, HtmlProjectError> {
    let project = PreparedProject::new(files)?;
    let entry_index = project.entry_index()?;
    let entry = &project.files[entry_index];
    let html = std::str::from_utf8(entry.bytes)
        .map_err(|_| HtmlProjectError::HtmlIsNotUtf8(entry.relative_path.clone()))?;
    let html = html.strip_prefix('\u{feff}').unwrap_or(html);
    let entry_path = entry.relative_path.clone();
    let html_entry_count = project
        .files
        .iter()
        .filter(|file| is_html_path(&file.relative_path))
        .count();
    let resources: HashMap<String, &[u8]> = project
        .files
        .iter()
        .map(|file| (virtual_url(&file.relative_path), file.bytes))
        .collect();
    let fetcher = |requested: &str| {
        virtual_resource_key(requested)
            .and_then(|key| resources.get(&key).copied())
            .map(<[u8]>::to_vec)
    };
    Ok(import_html_entry_with_fetcher(
        html,
        &entry_path,
        html_entry_count,
        options,
        &fetcher,
        transform,
    ))
}

pub fn import_html_project_document(
    files: &[HtmlProjectFile],
    options: &HtmlImportOptions,
) -> Result<HtmlDocumentResult, HtmlProjectError> {
    import_html_project_document_with_transform(files, options, None)
}

pub fn import_html_project_document_with_transform(
    files: &[HtmlProjectFile],
    options: &HtmlImportOptions,
    transform: Option<&ImageTransform<'_>>,
) -> Result<HtmlDocumentResult, HtmlProjectError> {
    import_html_project_with_transform(files, options, transform).map(wrap_imported_document)
}

pub(crate) fn normalize_relative_path(path: &str) -> Result<String, HtmlProjectError> {
    if path.is_empty() {
        return Err(invalid_path(path, "path is empty"));
    }
    if path.contains('\0') {
        return Err(invalid_path(path, "path contains a NUL byte"));
    }
    if path.starts_with(['/', '\\']) {
        return Err(invalid_path(path, "absolute paths are not allowed"));
    }
    let unified = path.replace('\\', "/");
    if unified
        .as_bytes()
        .get(1)
        .is_some_and(|byte| *byte == b':' && unified.as_bytes()[0].is_ascii_alphabetic())
    {
        return Err(invalid_path(path, "drive-prefixed paths are not allowed"));
    }
    let mut components = Vec::new();
    for component in unified.split('/') {
        match component {
            "" | "." => {}
            ".." => return Err(invalid_path(path, "parent traversal is not allowed")),
            value => components.push(value),
        }
    }
    if components.is_empty() {
        return Err(invalid_path(path, "path contains no file name"));
    }
    Ok(components.join("/"))
}

pub(crate) fn is_ignored_metadata_path(path: &str) -> bool {
    let components: Vec<&str> = path.split('/').collect();
    components
        .iter()
        .any(|component| component.eq_ignore_ascii_case("__MACOSX"))
        || components
            .last()
            .is_some_and(|name| name.eq_ignore_ascii_case(".DS_Store"))
}

fn invalid_path(path: &str, reason: &str) -> HtmlProjectError {
    HtmlProjectError::InvalidPath {
        path: path.to_string(),
        reason: reason.to_string(),
    }
}

struct PreparedFile<'a> {
    relative_path: String,
    bytes: &'a [u8],
}

struct PreparedProject<'a> {
    files: Vec<PreparedFile<'a>>,
}

impl<'a> PreparedProject<'a> {
    fn new(files: &'a [HtmlProjectFile]) -> Result<Self, HtmlProjectError> {
        if files.is_empty() {
            return Err(HtmlProjectError::EmptyProject);
        }
        if files.len() > MAX_PROJECT_FILES {
            return Err(HtmlProjectError::TooManyFiles {
                count: files.len(),
                limit: MAX_PROJECT_FILES,
            });
        }
        let mut seen = HashSet::new();
        let mut prepared = Vec::with_capacity(files.len());
        for file in files {
            let path = normalize_relative_path(&file.relative_path)?;
            if is_ignored_metadata_path(&path) {
                continue;
            }
            if !seen.insert(path.clone()) {
                return Err(HtmlProjectError::DuplicatePath(path));
            }
            prepared.push(PreparedFile {
                relative_path: path,
                bytes: &file.bytes,
            });
        }
        if prepared.is_empty() {
            return Err(HtmlProjectError::EmptyProject);
        }
        let mut paths: Vec<String> = prepared
            .iter()
            .map(|file| file.relative_path.clone())
            .collect();
        strip_common_root_paths(&mut paths);
        for (file, path) in prepared.iter_mut().zip(paths) {
            file.relative_path = path;
        }
        let mut stripped_seen = HashSet::new();
        for file in &prepared {
            if !stripped_seen.insert(file.relative_path.clone()) {
                return Err(HtmlProjectError::DuplicatePath(file.relative_path.clone()));
            }
        }
        Ok(Self { files: prepared })
    }

    fn entry_index(&self) -> Result<usize, HtmlProjectError> {
        select_html_entry_index(
            &self
                .files
                .iter()
                .map(|file| file.relative_path.clone())
                .collect::<Vec<_>>(),
        )
    }
}

pub(crate) fn strip_common_root_paths(paths: &mut [String]) {
    loop {
        let Some(root) = paths
            .first()
            .and_then(|path| path.split_once('/').map(|(root, _)| root))
        else {
            return;
        };
        if !paths.iter().all(|path| {
            path.split_once('/')
                .is_some_and(|(candidate, _)| candidate == root)
        }) {
            return;
        }
        let prefix_len = root.len() + 1;
        for path in &mut *paths {
            path.drain(..prefix_len);
        }
    }
}

pub(crate) fn is_html_path(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, extension)| {
        extension.eq_ignore_ascii_case("html") || extension.eq_ignore_ascii_case("htm")
    })
}

fn entry_file_stem(path: &str) -> &str {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem)
}

fn html_entry_rank(path: &str) -> (u8, usize, u8, String, String) {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    let lower_name = file_name.to_ascii_lowercase();
    let (index_rank, extension_rank) = match lower_name.as_str() {
        "index.html" => (0, 0),
        "index.htm" => (0, 1),
        _ => (1, 2),
    };
    (
        index_rank,
        path.matches('/').count(),
        extension_rank,
        path.to_ascii_lowercase(),
        path.to_string(),
    )
}

pub(crate) fn select_html_entry_index(paths: &[String]) -> Result<usize, HtmlProjectError> {
    paths
        .iter()
        .enumerate()
        .filter(|(_, path)| is_html_path(path))
        .min_by_key(|(_, path)| html_entry_rank(path))
        .map(|(index, _)| index)
        .ok_or(HtmlProjectError::NoHtmlEntry)
}

pub(crate) fn virtual_url(path: &str) -> String {
    let mut url = Url::parse(VIRTUAL_PROJECT_ORIGIN).expect("virtual project origin is valid");
    {
        let mut segments = url
            .path_segments_mut()
            .expect("virtual project origin is a hierarchical URL");
        segments.pop_if_empty();
        segments.extend(path.split('/'));
    }
    url.into()
}

pub(crate) fn virtual_resource_key(requested: &str) -> Option<String> {
    let mut url = Url::parse(requested).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("openpencil.local")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    url.set_query(None);
    url.set_fragment(None);
    Some(url.into())
}

pub(crate) fn import_html_entry_with_fetcher(
    html: &str,
    entry_path: &str,
    html_entry_count: usize,
    options: &HtmlImportOptions,
    fetcher: &crate::resources::ResourceFetcher<'_>,
    transform: Option<&ImageTransform<'_>>,
) -> HtmlImportResult {
    let project_options = HtmlImportOptions {
        viewport_width: options.viewport_width,
        base_font_size: options.base_font_size,
        document_name: options.document_name.clone(),
        base_url: Some(virtual_url(entry_path)),
    };
    let mut imported = import_html_with_resources(
        html.strip_prefix('\u{feff}').unwrap_or(html),
        &project_options,
        Some(fetcher),
        transform,
    );
    if options.document_name.is_none() {
        if let Some(jian_ops_schema::node::PenNode::Frame(root)) = imported.nodes.first_mut() {
            if root.base.name.as_deref() == Some("HTML Import") {
                root.base.name = Some(entry_file_stem(entry_path).to_string());
            }
        }
    }
    if html_entry_count > 1 {
        imported.warnings.push(format!(
            "multiple HTML entries found ({html_entry_count}); selected {entry_path}"
        ));
    }
    imported
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, bytes: &[u8]) -> HtmlProjectFile {
        HtmlProjectFile::new(path, bytes)
    }

    #[test]
    fn selects_index_before_shallower_named_html_deterministically() {
        let files = vec![
            file("site/README.html", b"readme"),
            file("site/dist/index.htm", b"htm"),
            file("site/dist/index.html", b"html"),
        ];
        assert_eq!(select_html_entry(&files).unwrap(), "dist/index.html");
    }

    #[test]
    fn rejects_traversal_duplicates_and_file_count_overflow() {
        let traversal = vec![file("../index.html", b"x")];
        assert!(matches!(
            select_html_entry(&traversal),
            Err(HtmlProjectError::InvalidPath { .. })
        ));
        let duplicate = vec![file("./index.html", b"a"), file("index.html", b"b")];
        assert!(matches!(
            select_html_entry(&duplicate),
            Err(HtmlProjectError::DuplicatePath(_))
        ));
        let too_many: Vec<_> = (0..=MAX_PROJECT_FILES)
            .map(|index| file(&format!("{index}.txt"), b"x"))
            .collect();
        assert!(matches!(
            select_html_entry(&too_many),
            Err(HtmlProjectError::TooManyFiles { .. })
        ));
    }

    #[test]
    fn reports_a_project_without_html() {
        let files = vec![file("styles/site.css", b"body{}")];
        assert_eq!(
            select_html_entry(&files),
            Err(HtmlProjectError::NoHtmlEntry)
        );
    }

    #[test]
    fn common_root_and_root_relative_css_and_images_share_the_virtual_origin() {
        let files = vec![
            file(
                "site/dist/index.html",
                br#"<link rel="stylesheet" href="/assets/site.css">
                    <div class="hero"><img src="/assets/pixel.png"></div>"#,
            ),
            file(
                "site/assets/site.css",
                b".hero{background-image:url('/assets/pixel.png');color:#123456}",
            ),
            file("site/assets/pixel.png", &[0x89, 0x50, 0x4e, 0x47, 1, 2, 3]),
        ];
        let imported = import_html_project(&files, &HtmlImportOptions::default()).unwrap();
        let json = serde_json::to_string(&imported.nodes).unwrap();
        assert!(
            json.matches("data:image/png;base64,").count() >= 2,
            "{json}"
        );
        assert!(!imported
            .warnings
            .iter()
            .any(|warning| warning.contains("unavailable") || warning.contains("skipped")));
    }

    #[test]
    fn uses_entry_stem_only_when_html_has_no_title_or_explicit_name() {
        let files = vec![file("landing.html", b"<p>Hello</p>")];
        let imported = import_html_project(&files, &HtmlImportOptions::default()).unwrap();
        let Some(jian_ops_schema::node::PenNode::Frame(root)) = imported.nodes.first() else {
            panic!("root frame")
        };
        assert_eq!(root.base.name.as_deref(), Some("landing"));
    }

    #[test]
    fn strips_all_shared_wrapper_layers_and_ignores_resource_query_and_fragment() {
        let files = vec![
            file(
                "outer/dist/index.html",
                br#"<link rel="stylesheet" href="/assets/site.css?v=4"><div class="hero"></div>"#,
            ),
            file(
                "outer/dist/assets/site.css",
                b".hero{background-image:url('./hero%20image.png?rev=2#preview')}",
            ),
            file(
                "outer/dist/assets/hero image.png",
                &[0x89, 0x50, 0x4e, 0x47, 9],
            ),
        ];
        assert_eq!(select_html_entry(&files).unwrap(), "index.html");
        let imported = import_html_project(&files, &HtmlImportOptions::default()).unwrap();
        let json = serde_json::to_string(&imported.nodes).unwrap();
        assert!(json.contains("data:image/png;base64,"), "{json}");
        assert!(!imported
            .warnings
            .iter()
            .any(|warning| warning.contains("unavailable") || warning.contains("skipped")));
    }

    #[test]
    fn imports_content_beyond_the_retired_html_and_project_byte_limits() {
        let mut html = b"<body>".to_vec();
        html.resize(10 * 1024 * 1024 + 1, b' ');
        html.extend_from_slice(b"<p>tail-marker</p></body>");
        let files = vec![
            HtmlProjectFile::new("index.html", html),
            HtmlProjectFile::new("unused-large.bin", vec![0; 17 * 1024 * 1024]),
        ];
        let imported = import_html_project(&files, &HtmlImportOptions::default()).unwrap();
        let json = serde_json::to_string(&imported.nodes).unwrap();
        assert!(
            json.contains("tail-marker"),
            "tail content must not be truncated"
        );
    }
}
