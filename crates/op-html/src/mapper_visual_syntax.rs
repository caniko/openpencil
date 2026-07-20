pub(super) fn exact_function_body<'a>(value: &'a str, name: &str) -> Option<&'a str> {
    let value = value.trim();
    let prefix = value.get(..name.len())?;
    if !prefix.eq_ignore_ascii_case(name) || value.as_bytes().get(name.len()) != Some(&b'(') {
        return None;
    }
    let close = matching_paren(value, name.len())?;
    (close + 1 == value.len()).then_some(&value[name.len() + 1..close])
}

pub(super) fn matching_paren(value: &str, open: usize) -> Option<usize> {
    if value.as_bytes().get(open) != Some(&b'(') {
        return None;
    }
    let mut depth = 0u32;
    let mut quote = None;
    let mut escaped = false;
    for (offset, ch) in value[open..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn split_top_level(input: &str, delimiter: char) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depths = [0u32; 3];
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => depths[0] += 1,
            ')' => depths[0] = depths[0].saturating_sub(1),
            '[' => depths[1] += 1,
            ']' => depths[1] = depths[1].saturating_sub(1),
            '{' => depths[2] += 1,
            '}' => depths[2] = depths[2].saturating_sub(1),
            _ if ch == delimiter && depths == [0, 0, 0] => {
                result.push(input[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    result.push(input[start..].trim());
    result
}

pub(super) fn split_whitespace_top_level(input: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = None;
    let mut depths = [0u32; 3];
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in input.char_indices() {
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
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            }
            start.get_or_insert(index);
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                start.get_or_insert(index);
            }
            '(' => {
                depths[0] += 1;
                start.get_or_insert(index);
            }
            ')' => depths[0] = depths[0].saturating_sub(1),
            '[' => {
                depths[1] += 1;
                start.get_or_insert(index);
            }
            ']' => depths[1] = depths[1].saturating_sub(1),
            '{' => {
                depths[2] += 1;
                start.get_or_insert(index);
            }
            '}' => depths[2] = depths[2].saturating_sub(1),
            _ if ch.is_whitespace() && depths == [0, 0, 0] => {
                if let Some(token_start) = start.take() {
                    result.push(&input[token_start..index]);
                }
            }
            _ => {
                start.get_or_insert(index);
            }
        }
    }
    if let Some(token_start) = start {
        result.push(&input[token_start..]);
    }
    result
}

pub(super) fn strip_interpolation_method(value: &str) -> (&str, bool) {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("in ") {
        ("", true)
    } else if let Some(index) = lower.find(" in ") {
        (value[..index].trim(), true)
    } else {
        (value, false)
    }
}

pub(super) fn parse_gradient_angle(value: &str) -> Option<f32> {
    let value = value.trim().to_ascii_lowercase();
    for (suffix, scale) in [
        ("deg", 1.0),
        ("turn", 360.0),
        ("rad", 180.0 / std::f32::consts::PI),
        ("grad", 0.9),
    ] {
        if let Some(number) = value.strip_suffix(suffix) {
            return number.trim().parse::<f32>().ok().map(|value| value * scale);
        }
    }
    match value.as_str() {
        "to top" => Some(0.0),
        "to top right" | "to right top" => Some(45.0),
        "to right" => Some(90.0),
        "to bottom right" | "to right bottom" => Some(135.0),
        "to bottom" => Some(180.0),
        "to bottom left" | "to left bottom" => Some(225.0),
        "to left" => Some(270.0),
        "to top left" | "to left top" => Some(315.0),
        _ => None,
    }
}

pub(super) fn parse_radial_position(value: &str) -> Option<(f32, f32)> {
    let tokens: Vec<_> = value.split_whitespace().collect();
    if tokens.is_empty() || tokens.len() > 2 {
        return None;
    }
    let percent = |token: &str| {
        token
            .strip_suffix('%')
            .and_then(|number| number.parse::<f32>().ok())
            .map(|number| number / 100.0)
    };
    let (mut x, mut y, mut ambiguous) = (None, None, Vec::new());
    for token in tokens {
        match token {
            "left" => x = Some(0.0),
            "right" => x = Some(1.0),
            "top" => y = Some(0.0),
            "bottom" => y = Some(1.0),
            "center" => ambiguous.push(0.5),
            _ => ambiguous.push(percent(token)?),
        }
    }
    for value in ambiguous {
        if x.is_none() {
            x = Some(value);
        } else if y.is_none() {
            y = Some(value);
        } else {
            return None;
        }
    }
    Some((x.unwrap_or(0.5), y.unwrap_or(0.5)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizer_preserves_functions_quotes_and_nested_delimiters() {
        assert_eq!(
            split_top_level("url(\"a,b c.png\"), linear-gradient(rgb(1 2 3), #fff)", ','),
            vec!["url(\"a,b c.png\")", "linear-gradient(rgb(1 2 3), #fff)"]
        );
        assert_eq!(
            split_whitespace_top_level("0 4px rgb(0 0 0 / 25%) inset"),
            vec!["0", "4px", "rgb(0 0 0 / 25%)", "inset"]
        );
        assert_eq!(
            exact_function_body("VaR(--tone, rgb(1 2 3))", "var"),
            Some("--tone, rgb(1 2 3)")
        );
    }
}
