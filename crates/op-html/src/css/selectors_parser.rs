use super::selectors::{
    AttributeOperator, AttributeSelector, Combinator, CompoundSelector, NthExpr, PseudoClass,
    PseudoElement, RelativeSelector, Selector, MAX_SELECTOR_COMBINATORS, MAX_SELECTOR_COMPOUNDS,
    MAX_SELECTOR_LIST_ITEMS, MAX_SELECTOR_NESTING, MAX_SELECTOR_SOURCE_BYTES, MAX_SELECTOR_TOKENS,
    MAX_SIMPLE_SELECTORS_PER_COMPOUND,
};

#[derive(Clone, Copy)]
enum ListMode {
    Strict,
    Forgiving,
}

pub(super) fn parse_selector_list(input: &str) -> Vec<Selector> {
    if input.len() > MAX_SELECTOR_SOURCE_BYTES {
        return Vec::new();
    }
    let mut budget = ParseBudget::default();
    parse_list(input, 0, ListMode::Strict, true, &mut budget).unwrap_or_default()
}

struct ParseBudget {
    selectors: usize,
    tokens: usize,
}

impl Default for ParseBudget {
    fn default() -> Self {
        Self {
            selectors: MAX_SELECTOR_LIST_ITEMS,
            tokens: MAX_SELECTOR_TOKENS,
        }
    }
}

impl ParseBudget {
    fn consume_selector(&mut self) -> Option<()> {
        self.selectors = self.selectors.checked_sub(1)?;
        Some(())
    }

    fn consume_token(&mut self) -> Option<()> {
        self.tokens = self.tokens.checked_sub(1)?;
        Some(())
    }
}

fn parse_list(
    input: &str,
    depth: usize,
    mode: ListMode,
    allow_pseudo_element: bool,
    budget: &mut ParseBudget,
) -> Option<Vec<Selector>> {
    if depth > MAX_SELECTOR_NESTING {
        return None;
    }
    let mut selectors = Vec::new();
    for candidate in split_selector_list(input)? {
        budget.consume_selector()?;
        let parsed =
            Parser::new(candidate.trim(), depth, allow_pseudo_element, budget).parse_selector();
        match (mode, parsed) {
            (_, Some(selector)) => selectors.push(selector),
            (ListMode::Strict, None) => return None,
            (ListMode::Forgiving, None) => {}
        }
    }
    (!selectors.is_empty()).then_some(selectors)
}

struct Parser<'a, 'budget> {
    input: &'a str,
    index: usize,
    depth: usize,
    allow_pseudo_element: bool,
    budget: &'budget mut ParseBudget,
}

impl<'a, 'budget> Parser<'a, 'budget> {
    fn new(
        input: &'a str,
        depth: usize,
        allow_pseudo_element: bool,
        budget: &'budget mut ParseBudget,
    ) -> Self {
        Self {
            input,
            index: 0,
            depth,
            allow_pseudo_element,
            budget,
        }
    }

    fn parse_selector(mut self) -> Option<Selector> {
        self.skip_whitespace();
        let mut pseudo_element = None;
        let mut compounds = vec![self.parse_compound(&mut pseudo_element)?];
        let mut combinators = Vec::new();
        loop {
            let had_whitespace = self.skip_whitespace();
            if self.eof() {
                break;
            }
            if pseudo_element.is_some() {
                return None;
            }
            let combinator = match self.peek()? {
                '>' => {
                    self.bump();
                    Combinator::Child
                }
                '+' => {
                    self.bump();
                    Combinator::AdjacentSibling
                }
                '~' => {
                    self.bump();
                    Combinator::GeneralSibling
                }
                _ if had_whitespace => Combinator::Descendant,
                _ => return None,
            };
            if combinators.len() >= MAX_SELECTOR_COMBINATORS
                || compounds.len() >= MAX_SELECTOR_COMPOUNDS
            {
                return None;
            }
            self.budget.consume_token()?;
            self.skip_whitespace();
            compounds.push(self.parse_compound(&mut pseudo_element)?);
            combinators.push(combinator);
        }
        if pseudo_element.is_some() && !self.allow_pseudo_element {
            return None;
        }
        (combinators.len() + 1 == compounds.len()).then_some(Selector {
            compounds,
            combinators,
            pseudo_element,
        })
    }

