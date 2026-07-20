use super::cascade::{CascadeLayer, StyleOrigin, StyleRule};
use super::cascade_conditions::{media_list, supports_condition};
use super::declarations::parse_declarations;
use super::selectors::parse_selector_list;
use std::collections::{BTreeMap, BTreeSet};

const MAX_AT_RULE_DEPTH: usize = 64;

/// Stateful stylesheet parser. Keeping one instance across author sheets also
/// keeps named cascade-layer order stable when a layer is reopened later.
#[derive(Debug)]
pub struct StylesheetParser {
    origin: StyleOrigin,
    order: usize,
    layers: BTreeMap<String, CascadeLayer>,
    next_layers: BTreeMap<String, u32>,
    next_anonymous_layer: u32,
}

impl StylesheetParser {
    pub fn new(origin: StyleOrigin, first_order: usize) -> Self {
        Self {
            origin,
            order: first_order,
            layers: BTreeMap::new(),
            next_layers: BTreeMap::new(),
            next_anonymous_layer: 0,
        }
    }

    pub fn next_order(&self) -> usize {
        self.order
    }

    pub fn parse(&mut self, css: &str) -> (Vec<StyleRule>, Vec<String>) {
        self.parse_inner(css, None)
    }

    pub fn parse_for_viewport(
        &mut self,
        css: &str,
        viewport_w: f64,
        viewport_h: f64,
    ) -> (Vec<StyleRule>, Vec<String>) {
        self.parse_inner(css, Some((viewport_w, viewport_h)))
    }

    fn parse_inner(
        &mut self,
        css: &str,
        viewport: Option<(f64, f64)>,
    ) -> (Vec<StyleRule>, Vec<String>) {
        let cleaned = strip_comments(css);
        let mut parser = SheetParser {
            css: &cleaned,
            context: self,
            viewport,
            rules: Vec::new(),
            warnings: Vec::new(),
            warned: BTreeSet::new(),
        };
        parser.parse_range(0, cleaned.len(), 0, None, None);
        (parser.rules, parser.warnings)
    }

    fn register_layer(&mut self, name: String) -> CascadeLayer {
        let mut qualified = String::new();
        let mut parent = String::new();
        let mut path = Vec::new();
        for component in name.split('.') {
            if !qualified.is_empty() {
                qualified.push('.');
            }
            qualified.push_str(component);
            if let Some(layer) = self.layers.get(&qualified) {
                path = layer.path.clone();
            } else {
                let next = self.next_layers.entry(parent.clone()).or_default();
                path.push(*next);
                *next = next.saturating_add(1);
                self.layers
                    .insert(qualified.clone(), CascadeLayer { path: path.clone() });
            }
            parent.clone_from(&qualified);
        }
        CascadeLayer { path }
    }

    fn anonymous_layer(&mut self, parent: Option<&str>) -> (String, CascadeLayer) {
        let suffix = self.next_anonymous_layer;
        self.next_anonymous_layer = self.next_anonymous_layer.saturating_add(1);
        let local = format!("__anonymous_{suffix}");
        let name = qualify_layer(parent, &local);
        let order = self.register_layer(name.clone());
        (name, order)
    }
}

pub fn parse_stylesheet(css: &str, first_order: usize) -> (Vec<StyleRule>, Vec<String>) {
    parse_stylesheet_with_origin(css, first_order, StyleOrigin::Author)
}

pub fn parse_stylesheet_with_origin(
    css: &str,
    first_order: usize,
    origin: StyleOrigin,
) -> (Vec<StyleRule>, Vec<String>) {
    StylesheetParser::new(origin, first_order).parse(css)
}

pub fn parse_stylesheet_for_viewport(
    css: &str,
    first_order: usize,
    viewport_w: f64,
    viewport_h: f64,
) -> (Vec<StyleRule>, Vec<String>) {
    parse_stylesheet_for_viewport_with_origin(
        css,
        first_order,
        viewport_w,
        viewport_h,
        StyleOrigin::Author,
    )
}

pub fn parse_stylesheet_for_viewport_with_origin(
    css: &str,
    first_order: usize,
    viewport_w: f64,
    viewport_h: f64,
    origin: StyleOrigin,
) -> (Vec<StyleRule>, Vec<String>) {
    StylesheetParser::new(origin, first_order).parse_for_viewport(css, viewport_w, viewport_h)
}

struct SheetParser<'a, 'b> {
    css: &'a str,
    context: &'b mut StylesheetParser,
    viewport: Option<(f64, f64)>,
    rules: Vec<StyleRule>,
    warnings: Vec<String>,
    warned: BTreeSet<String>,
}

