use crate::color::parse_css_color;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    pub name: String,
    pub value: String,
    pub important: bool,
}

pub fn parse_declarations(block: &str) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    for statement in split_top_level(block, ';') {
        let Some((name, raw_value)) = statement.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let (value, important) = strip_important(raw_value);
        if name.is_empty() || value.is_empty() {
            continue;
        }
        match name.as_str() {
            "margin" | "padding" => expand_box(&mut declarations, &name, value, important),
            "border" => expand_border(&mut declarations, None, value, important),
            "border-top" | "border-right" | "border-bottom" | "border-left" => expand_border(
                &mut declarations,
                name.strip_prefix("border-"),
                value,
                important,
            ),
            "background" => expand_background(&mut declarations, value, important),
            "font" => expand_font(&mut declarations, value, important),
            "gap" => {
                if let Some(first) = value.split_whitespace().next() {
                    push(&mut declarations, "gap", first, important);
                }
            }
            "flex" => {
                if let Some(first) = value.split_whitespace().next() {
                    if first.parse::<f64>().is_ok() {
                        push(&mut declarations, "flex-grow", first, important);
                    }
                }
            }
            "text-decoration" => {
                if value.split_whitespace().any(|part| part == "underline") {
                    push(
                        &mut declarations,
                        "text-decoration-line",
                        "underline",
                        important,
                    );
                }
                if value.split_whitespace().any(|part| part == "line-through") {
                    push(
                        &mut declarations,
                        "text-decoration-line",
                        "line-through",
                        important,
                    );
                }
            }
            _ => push(&mut declarations, &name, value, important),
        }
    }
    declarations
}

