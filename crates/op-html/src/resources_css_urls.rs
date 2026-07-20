use url::Url;

/// Resolves relative `url(...)` references against an external stylesheet URL.
///
/// Downstream style mapping only retains declaration values, so rebasing here
/// keeps the stylesheet's source origin without threading it through every CSS
/// rule and computed value. Strings and comments are skipped deliberately.
pub(crate) fn rebase_stylesheet_urls(source: &str, stylesheet_url: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    while cursor < source.len() {
        if source[cursor..].starts_with("/*") {
            let end = comment_end(source, cursor);
            output.push_str(&source[cursor..end]);
            cursor = end;
            continue;
        }

        let current = source[cursor..]
            .chars()
            .next()
            .expect("cursor remains on a character boundary");
        if matches!(current, '\'' | '"') {
            let Some(end) = string_end(source, cursor, current) else {
                output.push_str(&source[cursor..]);
                break;
            };
            output.push_str(&source[cursor..end]);
            cursor = end;
            continue;
        }

        if is_url_function(source, cursor) {
            let open = cursor + 3;
            let Some(close) = function_end(source, open) else {
                output.push_str(&source[cursor..]);
                break;
            };
            let argument = &source[open + 1..close];
            if let Some(parsed) = parse_url_argument(argument) {
                if let Some(resolved) = rebase_url(&parsed.value, stylesheet_url) {
                    output.push_str(&source[cursor..=open]);
                    push_quoted_url(&mut output, &resolved, parsed.quote.unwrap_or('"'));
                    output.push(')');
                    cursor = close + 1;
                    continue;
                }
            }
            output.push_str(&source[cursor..=close]);
            cursor = close + 1;
            continue;
        }

        output.push(current);
        cursor += current.len_utf8();
    }
    output
}

struct ParsedUrl {
    value: String,
    quote: Option<char>,
}

fn parse_url_argument(argument: &str) -> Option<ParsedUrl> {
    let without_comments = strip_comments(argument);
    let argument = without_comments.trim();
    if argument.is_empty() {
        return None;
    }
    let first = argument.chars().next()?;
    if matches!(first, '\'' | '"') {
        let end = string_end(argument, 0, first)?;
        if !argument[end..].trim().is_empty() {
            return None;
        }
        return Some(ParsedUrl {
            value: unescape_css(&argument[first.len_utf8()..end - first.len_utf8()], true)?,
            quote: Some(first),
        });
    }
    Some(ParsedUrl {
        value: unescape_css(argument, false)?,
        quote: None,
    })
}

fn rebase_url(reference: &str, stylesheet_url: &str) -> Option<String> {
    let reference = reference.trim();
    if reference.is_empty() || Url::parse(reference).is_ok() {
        return None;
    }
    super::resolve_url(Some(stylesheet_url), reference)
}

fn is_url_function(source: &str, start: usize) -> bool {
    let bytes = source.as_bytes();
    if start + 4 > bytes.len()
        || !bytes[start..start + 3].eq_ignore_ascii_case(b"url")
        || bytes[start + 3] != b'('
    {
        return false;
    }
    source[..start]
        .chars()
        .next_back()
        .is_none_or(|previous| !is_identifier_char(previous))
}

fn is_identifier_char(value: char) -> bool {
    value.is_alphanumeric() || value == '-' || value == '_' || value == '\\' || !value.is_ascii()
}

fn function_end(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
    let mut depth = 1usize;
    let mut cursor = open + 1;
    while cursor < source.len() {
        if source[cursor..].starts_with("/*") {
            cursor = comment_end(source, cursor);
            continue;
        }
        match bytes[cursor] {
            b'\'' | b'"' => {
                cursor = string_end(source, cursor, bytes[cursor] as char)?;
            }
            b'\\' => cursor = escaped_end(source, cursor),
            b'(' => {
                depth += 1;
                cursor += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(cursor);
                }
                cursor += 1;
            }
            _ => {
                cursor += source[cursor..]
                    .chars()
                    .next()
                    .expect("cursor remains on a character boundary")
                    .len_utf8();
            }
        }
    }
    None
}

fn string_end(source: &str, start: usize, quote: char) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut cursor = start + quote.len_utf8();
    while cursor < source.len() {
        match bytes[cursor] {
            b'\\' => cursor = escaped_end(source, cursor),
            value if value == quote as u8 => return Some(cursor + 1),
            _ => cursor += 1,
        }
    }
    None
}

fn escaped_end(source: &str, slash: usize) -> usize {
    let bytes = source.as_bytes();
    let next = slash + 1;
    if next >= bytes.len() {
        return bytes.len();
    }
    if bytes[next] == b'\r' && bytes.get(next + 1) == Some(&b'\n') {
        next + 2
    } else {
        next + source[next..].chars().next().map_or(0, char::len_utf8)
    }
}

fn comment_end(source: &str, start: usize) -> usize {
    source[start + 2..]
        .find("*/")
        .map_or(source.len(), |offset| start + 2 + offset + 2)
}