    fn parse_compound(
        &mut self,
        pseudo_element: &mut Option<PseudoElement>,
    ) -> Option<CompoundSelector> {
        let start = self.index;
        let mut simple_selectors = 0usize;
        let mut tag = None;
        let mut universal = false;
        if self.peek() == Some('*') {
            self.consume_simple_selector(&mut simple_selectors)?;
            self.bump();
            universal = true;
        } else if self.peek().is_some_and(is_name_char) || self.peek() == Some('\\') {
            self.consume_simple_selector(&mut simple_selectors)?;
            tag = Some(self.parse_ident()?.to_ascii_lowercase());
        }
        let mut id = None;
        let mut classes = Vec::new();
        let mut attributes = Vec::new();
        let mut pseudo_classes = Vec::new();
        loop {
            match self.peek() {
                Some('#') => {
                    self.consume_simple_selector(&mut simple_selectors)?;
                    self.bump();
                    let value = self.parse_ident()?;
                    if id.replace(value).is_some() {
                        return None;
                    }
                }
                Some('.') => {
                    self.consume_simple_selector(&mut simple_selectors)?;
                    self.bump();
                    classes.push(self.parse_ident()?);
                }
                Some('[') => {
                    self.consume_simple_selector(&mut simple_selectors)?;
                    attributes.push(self.parse_attribute()?);
                }
                Some(':') => {
                    self.consume_simple_selector(&mut simple_selectors)?;
                    self.bump();
                    let double_colon = self.peek() == Some(':');
                    if double_colon {
                        self.bump();
                    }
                    let name = self.parse_ident()?.to_ascii_lowercase();
                    let functional = self.peek() == Some('(');
                    if !functional && matches!(name.as_str(), "before" | "after") {
                        *pseudo_element = Some(if name == "before" {
                            PseudoElement::Before
                        } else {
                            PseudoElement::After
                        });
                        break;
                    }
                    if double_colon {
                        return None;
                    }
                    pseudo_classes.push(if functional {
                        self.parse_functional_pseudo(name)?
                    } else {
                        parse_simple_pseudo(name)
                    });
                }
                _ => break,
            }
        }
        (self.index > start).then_some(CompoundSelector {
            tag,
            id,
            classes,
            universal,
            attributes,
            pseudo_classes,
        })
    }

    fn consume_simple_selector(&mut self, count: &mut usize) -> Option<()> {
        if *count >= MAX_SIMPLE_SELECTORS_PER_COMPOUND {
            return None;
        }
        *count += 1;
        self.budget.consume_token()
    }

    fn parse_attribute(&mut self) -> Option<AttributeSelector> {
        self.bump();
        self.skip_whitespace();
        let name = self.parse_ident()?.to_ascii_lowercase();
        self.skip_whitespace();
        if self.peek() == Some(']') {
            self.bump();
            return Some(AttributeSelector {
                name,
                operator: AttributeOperator::Exists,
                value: None,
                case_insensitive: false,
            });
        }
        let operator = match (self.peek()?, self.peek_next()) {
            ('=', _) => {
                self.bump();
                AttributeOperator::Equals
            }
            ('~', Some('=')) => AttributeOperator::Includes,
            ('|', Some('=')) => AttributeOperator::DashMatch,
            ('^', Some('=')) => AttributeOperator::Prefix,
            ('$', Some('=')) => AttributeOperator::Suffix,
            ('*', Some('=')) => AttributeOperator::Substring,
            _ => return None,
        };
        if operator != AttributeOperator::Equals {
            self.bump();
            self.bump();
        }
        self.skip_whitespace();
        let value = match self.peek()? {
            '\'' | '"' => self.parse_string()?,
            _ => self.parse_ident()?,
        };
        self.skip_whitespace();
        let mut case_insensitive = false;
        if self.peek() != Some(']') {
            let flag = self.parse_ident()?.to_ascii_lowercase();
            match flag.as_str() {
                "i" => case_insensitive = true,
                "s" => {}
                _ => return None,
            }
            self.skip_whitespace();
        }
        (self.peek() == Some(']')).then(|| self.bump())?;
        Some(AttributeSelector {
            name,
            operator,
            value: Some(value),
            case_insensitive,
        })
    }

