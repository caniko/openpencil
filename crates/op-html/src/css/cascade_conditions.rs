use super::declarations::parse_declarations;
use super::selectors::{parse_selector_list, PseudoClass, Selector};
use crate::color::parse_css_color;
use crate::length::{parse_length, LengthCtx};

const MAX_CONDITION_DEPTH: usize = 64;

pub(super) fn media_list(input: &str, viewport: (f64, f64)) -> (bool, Vec<String>) {
    let mut warnings = Vec::new();
    let applies = split_top_level(input, ",").into_iter().any(|query| {
        match media_query(query.trim(), viewport) {
            Ok(value) => value,
            Err(reason) => {
                warnings.push(reason);
                false
            }
        }
    });
    (applies, warnings)
}

fn media_query(input: &str, viewport: (f64, f64)) -> Result<bool, String> {
    let mut query = input.trim();
    let mut negate = false;
    if let Some(rest) = strip_keyword(query, "not") {
        negate = true;
        query = rest;
    } else if let Some(rest) = strip_keyword(query, "only") {
        query = rest;
    }
    let parts = split_top_level(query, "and");
    if parts.is_empty() {
        return Err("empty @media query ignored".into());
    }
    let mut result = true;
    for (index, part) in parts.iter().enumerate() {
        let part = part.trim();
        if index == 0 && !part.starts_with('(') {
            result &= match part.to_ascii_lowercase().as_str() {
                "all" | "screen" => true,
                "print" | "speech" => false,
                other => return Err(format!("unsupported @media type '{other}' ignored")),
            };
        } else {
            result &= media_feature(part, viewport)?;
        }
    }
    Ok(if negate { !result } else { result })
}

fn media_feature(input: &str, viewport: (f64, f64)) -> Result<bool, String> {
    let inner = strip_outer_parens(input)
        .ok_or_else(|| format!("unsupported @media condition '{input}' ignored"))?;
    if let Some((name, value)) = split_once_top_level(inner, ':') {
        return media_colon_feature(name, value, viewport);
    }
    media_range_feature(inner, viewport)
}

fn media_colon_feature(name: &str, value: &str, viewport: (f64, f64)) -> Result<bool, String> {
    let name = name.trim().to_ascii_lowercase();
    let value = value.trim();
    if name == "orientation" {
        return match value.to_ascii_lowercase().as_str() {
            "portrait" => Ok(viewport.1 >= viewport.0),
            "landscape" => Ok(viewport.0 > viewport.1),
            _ => Err(format!("invalid @media orientation '{value}' ignored")),
        };
    }
    let (axis, minimum, maximum) = match name.as_str() {
        "width" => (viewport.0, false, false),
        "min-width" => (viewport.0, true, false),
        "max-width" => (viewport.0, false, true),
        "height" => (viewport.1, false, false),
        "min-height" => (viewport.1, true, false),
        "max-height" => (viewport.1, false, true),
        _ => return Err(format!("unsupported @media feature '{name}' ignored")),
    };
    let target = media_length(value, axis, viewport)?;
    Ok(if minimum {
        axis >= target
    } else if maximum {
        axis <= target
    } else {
        (axis - target).abs() < f64::EPSILON
    })
}

