pub fn parse_css_color(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    if let Some(hex) = value.strip_prefix('#') {
        return parse_hex(hex);
    }
    if value.starts_with("rgb(") || value.starts_with("rgba(") {
        return parse_rgb(&value);
    }
    if value.starts_with("hsl(") || value.starts_with("hsla(") {
        return parse_hsl(&value);
    }
    match value.as_str() {
        "black" => Some("#000000".into()),
        "white" => Some("#ffffff".into()),
        "red" => Some("#ff0000".into()),
        "green" => Some("#008000".into()),
        "blue" => Some("#0000ff".into()),
        "yellow" => Some("#ffff00".into()),
        "orange" => Some("#ffa500".into()),
        "purple" => Some("#800080".into()),
        "pink" => Some("#ffc0cb".into()),
        "gray" | "grey" => Some("#808080".into()),
        "silver" => Some("#c0c0c0".into()),
        "transparent" => Some("#00000000".into()),
        "currentcolor" => None,
        _ => None,
    }
}

fn parse_hex(hex: &str) -> Option<String> {
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let expanded = match hex.len() {
        3 | 4 => hex.chars().flat_map(|ch| [ch, ch]).collect::<String>(),
        6 | 8 => hex.to_string(),
        _ => return None,
    };
    let r = u8::from_str_radix(&expanded[0..2], 16).ok()?;
    let g = u8::from_str_radix(&expanded[2..4], 16).ok()?;
    let b = u8::from_str_radix(&expanded[4..6], 16).ok()?;
    let a = if expanded.len() == 8 {
        u8::from_str_radix(&expanded[6..8], 16).ok()?
    } else {
        u8::MAX
    };
    Some(format_rgba(r, g, b, a))
}

fn function_parts(value: &str) -> Option<Vec<&str>> {
    let (_, body) = value.split_once('(')?;
    let body = body.strip_suffix(')')?;
    Some(
        body.split(|ch: char| ch == ',' || ch == '/' || ch.is_whitespace())
            .filter(|part| !part.is_empty())
            .collect(),
    )
}

fn parse_rgb(value: &str) -> Option<String> {
    let parts = function_parts(value)?;
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let r = parse_rgb_component(parts[0])?;
    let g = parse_rgb_component(parts[1])?;
    let b = parse_rgb_component(parts[2])?;
    let a = parts
        .get(3)
        .map_or(Some(u8::MAX), |part| parse_alpha(part))?;
    Some(format_rgba(r, g, b, a))
}

fn parse_rgb_component(value: &str) -> Option<u8> {
    let number = if let Some(percent) = value.strip_suffix('%') {
        percent.parse::<f64>().ok()? * 255.0 / 100.0
    } else {
        value.parse::<f64>().ok()?
    };
    Some(number.clamp(0.0, 255.0).round() as u8)
}

fn parse_alpha(value: &str) -> Option<u8> {
    let alpha = if let Some(percent) = value.strip_suffix('%') {
        percent.parse::<f64>().ok()? / 100.0
    } else {
        value.parse::<f64>().ok()?
    };
    Some((alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn parse_hsl(value: &str) -> Option<String> {
    let parts = function_parts(value)?;
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let hue = parts[0].strip_suffix("deg").unwrap_or(parts[0]);
    let hue = hue.parse::<f64>().ok()?.rem_euclid(360.0);
    let saturation = parts[1].strip_suffix('%')?.parse::<f64>().ok()? / 100.0;
    let lightness = parts[2].strip_suffix('%')?.parse::<f64>().ok()? / 100.0;
    let saturation = saturation.clamp(0.0, 1.0);
    let lightness = lightness.clamp(0.0, 1.0);
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue / 60.0;
    let x = chroma * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match sector as u8 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = lightness - chroma / 2.0;
    let channel = |value: f64| ((value + m) * 255.0).round() as u8;
    let a = parts
        .get(3)
        .map_or(Some(u8::MAX), |part| parse_alpha(part))?;
    Some(format_rgba(channel(r), channel(g), channel(b), a))
}

fn format_rgba(r: u8, g: u8, b: u8, a: u8) -> String {
    if a == u8::MAX {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_forms() {
        assert_eq!(parse_css_color("#FA3").as_deref(), Some("#ffaa33"));
        assert_eq!(parse_css_color("#ffaa33").as_deref(), Some("#ffaa33"));
        assert_eq!(parse_css_color("#ffaa3380").as_deref(), Some("#ffaa3380"));
    }

    #[test]
    fn rgb_hsl_named() {
        assert_eq!(
            parse_css_color("rgb(255, 0, 0)").as_deref(),
            Some("#ff0000")
        );
        assert_eq!(
            parse_css_color("rgba(0,0,0,0.5)").as_deref(),
            Some("#00000080")
        );
        assert_eq!(
            parse_css_color("hsl(0, 100%, 50%)").as_deref(),
            Some("#ff0000")
        );
        assert_eq!(parse_css_color("white").as_deref(), Some("#ffffff"));
        assert_eq!(parse_css_color("transparent").as_deref(), Some("#00000000"));
        assert!(parse_css_color("var(--x)").is_none());
    }
}