    fn parse_functional_pseudo(&mut self, name: String) -> Option<PseudoClass> {
        self.bump();
        let contents = self.take_parenthesized()?;
        match name.as_str() {
            "not" | "is" | "where" => {
                let selectors = parse_list(
                    contents,
                    self.depth.checked_add(1)?,
                    ListMode::Forgiving,
                    false,
                    self.budget,
                )?;
                Some(match name.as_str() {
                    "not" => PseudoClass::Not(selectors),
                    "is" => PseudoClass::Is(selectors),
                    _ => PseudoClass::Where(selectors),
                })
            }
            "has" => Some(PseudoClass::Has(parse_relative_list(
                contents,
                self.depth.checked_add(1)?,
                self.budget,
            )?)),
            "lang" => Some(PseudoClass::Lang(parse_lang_list(
                contents,
                self.depth,
                self.budget,
            )?)),
            "nth-child" | "nth-last-child" => {
                let (formula, of_selectors) = split_nth_of(contents)?;
                let expression = parse_nth(formula)?;
                let selectors = match of_selectors {
                    Some(input) => Some(parse_list(
                        input,
                        self.depth.checked_add(1)?,
                        ListMode::Strict,
                        false,
                        self.budget,
                    )?),
                    None => None,
                };
                Some(match (name.as_str(), selectors) {
                    ("nth-child", Some(selectors)) => {
                        PseudoClass::NthChildOf(expression, selectors)
                    }
                    ("nth-last-child", Some(selectors)) => {
                        PseudoClass::NthLastChildOf(expression, selectors)
                    }
                    ("nth-child", None) => PseudoClass::NthChild(expression),
                    _ => PseudoClass::NthLastChild(expression),
                })
            }
            "nth-of-type" => Some(PseudoClass::NthOfType(parse_nth(contents)?)),
            "nth-last-of-type" => Some(PseudoClass::NthLastOfType(parse_nth(contents)?)),
            _ => Some(PseudoClass::Unsupported(name)),
        }
    }

    fn parse_ident(&mut self) -> Option<String> {
        let mut output = String::new();
        while let Some(ch) = self.peek() {
            if ch == '\\' {
                output.push(self.parse_escape()?);
            } else if is_name_char(ch) {
                output.push(ch);
                self.bump();
            } else {
                break;
            }
        }
        (!output.is_empty()).then_some(output)
    }

    fn parse_string(&mut self) -> Option<String> {
        let quote = self.bump()?;
        let mut output = String::new();
        loop {
            let ch = self.peek()?;
            if ch == quote {
                self.bump();
                return Some(output);
            }
            if ch == '\\' {
                output.push(self.parse_escape()?);
            } else if matches!(ch, '\n' | '\r' | '\u{c}') {
                return None;
            } else {
                output.push(ch);
                self.bump();
            }
        }
    }

    fn parse_escape(&mut self) -> Option<char> {
        (self.bump() == Some('\\')).then_some(())?;
        let first = self.peek()?;
        if first.is_ascii_hexdigit() {
            let start = self.index;
            for _ in 0..6 {
                if !self.peek().is_some_and(|ch| ch.is_ascii_hexdigit()) {
                    break;
                }
                self.bump();
            }
            let value = u32::from_str_radix(&self.input[start..self.index], 16).ok()?;
            if self.peek().is_some_and(char::is_whitespace) {
                self.bump();
            }
            Some(
                char::from_u32(value)
                    .filter(|ch| *ch != '\0')
                    .unwrap_or('\u{fffd}'),
            )
        } else if matches!(first, '\n' | '\r' | '\u{c}') {
            None
        } else {
            self.bump()
        }
    }