fn media_range_feature(input: &str, viewport: (f64, f64)) -> Result<bool, String> {
    let (operands, operators) = range_tokens(input)?;
    if !(operators.len() == 1 || operators.len() == 2) || operands.len() != operators.len() + 1 {
        return Err(format!("unsupported @media range '({input})' ignored"));
    }
    let feature_count = operands
        .iter()
        .filter(|operand| {
            matches!(
                operand.trim().to_ascii_lowercase().as_str(),
                "width" | "height"
            )
        })
        .count();
    if feature_count != 1 {
        return Err(format!("unsupported @media range '({input})' ignored"));
    }
    let axis = if operands
        .iter()
        .any(|operand| operand.trim().eq_ignore_ascii_case("width"))
    {
        viewport.0
    } else {
        viewport.1
    };
    let values = operands
        .iter()
        .map(|operand| {
            if matches!(
                operand.trim().to_ascii_lowercase().as_str(),
                "width" | "height"
            ) {
                Ok(axis)
            } else {
                media_length(operand, axis, viewport)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(operators
        .iter()
        .enumerate()
        .all(|(index, operator)| compare_range(values[index], operator, values[index + 1])))
}

fn range_tokens(input: &str) -> Result<(Vec<&str>, Vec<&str>), String> {
    let mut operands = Vec::new();
    let mut operators = Vec::new();
    let mut start = 0;
    let bytes = input.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if matches!(bytes[at], b'<' | b'>' | b'=') {
            operands.push(input[start..at].trim());
            let width = usize::from(bytes.get(at + 1) == Some(&b'=')) + 1;
            operators.push(&input[at..at + width]);
            at += width;
            start = at;
        } else {
            at += 1;
        }
    }
    operands.push(input[start..].trim());
    if operands.iter().any(|operand| operand.is_empty()) {
        return Err(format!("invalid @media range '({input})' ignored"));
    }
    Ok((operands, operators))
}

fn compare_range(left: f64, operator: &str, right: f64) -> bool {
    match operator {
        "<" => left < right,
        "<=" => left <= right,
        ">" => left > right,
        ">=" => left >= right,
        "=" => (left - right).abs() < f64::EPSILON,
        _ => false,
    }
}

fn media_length(value: &str, axis: f64, viewport: (f64, f64)) -> Result<f64, String> {
    let context = LengthCtx {
        font_size: 16.0,
        root_font_size: 16.0,
        viewport_w: viewport.0,
        viewport_h: viewport.1,
    };
    parse_length(value.trim(), &context)
        .map(|length| length.resolve(axis))
        .ok_or_else(|| format!("invalid @media length '{}' ignored", value.trim()))
}

pub(super) fn supports_condition(input: &str, depth: usize) -> Option<bool> {
    if depth >= MAX_CONDITION_DEPTH {
        return None;
    }
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if let Some(rest) = strip_keyword(input, "not") {
        return supports_condition(rest, depth + 1).map(|value| !value);
    }
    for operator in ["or", "and"] {
        let parts = split_top_level(input, operator);
        if parts.len() > 1 {
            let values = parts
                .into_iter()
                .map(|part| supports_condition(part, depth + 1))
                .collect::<Option<Vec<_>>>()?;
            return Some(if operator == "or" {
                values.into_iter().any(|value| value)
            } else {
                values.into_iter().all(|value| value)
            });
        }
    }
    if let Some(body) = function_body(input, "selector") {
        return Some(supports_selector(body));
    }
    if let Some(inner) = strip_outer_parens(input) {
        if split_once_top_level(inner, ':').is_some() {
            return Some(supports_declaration(inner));
        }
        return supports_condition(inner, depth + 1);
    }
    if split_once_top_level(input, ':').is_some() {
        return Some(supports_declaration(input));
    }
    Some(false)
}

fn supports_selector(input: &str) -> bool {
    let selectors = parse_selector_list(input);
    !selectors.is_empty() && selectors.iter().all(selector_is_supported)
}

fn selector_is_supported(selector: &Selector) -> bool {
    selector.compounds.iter().all(|compound| {
        compound.pseudo_classes.iter().all(|pseudo| match pseudo {
            PseudoClass::Unsupported(_) => false,
            PseudoClass::Not(selectors)
            | PseudoClass::Is(selectors)
            | PseudoClass::Where(selectors)
            | PseudoClass::NthChildOf(_, selectors)
            | PseudoClass::NthLastChildOf(_, selectors) => {
                selectors.iter().all(selector_is_supported)
            }
            PseudoClass::Has(selectors) => selectors
                .iter()
                .all(|relative| selector_is_supported(&relative.selector)),
            _ => true,
        })
    })
}

fn supports_declaration(input: &str) -> bool {
    let Some((name, value)) = split_once_top_level(input, ':') else {
        return false;
    };
    let name = name.trim().to_ascii_lowercase();
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    if name.starts_with("--") {
        return name.len() > 2;
    }
    if !supported_property(&name) {
        return false;
    }
    if parse_declarations(&format!("{name}:{value}")).is_empty() {
        return false;
    }
    match name.as_str() {
        "display" => matches!(
            value.to_ascii_lowercase().as_str(),
            "none"
                | "contents"
                | "block"
                | "inline"
                | "inline-block"
                | "flow-root"
                | "flex"
                | "inline-flex"
                | "grid"
                | "inline-grid"
                | "list-item"
                | "table"
        ),
        "position" => matches!(
            value.to_ascii_lowercase().as_str(),
            "static" | "relative" | "absolute" | "fixed" | "sticky"
        ),
        "color" | "background-color" | "border-color" => {
            parse_css_color(value).is_some()
                || value.eq_ignore_ascii_case("currentcolor")
                || value.trim_start().to_ascii_lowercase().starts_with("var(")
        }
        "opacity" => {
            value
                .parse::<f64>()
                .is_ok_and(|number| (0.0..=1.0).contains(&number))
                || value.trim_start().to_ascii_lowercase().starts_with("var(")
        }
        "width" | "height" | "min-width" | "min-height" | "max-width" | "max-height" | "top"
        | "right" | "bottom" | "left" | "margin" | "margin-top" | "margin-right"
        | "margin-bottom" | "margin-left" | "padding" | "padding-top" | "padding-right"
        | "padding-bottom" | "padding-left" | "gap" | "row-gap" | "column-gap" | "font-size"
        | "letter-spacing" => supports_length(value),
        _ => true,
    }
}

fn supports_length(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    let context = LengthCtx {
        font_size: 16.0,
        root_font_size: 16.0,
        viewport_w: 1280.0,
        viewport_h: 800.0,
    };
    let valid = |part: &str| {
        matches!(
            part,
            "auto" | "none" | "normal" | "min-content" | "max-content" | "fit-content"
        ) || part.starts_with("var(")
            || parse_length(part, &context).is_some()
    };
    if valid(&lower) {
        return true;
    }
    lower.split_ascii_whitespace().all(valid)
}

fn supported_property(name: &str) -> bool {
    matches!(
        name,
        "align-content"
            | "align-items"
            | "align-self"
            | "appearance"
            | "backdrop-filter"
            | "background"
            | "background-attachment"
            | "background-blend-mode"
            | "background-clip"
            | "background-color"
            | "background-image"
            | "background-origin"
            | "background-position"
            | "background-repeat"
            | "background-size"
            | "border"
            | "border-block"
            | "border-bottom"
            | "border-color"
            | "border-inline"
            | "border-left"
            | "border-radius"
            | "border-right"
            | "border-style"
            | "border-top"
            | "border-width"
            | "bottom"
            | "box-shadow"
            | "box-sizing"
            | "color"
            | "column-gap"
            | "columns"
            | "content"
            | "cursor"
            | "display"
            | "filter"
            | "flex"
            | "flex-basis"
            | "flex-direction"
            | "flex-flow"
            | "flex-grow"
            | "flex-shrink"
            | "flex-wrap"
            | "float"
            | "font"
            | "font-family"
            | "font-feature-settings"
            | "font-size"
            | "font-stretch"
            | "font-style"
            | "font-variant"
            | "font-weight"
            | "gap"
            | "grid-template-columns"
            | "height"
            | "inset"
            | "justify-content"
            | "justify-items"
            | "justify-self"
            | "left"
            | "letter-spacing"
            | "line-height"
            | "margin"
            | "margin-block"
            | "margin-bottom"
            | "margin-inline"
            | "margin-left"
            | "margin-right"
            | "margin-top"
            | "max-height"
            | "max-width"
            | "min-height"
            | "min-width"
            | "mix-blend-mode"
            | "object-fit"
            | "opacity"
            | "outline"
            | "overflow"
            | "overflow-x"
            | "overflow-y"
            | "padding"
            | "padding-block"
            | "padding-bottom"
            | "padding-inline"
            | "padding-left"
            | "padding-right"
            | "padding-top"
            | "place-content"
            | "place-items"
            | "place-self"
            | "position"
            | "right"
            | "row-gap"
            | "text-align"
            | "text-decoration"
            | "text-decoration-line"
            | "text-transform"
            | "top"
            | "transform"
            | "visibility"
            | "white-space"
            | "width"
            | "z-index"
    )
}

fn function_body<'a>(input: &'a str, name: &str) -> Option<&'a str> {
    let prefix = input.get(..name.len())?;
    if !prefix.eq_ignore_ascii_case(name) || input.as_bytes().get(name.len()) != Some(&b'(') {
        return None;
    }
    let close = matching(input, name.len(), b'(', b')', input.len())?;
    (close + 1 == input.len()).then(|| input[name.len() + 1..close].trim())
}

fn strip_outer_parens(input: &str) -> Option<&str> {
    let input = input.trim();
    if !input.starts_with('(') {
        return None;
    }
    let close = matching(input, 0, b'(', b')', input.len())?;
    (close + 1 == input.len()).then(|| input[1..close].trim())
}

fn matching(input: &str, open: usize, left: u8, right: u8, end: usize) -> Option<usize> {
    let (left, right) = (left as char, right as char);
    let (mut depth, mut quote, mut escaped) = (0u32, None, false);
    for (at, ch) in input[open..end]
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

fn split_once_top_level(input: &str, delimiter: char) -> Option<(&str, &str)> {
    let at = top_level_delimiter(input, delimiter)?;
    Some((&input[..at], &input[at + delimiter.len_utf8()..]))
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

fn strip_keyword<'a>(input: &'a str, keyword: &str) -> Option<&'a str> {
    keyword_at(input, 0, keyword).then(|| input[keyword.len()..].trim_start())
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