impl SheetParser<'_, '_> {
    fn parse_range(
        &mut self,
        mut at: usize,
        end: usize,
        depth: usize,
        layer_scope: Option<String>,
        active_layer: Option<CascadeLayer>,
    ) {
        if depth > MAX_AT_RULE_DEPTH {
            self.warn(format!(
                "nested at-rule depth limit ({MAX_AT_RULE_DEPTH}) reached; inner rules ignored"
            ));
            return;
        }
        while at < end {
            at = skip_space(self.css, at, end);
            if at >= end {
                return;
            }
            let Some((delimiter, byte)) = next_delimiter(self.css, at, end) else {
                return;
            };
            let prelude = self.css[at..delimiter].trim().to_string();
            if byte == b';' {
                if prelude.starts_with('@') {
                    self.at_statement(&prelude, layer_scope.as_deref());
                }
                at = delimiter + 1;
                continue;
            }
            let Some(close) = matching(self.css, delimiter, b'{', b'}', end) else {
                self.warn("unterminated CSS rule ignored".to_string());
                return;
            };
            if prelude.starts_with('@') {
                self.at_block(
                    &prelude,
                    delimiter + 1,
                    close,
                    depth,
                    layer_scope.clone(),
                    active_layer.clone(),
                );
            } else if !prelude.is_empty() {
                let (declaration_text, contained_nesting) =
                    declarations_only(&self.css[delimiter + 1..close]);
                if contained_nesting {
                    self.warn("CSS nesting is not supported; nested style rules ignored".into());
                }
                let declarations = parse_declarations(&declaration_text);
                let selectors = parse_selector_list(&prelude);
                let order = self.context.order;
                for selector in &selectors {
                    self.rules.push(StyleRule {
                        selector: selector.clone(),
                        declarations: declarations.clone(),
                        order,
                        origin: self.context.origin,
                        layer: active_layer.clone(),
                    });
                }
                if !selectors.is_empty() {
                    self.context.order = self.context.order.saturating_add(1);
                }
            }
            at = close + 1;
        }
    }

    fn at_statement(&mut self, prelude: &str, layer_scope: Option<&str>) {
        let (name, condition) = at_name(prelude);
        match name.as_str() {
            "layer" => {
                for layer in split_top_level(condition, ",") {
                    let layer = layer.trim();
                    if valid_layer_name(layer) {
                        self.context
                            .register_layer(qualify_layer(layer_scope, layer));
                    } else if !layer.is_empty() {
                        self.warn(format!("invalid @layer name '{layer}' ignored"));
                    }
                }
            }
            "charset" | "namespace" => {}
            _ => self.warn(format!("unsupported @{name} statement ignored")),
        }
    }

    fn at_block(
        &mut self,
        prelude: &str,
        start: usize,
        end: usize,
        depth: usize,
        layer_scope: Option<String>,
        active_layer: Option<CascadeLayer>,
    ) {
        if depth >= MAX_AT_RULE_DEPTH {
            self.warn(format!(
                "nested at-rule depth limit ({MAX_AT_RULE_DEPTH}) reached; inner rules ignored"
            ));
            return;
        }
        let (name, condition) = at_name(prelude);
        match name.as_str() {
            "media" => match self.viewport {
                Some(viewport) => {
                    let (applies, warnings) = media_list(condition, viewport);
                    for warning in warnings {
                        self.warn(warning);
                    }
                    if applies {
                        self.parse_range(start, end, depth + 1, layer_scope, active_layer);
                    }
                }
                None => self.warn("@media rules ignored because no viewport was provided".into()),
            },
            "supports" => {
                if supports_condition(condition, 0).unwrap_or(false) {
                    self.parse_range(start, end, depth + 1, layer_scope, active_layer);
                }
            }
            "layer" => {
                let (qualified, layer) = if condition.trim().is_empty() {
                    self.context.anonymous_layer(layer_scope.as_deref())
                } else if valid_layer_name(condition) && !condition.contains(',') {
                    let qualified = qualify_layer(layer_scope.as_deref(), condition.trim());
                    let layer = self.context.register_layer(qualified.clone());
                    (qualified, layer)
                } else {
                    self.warn(format!("invalid @layer block name '{condition}' ignored"));
                    return;
                };
                self.parse_range(start, end, depth + 1, Some(qualified), Some(layer));
            }
            "font-face" | "property" | "keyframes" => {}
            name if name.ends_with("keyframes") => {}
            "container" => self.warn("unsupported @container block ignored".into()),
            _ => self.warn(format!("unsupported @{name} block ignored")),
        }
    }

    fn warn(&mut self, warning: String) {
        if self.warned.insert(warning.clone()) {
            self.warnings.push(warning);
        }
    }
}

fn qualify_layer(parent: Option<&str>, name: &str) -> String {
    parent.map_or_else(|| name.to_string(), |parent| format!("{parent}.{name}"))
}

fn valid_layer_name(value: &str) -> bool {
    !value.is_empty()
        && value.split('.').all(|part| {
            part.chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic() || matches!(ch, '_' | '-'))
                && part.chars().all(is_ident)
        })
}