    fn take_parenthesized(&mut self) -> Option<&'a str> {
        let start = self.index;
        let mut depth = 1u32;
        let mut quote = None;
        let mut escaped = false;
        for (offset, ch) in self.input[start..].char_indices() {
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
                '(' => depth = depth.checked_add(1)?,
                ')' => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        let end = start + offset;
                        self.index = end + 1;
                        return Some(&self.input[start..end]);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn skip_whitespace(&mut self) -> bool {
        let start = self.index;
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
        self.index != start
    }

    fn peek(&self) -> Option<char> {
        self.input[self.index..].chars().next()
    }

    fn peek_next(&self) -> Option<char> {
        let mut chars = self.input[self.index..].chars();
        chars.next()?;
        chars.next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.index += ch.len_utf8();
        Some(ch)
    }

    fn eof(&self) -> bool {
        self.index == self.input.len()
    }
}

fn parse_simple_pseudo(name: String) -> PseudoClass {
    match name.as_str() {
        "root" => PseudoClass::Root,
        "first-child" => PseudoClass::FirstChild,
        "last-child" => PseudoClass::LastChild,
        "only-child" => PseudoClass::OnlyChild,
        "first-of-type" => PseudoClass::FirstOfType,
        "last-of-type" => PseudoClass::LastOfType,
        "only-of-type" => PseudoClass::OnlyOfType,
        "empty" => PseudoClass::Empty,
        "checked" | "selected" => PseudoClass::Checked,
        "disabled" => PseudoClass::Disabled,
        "enabled" => PseudoClass::Enabled,
        "required" => PseudoClass::Required,
        "optional" => PseudoClass::Optional,
        "read-only" => PseudoClass::ReadOnly,
        "read-write" => PseudoClass::ReadWrite,
        "placeholder-shown" => PseudoClass::PlaceholderShown,
        "default" => PseudoClass::Default,
        _ => PseudoClass::Unsupported(name),
    }
}

fn parse_relative_list(
    input: &str,
    depth: usize,
    budget: &mut ParseBudget,
) -> Option<Vec<RelativeSelector>> {
    if depth > MAX_SELECTOR_NESTING {
        return None;
    }
    let mut selectors = Vec::new();
    for candidate in split_selector_list(input)? {
        budget.consume_selector()?;
        let mut candidate = candidate.trim();
        let leading_combinator = match candidate.chars().next() {
            Some('>') => Combinator::Child,
            Some('+') => Combinator::AdjacentSibling,
            Some('~') => Combinator::GeneralSibling,
            _ => Combinator::Descendant,
        };
        if leading_combinator != Combinator::Descendant {
            candidate = candidate[1..].trim_start();
        }
        if let Some(selector) = Parser::new(candidate, depth, false, budget).parse_selector() {
            selectors.push(RelativeSelector {
                leading_combinator,
                selector,
            });
        }
    }
    (!selectors.is_empty()).then_some(selectors)
}

fn parse_lang_list(input: &str, depth: usize, budget: &mut ParseBudget) -> Option<Vec<String>> {
    let mut languages = Vec::new();
    for candidate in split_selector_list(input)? {
        budget.consume_token()?;
        let mut parser = Parser::new(candidate.trim(), depth, false, budget);
        let language = match parser.peek()? {
            '\'' | '"' => parser.parse_string()?,
            '*' => {
                parser.bump();
                "*".to_string()
            }
            _ => parser.parse_ident()?,
        };
        parser.skip_whitespace();
        if !parser.eof() || language.is_empty() {
            return None;
        }
        languages.push(language);
    }
    (!languages.is_empty()).then_some(languages)
}

fn is_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || !ch.is_ascii()
}