fn split_top_level(input: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0u32;
    let mut quote = None;
    for (index, ch) in input.char_indices() {
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if ch == delimiter && depth == 0 => {
                parts.push(input[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(input[start..].trim());
    parts
}

fn strip_important(value: &str) -> (&str, bool) {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.ends_with("!important") {
        let cutoff = trimmed.len() - "!important".len();
        (trimmed[..cutoff].trim_end(), true)
    } else {
        (trimmed, false)
    }
}

fn push(out: &mut Vec<Declaration>, name: &str, value: &str, important: bool) {
    out.push(Declaration {
        name: name.to_string(),
        value: value.trim().to_string(),
        important,
    });
}

fn expand_box(out: &mut Vec<Declaration>, name: &str, value: &str, important: bool) {
    let values: Vec<_> = value.split_whitespace().collect();
    let sides = match values.as_slice() {
        [all] => [*all, *all, *all, *all],
        [vertical, horizontal] => [*vertical, *horizontal, *vertical, *horizontal],
        [top, horizontal, bottom] => [*top, *horizontal, *bottom, *horizontal],
        [top, right, bottom, left] => [*top, *right, *bottom, *left],
        _ => return,
    };
    for (side, value) in ["top", "right", "bottom", "left"].into_iter().zip(sides) {
        push(out, &format!("{name}-{side}"), value, important);
    }
}

fn expand_border(out: &mut Vec<Declaration>, side: Option<&str>, value: &str, important: bool) {
    let mut width = None;
    let mut style = None;
    let mut color = None;
    for token in value.split_whitespace() {
        if parse_css_color(token).is_some() {
            color = Some(token);
        } else if is_border_width(token) {
            width = Some(token);
        } else if matches!(
            token,
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
        ) {
            style = Some(token);
        }
    }
    let prefix = side.map_or_else(|| "border".to_string(), |side| format!("border-{side}"));
    if let Some(width) = width {
        push(out, &format!("{prefix}-width"), width, important);
    }
    if side.is_none() {
        if let Some(style) = style {
            push(out, "border-style", style, important);
        }
    }
    if let Some(color) = color {
        push(out, &format!("{prefix}-color"), color, important);
    }
}

fn is_border_width(token: &str) -> bool {
    matches!(token, "thin" | "medium" | "thick" | "0")
        || ["px", "em", "rem", "pt"].iter().any(|unit| {
            token
                .strip_suffix(unit)
                .is_some_and(|n| n.parse::<f64>().is_ok())
        })
}

fn expand_background(out: &mut Vec<Declaration>, value: &str, important: bool) {
    if value.contains("-gradient(") {
        push(out, "background-image", value, important);
        return;
    }
    if let Some(image) = extract_function(value, "url(") {
        push(out, "background-image", image, important);
    }
    if parse_css_color(value).is_some() {
        push(out, "background-color", value, important);
        return;
    }
    if let Some(color) = value
        .split_whitespace()
        .find(|token| parse_css_color(token).is_some())
    {
        push(out, "background-color", color, important);
    }
}

fn extract_function<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let start = value.find(prefix)?;
    let mut depth = 0u32;
    for (offset, ch) in value[start..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(&value[start..start + offset + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

fn expand_font(out: &mut Vec<Declaration>, value: &str, important: bool) {
    let tokens: Vec<_> = value.split_whitespace().collect();
    let Some(size_index) = tokens.iter().position(|token| is_font_size_token(token)) else {
        return;
    };
    for token in &tokens[..size_index] {
        if *token == "italic" || *token == "oblique" {
            push(out, "font-style", token, important);
        } else if *token == "bold"
            || token
                .parse::<u16>()
                .is_ok_and(|weight| (100..=900).contains(&weight))
        {
            push(out, "font-weight", token, important);
        }
    }
    let (size, line_height) = tokens[size_index]
        .split_once('/')
        .map_or((tokens[size_index], None), |(size, height)| {
            (size, Some(height))
        });
    push(out, "font-size", size, important);
    if let Some(line_height) = line_height {
        push(out, "line-height", line_height, important);
    }
    if size_index + 1 < tokens.len() {
        push(
            out,
            "font-family",
            &tokens[size_index + 1..].join(" "),
            important,
        );
    }
}

fn is_font_size_token(token: &str) -> bool {
    let size = token.split('/').next().unwrap_or(token);
    ["px", "em", "rem", "pt", "%"].iter().any(|unit| {
        size.strip_suffix(unit)
            .is_some_and(|n| n.parse::<f64>().is_ok())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get<'a>(declarations: &'a [Declaration], name: &str) -> Option<&'a str> {
        declarations
            .iter()
            .rev()
            .find(|declaration| declaration.name == name)
            .map(|declaration| declaration.value.as_str())
    }

    #[test]
    fn margin_shorthand_expands() {
        let declarations = parse_declarations("margin: 10px 20px");
        assert_eq!(get(&declarations, "margin-top"), Some("10px"));
        assert_eq!(get(&declarations, "margin-right"), Some("20px"));
        assert_eq!(get(&declarations, "margin-bottom"), Some("10px"));
        assert_eq!(get(&declarations, "margin-left"), Some("20px"));
    }

    #[test]
    fn border_and_background() {
        let declarations =
            parse_declarations("border: 1px solid #000; background: #fff url(x.png)");
        assert_eq!(get(&declarations, "border-width"), Some("1px"));
        assert_eq!(get(&declarations, "border-color"), Some("#000"));
        assert_eq!(get(&declarations, "background-image"), Some("url(x.png)"));
        assert_eq!(get(&declarations, "background-color"), Some("#fff"));
    }

    #[test]
    fn important_flag_and_case() {
        let declarations = parse_declarations("COLOR: red !important");
        assert_eq!(declarations[0].name, "color");
        assert_eq!(declarations[0].value, "red");
        assert!(declarations[0].important);
    }

    #[test]
    fn gradient_goes_to_background_image() {
        let declarations = parse_declarations("background: linear-gradient(90deg, #000, #fff)");
        assert_eq!(
            get(&declarations, "background-image"),
            Some("linear-gradient(90deg, #000, #fff)")
        );
        assert_eq!(get(&declarations, "background-color"), None);
    }

    #[test]
    fn flex_and_font() {
        let declarations =
            parse_declarations("flex: 1; font: italic 700 18px/1.4 Inter, sans-serif");
        assert_eq!(get(&declarations, "flex-grow"), Some("1"));
        assert_eq!(get(&declarations, "font-style"), Some("italic"));
        assert_eq!(get(&declarations, "font-weight"), Some("700"));
        assert_eq!(get(&declarations, "font-size"), Some("18px"));
        assert_eq!(get(&declarations, "line-height"), Some("1.4"));
        assert_eq!(get(&declarations, "font-family"), Some("Inter, sans-serif"));
    }
}