fn at_name(prelude: &str) -> (String, &str) {
    let rest = prelude.trim_start_matches('@');
    let end = rest
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
        .unwrap_or(rest.len());
    (rest[..end].to_ascii_lowercase(), rest[end..].trim())
}

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    let (mut quote, mut escaped) = (None, false);
    while let Some(ch) = chars.next() {
        if let Some(active) = quote {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
        } else if matches!(ch, '\'' | '"') {
            quote = Some(ch);
            out.push(ch);
        } else if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut previous = '\0';
            for current in chars.by_ref() {
                if previous == '*' && current == '/' {
                    break;
                }
                previous = current;
            }
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

fn skip_space(css: &str, mut at: usize, end: usize) -> usize {
    while at < end && (css.as_bytes()[at].is_ascii_whitespace() || css.as_bytes()[at] == b';') {
        at += 1;
    }
    at
}

fn next_delimiter(css: &str, start: usize, end: usize) -> Option<(usize, u8)> {
    let (mut quote, mut escaped, mut parens, mut brackets) = (None, false, 0u32, 0u32);
    for (at, ch) in css[start..end]
        .char_indices()
        .map(|(at, ch)| (at + start, ch))
    {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => parens += 1,
            ')' => parens = parens.saturating_sub(1),
            '[' => brackets += 1,
            ']' => brackets = brackets.saturating_sub(1),
            '{' | ';' if parens == 0 && brackets == 0 => return Some((at, ch as u8)),
            _ => {}
        }
    }
    None
}

fn matching(css: &str, open: usize, left: u8, right: u8, end: usize) -> Option<usize> {
    let (left, right) = (left as char, right as char);
    let (mut depth, mut quote, mut escaped) = (0u32, None, false);
    for (at, ch) in css[open..end]
        .char_indices()
        .map(|(at, ch)| (at + open, ch))
    {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            ch if ch == left => depth += 1,
            ch if ch == right => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => {}
        }
    }
    None
}

fn declarations_only(block: &str) -> (String, bool) {
    let mut out = String::with_capacity(block.len());
    let (mut at, mut statement_start, mut token_braces) = (0, 0, 0u32);
    let (mut quote, mut escaped) = (None, false);
    let mut nested = false;
    while at < block.len() {
        let ch = block[at..].chars().next().expect("character boundary");
        let width = ch.len_utf8();
        if let Some(active) = quote {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            at += width;
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                out.push(ch);
            }
            '{' if token_braces > 0
                || looks_like_custom_declaration(&block[statement_start..at]) =>
            {
                token_braces += 1;
                out.push(ch);
            }
            '{' => {
                let Some(close) = matching(block, at, b'{', b'}', block.len()) else {
                    nested = true;
                    break;
                };
                out.push(';');
                nested = true;
                at = close + 1;
                statement_start = at;
                continue;
            }
            '}' if token_braces > 0 => {
                token_braces -= 1;
                out.push(ch);
            }
            ';' if token_braces == 0 => {
                out.push(ch);
                statement_start = at + width;
            }
            _ => out.push(ch),
        }
        at += width;
    }
    (out, nested)
}

fn looks_like_custom_declaration(input: &str) -> bool {
    let Some(colon) = top_level_delimiter(input, ':') else {
        return false;
    };
    let name = input[..colon].trim();
    name.strip_prefix("--")
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(is_ident))
}

fn top_level_delimiter(input: &str, delimiter: char) -> Option<usize> {
    let (mut parens, mut brackets, mut braces) = (0u32, 0u32, 0u32);
    let (mut quote, mut escaped) = (None, false);
    for (at, ch) in input.char_indices() {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => parens += 1,
            ')' => parens = parens.saturating_sub(1),
            '[' => brackets += 1,
            ']' => brackets = brackets.saturating_sub(1),
            '{' => braces += 1,
            '}' => braces = braces.saturating_sub(1),
            _ if ch == delimiter && parens == 0 && brackets == 0 && braces == 0 => {
                return Some(at);
            }
            _ => {}
        }
    }
    None
}

fn split_top_level<'a>(input: &'a str, separator: &str) -> Vec<&'a str> {
    let bytes = input.as_bytes();
    let (mut start, mut depth, mut quote, mut escaped) = (0, 0u32, None, false);
    let mut parts = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        if let Some(active) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == active {
                quote = None;
            }
        } else {
            match byte {
                b'\'' | b'"' => quote = Some(byte),
                b'(' => depth += 1,
                b')' => depth = depth.saturating_sub(1),
                _ => {}
            }
            let hit = depth == 0
                && if separator == "," {
                    byte == b','
                } else {
                    keyword_at(input, at, separator)
                };
            if hit {
                parts.push(input[start..at].trim());
                at += separator.len();
                start = at;
                continue;
            }
        }
        at += 1;
    }
    parts.push(input[start..].trim());
    parts
}

fn keyword_at(input: &str, at: usize, keyword: &str) -> bool {
    let Some(candidate) = input.get(at..at + keyword.len()) else {
        return false;
    };
    candidate.eq_ignore_ascii_case(keyword)
        && input[..at]
            .chars()
            .next_back()
            .is_none_or(|ch| !is_ident(ch))
        && input[at + keyword.len()..]
            .chars()
            .next()
            .is_none_or(|ch| !is_ident(ch))
}

fn is_ident(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '-' | '_') || !ch.is_ascii()
}
