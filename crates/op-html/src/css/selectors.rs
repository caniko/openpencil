use crate::dom::DomElement;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompoundSelector {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selector {
    pub compounds: Vec<CompoundSelector>,
}

pub fn parse_selector_list(input: &str) -> Vec<Selector> {
    input
        .split(',')
        .filter_map(|candidate| {
            let candidate = candidate.trim();
            if candidate.is_empty()
                || candidate
                    .chars()
                    .any(|ch| matches!(ch, '>' | '+' | '~' | '[' | ':' | '*'))
            {
                return None;
            }
            let compounds: Option<Vec<_>> =
                candidate.split_whitespace().map(parse_compound).collect();
            let compounds = compounds?;
            (!compounds.is_empty()).then_some(Selector { compounds })
        })
        .collect()
}

fn parse_compound(input: &str) -> Option<CompoundSelector> {
    let bytes = input.as_bytes();
    let mut index = 0;
    let mut tag = None;
    let mut id = None;
    let mut classes = Vec::new();
    if !matches!(bytes.first(), Some(b'.' | b'#')) {
        let end = input.find(['.', '#']).unwrap_or(input.len());
        if end == 0 {
            return None;
        }
        tag = Some(input[..end].to_ascii_lowercase());
        index = end;
    }
    while index < input.len() {
        let marker = bytes[index];
        if marker != b'.' && marker != b'#' {
            return None;
        }
        let start = index + 1;
        let relative_end = input[start..]
            .find(['.', '#'])
            .unwrap_or(input.len() - start);
        let end = start + relative_end;
        if start == end {
            return None;
        }
        let token = input[start..end].to_string();
        if marker == b'#' {
            if id.replace(token).is_some() {
                return None;
            }
        } else {
            classes.push(token);
        }
        index = end;
    }
    Some(CompoundSelector { tag, id, classes })
}

pub fn specificity(selector: &Selector) -> (u32, u32, u32) {
    selector
        .compounds
        .iter()
        .fold((0, 0, 0), |(ids, classes, tags), compound| {
            (
                ids + u32::from(compound.id.is_some()),
                classes + compound.classes.len() as u32,
                tags + u32::from(compound.tag.is_some()),
            )
        })
}

pub fn matches(selector: &Selector, path: &[&DomElement]) -> bool {
    let Some((target, ancestors)) = path.split_last() else {
        return false;
    };
    let Some((target_selector, earlier_selectors)) = selector.compounds.split_last() else {
        return false;
    };
    if !compound_matches(target_selector, target) {
        return false;
    }
    let mut ancestor_end = ancestors.len();
    for compound in earlier_selectors.iter().rev() {
        let Some(index) = ancestors[..ancestor_end]
            .iter()
            .rposition(|element| compound_matches(compound, element))
        else {
            return false;
        };
        ancestor_end = index;
    }
    true
}

fn compound_matches(selector: &CompoundSelector, element: &DomElement) -> bool {
    if selector
        .tag
        .as_deref()
        .is_some_and(|tag| tag != element.tag)
    {
        return false;
    }
    if selector
        .id
        .as_deref()
        .is_some_and(|id| Some(id) != element.id())
    {
        return false;
    }
    let element_classes = element.classes();
    selector
        .classes
        .iter()
        .all(|class| element_classes.contains(&class.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::DomElement;

    fn el(tag: &str, class: &str, id: &str) -> DomElement {
        let mut attrs = Vec::new();
        if !class.is_empty() {
            attrs.push(("class".into(), class.into()));
        }
        if !id.is_empty() {
            attrs.push(("id".into(), id.into()));
        }
        DomElement {
            tag: tag.into(),
            attrs,
            children: Vec::new(),
        }
    }

    #[test]
    fn parses_and_scores() {
        let selectors = parse_selector_list("div.card, #hero .title, p");
        assert_eq!(selectors.len(), 3);
        assert_eq!(specificity(&selectors[0]), (0, 1, 1));
        assert_eq!(specificity(&selectors[1]), (1, 1, 0));
        assert_eq!(specificity(&selectors[2]), (0, 0, 1));
    }

    #[test]
    fn unsupported_selectors_are_dropped() {
        let selectors = parse_selector_list("a:hover, div > p, .ok");
        assert_eq!(selectors.len(), 1);
        assert_eq!(selectors[0].compounds[0].classes, vec!["ok".to_string()]);
    }

    #[test]
    fn descendant_matching() {
        let hero = el("section", "", "hero");
        let mid = el("div", "card", "");
        let title = el("h2", "title", "");
        let path: Vec<&DomElement> = vec![&hero, &mid, &title];
        let selector = &parse_selector_list("#hero .title")[0];
        assert!(matches(selector, &path));
        let selector = &parse_selector_list("#hero .card .title")[0];
        assert!(matches(selector, &path));
        let selector = &parse_selector_list("#other .title")[0];
        assert!(!matches(selector, &path));
    }
}