fn split_selector_list(input: &str) -> Option<Vec<&str>> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut parentheses = 0u32;
    let mut brackets = 0u32;
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
            '(' => parentheses = parentheses.saturating_add(1),
            ')' => parentheses = parentheses.saturating_sub(1),
            '[' => brackets = brackets.saturating_add(1),
            ']' => brackets = brackets.saturating_sub(1),
            ',' if parentheses == 0 && brackets == 0 => {
                if parts.len() >= MAX_SELECTOR_LIST_ITEMS {
                    return None;
                }
                parts.push(&input[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if parts.len() >= MAX_SELECTOR_LIST_ITEMS {
        return None;
    }
    parts.push(&input[start..]);
    Some(parts)
}

fn split_nth_of(input: &str) -> Option<(&str, Option<&str>)> {
    let mut parentheses = 0u32;
    let mut brackets = 0u32;
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
            '(' => parentheses = parentheses.saturating_add(1),
            ')' => parentheses = parentheses.checked_sub(1)?,
            '[' => brackets = brackets.saturating_add(1),
            ']' => brackets = brackets.checked_sub(1)?,
            _ => {}
        }
        if parentheses == 0
            && brackets == 0
            && (ch == 'o' || ch == 'O')
            && input
                .get(index..index + 2)
                .is_some_and(|word| word.eq_ignore_ascii_case("of"))
            && input[..index]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
            && input[index + 2..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
        {
            let formula = input[..index].trim();
            let selectors = input[index + 2..].trim();
            return (!formula.is_empty() && !selectors.is_empty())
                .then_some((formula, Some(selectors)));
        }
    }
    Some((input.trim(), None))
}

fn parse_nth(input: &str) -> Option<NthExpr> {
    let compact: String = input.chars().filter(|ch| !ch.is_whitespace()).collect();
    let compact = compact.to_ascii_lowercase();
    match compact.as_str() {
        "odd" => return Some(NthExpr { a: 2, b: 1 }),
        "even" => return Some(NthExpr { a: 2, b: 0 }),
        _ => {}
    }
    if let Some(n) = compact.find('n') {
        let a = match &compact[..n] {
            "" | "+" => 1,
            "-" => -1,
            value => value.parse().ok()?,
        };
        let b = match &compact[n + 1..] {
            "" => 0,
            value => value.parse().ok()?,
        };
        Some(NthExpr { a, b })
    } else {
        Some(NthExpr {
            a: 0,
            b: compact.parse().ok()?,
        })
    }
}

#[cfg(test)]
mod complexity_tests {
    use super::*;

    #[test]
    fn enforces_selector_shape_and_global_token_limits() {
        let max_chain = (0..MAX_SELECTOR_COMPOUNDS)
            .map(|_| "div")
            .collect::<Vec<_>>()
            .join(" > ");
        assert_eq!(
            parse_selector_list(&max_chain)[0].compounds.len(),
            MAX_SELECTOR_COMPOUNDS
        );
        assert!(parse_selector_list(&format!("{max_chain} > div")).is_empty());

        let max_simple = format!("div{}", ".x".repeat(MAX_SIMPLE_SELECTORS_PER_COMPOUND - 1));
        assert!(parse_selector_list(&max_simple).len() == 1);
        assert!(parse_selector_list(&format!("{max_simple}.x")).is_empty());

        let dense = || format!("div{}", ".x".repeat(MAX_SIMPLE_SELECTORS_PER_COMPOUND - 1));
        let within_budget = (0..7).map(|_| dense()).collect::<Vec<_>>().join(" > ");
        let over_budget = (0..8).map(|_| dense()).collect::<Vec<_>>().join(" > ");
        assert!(parse_selector_list(&within_budget).len() == 1);
        assert!(parse_selector_list(&over_budget).is_empty());
    }

    #[test]
    fn bounds_selector_lists_and_source_size() {
        let maximum = vec![".x"; MAX_SELECTOR_LIST_ITEMS].join(",");
        let excessive = vec![".x"; MAX_SELECTOR_LIST_ITEMS + 1].join(",");
        assert_eq!(parse_selector_list(&maximum).len(), MAX_SELECTOR_LIST_ITEMS);
        assert!(parse_selector_list(&excessive).is_empty());
        assert!(parse_selector_list(&"x".repeat(MAX_SELECTOR_SOURCE_BYTES + 1)).is_empty());
    }
}
