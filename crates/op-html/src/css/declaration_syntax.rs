pub(super) fn is_border_width(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "thin" | "medium" | "thick"
    ) || value == "0"
        || starts_length_function(value)
        || value
            .trim_end_matches(|ch: char| ch.is_ascii_alphabetic() || ch == '%')
            .parse::<f64>()
            .is_ok()
}

pub(super) fn is_border_style(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "none"
            | "hidden"
            | "dotted"
            | "dashed"
            | "solid"
            | "double"
            | "groove"
            | "ridge"
            | "inset"
            | "outset"
    )
}

pub(super) fn is_font_weight(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "normal" | "bold" | "bolder" | "lighter"
    ) || value
        .parse::<u16>()
        .is_ok_and(|weight| (1..=1000).contains(&weight))
}

pub(super) fn is_font_size_token(value: &str) -> bool {
    let size = split_once_top_level(value, '/')
        .map_or(value, |(size, _)| size)
        .trim();
    matches!(
        size.to_ascii_lowercase().as_str(),
        "xx-small"
            | "x-small"
            | "small"
            | "medium"
            | "large"
            | "x-large"
            | "xx-large"
            | "xxx-large"
            | "smaller"
            | "larger"
    ) || starts_length_function(size)
        || is_font_dimension(size)
}

fn is_font_dimension(value: &str) -> bool {
    if value == "0" {
        return true;
    }
    let unit_start = value
        .char_indices()
        .find_map(|(index, ch)| (ch.is_ascii_alphabetic() || ch == '%').then_some(index));
    let Some(unit_start) = unit_start else {
        return false;
    };
    value[..unit_start].parse::<f64>().is_ok()
        && matches!(
            &value[unit_start..],
            "px" | "em"
                | "rem"
                | "%"
                | "vw"
                | "vh"
                | "vmin"
                | "vmax"
                | "pt"
                | "pc"
                | "in"
                | "cm"
                | "mm"
                | "q"
                | "ch"
                | "ex"
        )
}

fn starts_length_function(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    ["var(", "calc(", "min(", "max(", "clamp("]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

pub(super) fn is_background_repeat(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "repeat" | "repeat-x" | "repeat-y" | "no-repeat" | "space" | "round"
    )
}

pub(super) fn is_background_position(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "left" | "center" | "right" | "top" | "bottom"
    ) || value.ends_with('%')
        || value == "0"
        || value
            .trim_end_matches(|ch: char| ch.is_ascii_alphabetic())
            .parse::<f64>()
            .is_ok()
}

pub(super) fn is_css_wide(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer"
    )
}

pub(super) fn strip_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0;
    let mut quote = None;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if let Some(active) = quote {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            index += 1;
            continue;
        }
        if escaped {
            escaped = false;
            out.push(ch);
            index += 1;
        } else if ch == '\\' {
            escaped = true;
            out.push(ch);
            index += 1;
        } else if matches!(ch, '\'' | '"') {
            quote = Some(ch);
            out.push(ch);
            index += 1;
        } else if ch == '/' && chars.get(index + 1) == Some(&'*') {
            index += 2;
            while index + 1 < chars.len() && !(chars[index] == '*' && chars[index + 1] == '/') {
                index += 1;
            }
            index = (index + 2).min(chars.len());
            out.push(' ');
        } else {
            out.push(ch);
            index += 1;
        }
    }
    out
}

pub(super) fn split_top_level(input: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut paren = 0u32;
    let mut bracket = 0u32;
    let mut brace = 0u32;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in input.char_indices() {
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
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => paren += 1,
            ')' => paren = paren.saturating_sub(1),
            '[' => bracket += 1,
            ']' => bracket = bracket.saturating_sub(1),
            '{' => brace += 1,
            '}' => brace = brace.saturating_sub(1),
            _ if ch == delimiter && paren == 0 && bracket == 0 && brace == 0 => {
                parts.push(input[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(input[start..].trim());
    parts
}

pub(super) fn split_whitespace_top_level(input: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut start = None;
    let mut paren = 0u32;
    let mut bracket = 0u32;
    let mut brace = 0u32;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in input.char_indices() {
        if let Some(active) = quote {
            start.get_or_insert(index);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            continue;
        }
        if escaped {
            escaped = false;
            start.get_or_insert(index);
            continue;
        }
        if ch == '\\' {
            escaped = true;
            start.get_or_insert(index);
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                start.get_or_insert(index);
            }
            '(' => {
                paren += 1;
                start.get_or_insert(index);
            }
            ')' => paren = paren.saturating_sub(1),
            '[' => {
                bracket += 1;
                start.get_or_insert(index);
            }
            ']' => bracket = bracket.saturating_sub(1),
            '{' => {
                brace += 1;
                start.get_or_insert(index);
            }
            '}' => brace = brace.saturating_sub(1),
            '/' if paren == 0 && bracket == 0 && brace == 0 => {
                if let Some(token_start) = start.take() {
                    result.push(input[token_start..index].to_string());
                }
                result.push("/".to_string());
            }
            _ if ch.is_whitespace() && paren == 0 && bracket == 0 && brace == 0 => {
                if let Some(token_start) = start.take() {
                    result.push(input[token_start..index].to_string());
                }
            }
            _ => {
                start.get_or_insert(index);
            }
        }
    }
    if let Some(token_start) = start {
        result.push(input[token_start..].to_string());
    }
    result
}

pub(super) fn find_top_level(input: &str, needle: char) -> Option<usize> {
    let mut paren = 0u32;
    let mut bracket = 0u32;
    let mut brace = 0u32;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in input.char_indices() {
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
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => paren += 1,
            ')' => paren = paren.saturating_sub(1),
            '[' => bracket += 1,
            ']' => bracket = bracket.saturating_sub(1),
            '{' => brace += 1,
            '}' => brace = brace.saturating_sub(1),
            _ if ch == needle && paren == 0 && bracket == 0 && brace == 0 => return Some(index),
            _ => {}
        }
    }
    None
}

pub(super) fn split_once_top_level(input: &str, needle: char) -> Option<(&str, &str)> {
    let index = find_top_level(input, needle)?;
    Some((&input[..index], &input[index + needle.len_utf8()..]))
}

pub(super) fn strip_important(value: &str) -> (&str, bool) {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    let Some(index) = lower.rfind("!important") else {
        return (trimmed, false);
    };
    if lower[index + "!important".len()..].trim().is_empty()
        && find_top_level(&trimmed[index..], '!') == Some(0)
    {
        (trimmed[..index].trim_end(), true)
    } else {
        (trimmed, false)
    }
}

pub(super) fn extract_function(value: &str, start: usize) -> Option<String> {
    let open = value[start..].find('(')? + start;
    let mut depth = 0u32;
    let mut quote = None;
    let mut escaped = false;
    for (offset, ch) in value[open..].char_indices() {
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
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(value[start..open + offset + 1].trim().to_string());
                }
            }
            _ => {}
        }
    }
    None
}