fn strip_comments(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    while cursor < source.len() {
        if source[cursor..].starts_with("/*") {
            cursor = comment_end(source, cursor);
            continue;
        }
        let current = source[cursor..]
            .chars()
            .next()
            .expect("cursor remains on a character boundary");
        if matches!(current, '\'' | '"') {
            let Some(end) = string_end(source, cursor, current) else {
                output.push_str(&source[cursor..]);
                break;
            };
            output.push_str(&source[cursor..end]);
            cursor = end;
        } else {
            output.push(current);
            cursor += current.len_utf8();
        }
    }
    output
}

fn unescape_css(source: &str, quoted: bool) -> Option<String> {
    let mut output = String::with_capacity(source.len());
    let mut characters = source.chars().peekable();
    while let Some(current) = characters.next() {
        if current != '\\' {
            if current == '\0' {
                output.push('\u{fffd}');
            } else if (!quoted
                && (current.is_ascii_whitespace() || matches!(current, '\'' | '"' | '(')))
                || (quoted && matches!(current, '\n' | '\r' | '\u{c}'))
            {
                return None;
            } else {
                output.push(current);
            }
            continue;
        }

        let escaped = characters.next()?;
        if escaped.is_ascii_hexdigit() {
            let mut digits = String::from(escaped);
            while digits.len() < 6
                && characters
                    .peek()
                    .is_some_and(|next| next.is_ascii_hexdigit())
            {
                digits.push(characters.next().expect("peeked character exists"));
            }
            if characters
                .peek()
                .is_some_and(|next| next.is_ascii_whitespace())
            {
                let whitespace = characters.next();
                if whitespace == Some('\r') && characters.peek() == Some(&'\n') {
                    characters.next();
                }
            }
            let codepoint = u32::from_str_radix(&digits, 16).ok()?;
            output.push(
                char::from_u32(codepoint)
                    .filter(|value| *value != '\0')
                    .unwrap_or('\u{fffd}'),
            );
        } else if escaped == '\r' {
            if characters.peek() == Some(&'\n') {
                characters.next();
            }
        } else if !matches!(escaped, '\n' | '\u{c}') {
            output.push(escaped);
        }
    }
    Some(output)
}

fn push_quoted_url(output: &mut String, value: &str, quote: char) {
    output.push(quote);
    for current in value.chars() {
        if current == quote || current == '\\' {
            output.push('\\');
        }
        output.push(current);
    }
    output.push(quote);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebases_urls_in_declarations_gradients_and_at_rules() {
        let source = r#"
            @font-face { src: url('../fonts/ui.woff2') format('woff2'); }
            @media (min-width: 1px) {
                .hero { background: linear-gradient(#fff8, #0000), URL(../img/hero\ image.png); }
            }
            .nested { background-image: cross-fade(url("./a.png"), url(/shared/b.png)); }
        "#;
        let rebased = rebase_stylesheet_urls(source, "https://example.test/assets/css/site.css");
        assert!(rebased.contains("url('https://example.test/assets/fonts/ui.woff2')"));
        assert!(rebased.contains("URL(\"https://example.test/assets/img/hero%20image.png\")"));
        assert!(rebased.contains("url(\"https://example.test/assets/css/a.png\")"));
        assert!(rebased.contains("url(\"https://example.test/shared/b.png\")"));
    }

    #[test]
    fn preserves_absolute_data_string_and_comment_urls() {
        let source = r#"
            a { background: url(data:image/svg+xml,%3Csvg%3E), url("https://cdn.test/a.png"); }
            b::before { content: "url(../not-an-asset.png)"; }
            /* url(../also-not-an-asset.png) */
        "#;
        assert_eq!(
            rebase_stylesheet_urls(source, "https://example.test/css/site.css"),
            source
        );
    }

    #[test]
    fn handles_comments_escapes_protocol_relative_and_fragments() {
        let source = r#"a { mask: URL(/* origin */ '..\2f icons/mask.svg#shape');
            filter: url(#local); cursor: url(//cdn.test/cursor.svg), auto; }"#;
        let rebased = rebase_stylesheet_urls(source, "https://example.test/css/theme/site.css?v=2");
        assert!(rebased.contains("URL('https://example.test/css/icons/mask.svg#shape')"));
        assert!(rebased.contains("url(\"https://example.test/css/theme/site.css?v=2#local\")"));
        assert!(rebased.contains("url(\"https://cdn.test/cursor.svg\")"));
    }

    #[test]
    fn rebases_an_unescaped_unicode_path_without_losing_character_boundaries() {
        let rebased = rebase_stylesheet_urls(
            ".hero { background-image: url(../图/主视觉.png); }",
            "https://example.test/assets/css/site.css",
        );
        assert_eq!(
            rebased,
            ".hero { background-image: url(\"https://example.test/assets/%E5%9B%BE/%E4%B8%BB%E8%A7%86%E8%A7%89.png\"); }"
        );
    }
}
