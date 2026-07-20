use crate::dom::{DomElement, DomNode};

pub(super) const MAX_SELECTOR_SOURCE_BYTES: usize = 64 * 1024;
pub(super) const MAX_SELECTOR_LIST_ITEMS: usize = 256;
pub(super) const MAX_SELECTOR_TOKENS: usize = 512;
pub(super) const MAX_SELECTOR_COMPOUNDS: usize = 64;
pub(super) const MAX_SELECTOR_COMBINATORS: usize = MAX_SELECTOR_COMPOUNDS - 1;
pub(super) const MAX_SIMPLE_SELECTORS_PER_COMPOUND: usize = 64;
pub(super) const MAX_SELECTOR_NESTING: usize = 32;

#[rustfmt::skip] #[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combinator { Descendant, Child, AdjacentSibling, GeneralSibling }

#[rustfmt::skip]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PseudoElement { Before, After }

#[rustfmt::skip]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeOperator { Exists, Equals, Includes, DashMatch, Prefix, Suffix, Substring }

#[rustfmt::skip]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributeSelector {
    pub name: String, pub operator: AttributeOperator, pub value: Option<String>,
    pub case_insensitive: bool,
}

#[rustfmt::skip]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NthExpr { pub a: i32, pub b: i32 }

#[rustfmt::skip]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelativeSelector { pub leading_combinator: Combinator, pub selector: Selector }

#[rustfmt::skip]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PseudoClass {
    Root, FirstChild, LastChild, OnlyChild, Empty,
    FirstOfType, LastOfType, OnlyOfType,
    NthChild(NthExpr), NthLastChild(NthExpr),
    NthChildOf(NthExpr, Vec<Selector>), NthLastChildOf(NthExpr, Vec<Selector>),
    NthOfType(NthExpr), NthLastOfType(NthExpr),
    Not(Vec<Selector>), Is(Vec<Selector>), Where(Vec<Selector>), Has(Vec<RelativeSelector>),
    Lang(Vec<String>),
    Checked, Disabled, Enabled, Required, Optional,
    ReadOnly, ReadWrite, PlaceholderShown, Default,
    Unsupported(String),
}

#[rustfmt::skip]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompoundSelector {
    pub tag: Option<String>, pub id: Option<String>, pub classes: Vec<String>,
    pub universal: bool, pub attributes: Vec<AttributeSelector>,
    pub pseudo_classes: Vec<PseudoClass>,
}

#[rustfmt::skip]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selector {
    pub compounds: Vec<CompoundSelector>, // Combinator i joins compounds i and i + 1.
    pub combinators: Vec<Combinator>,
    pub pseudo_element: Option<PseudoElement>,
}

pub fn parse_selector_list(input: &str) -> Vec<Selector> {
    super::selectors_parser::parse_selector_list(input)
}

pub fn specificity(selector: &Selector) -> (u32, u32, u32) {
    let mut score = (0u32, 0u32, 0u32);
    for compound in &selector.compounds {
        score.0 = score.0.saturating_add(u32::from(compound.id.is_some()));
        score.1 = score
            .1
            .saturating_add(compound.classes.len() as u32 + compound.attributes.len() as u32);
        score.2 = score.2.saturating_add(u32::from(compound.tag.is_some()));
        for pseudo in &compound.pseudo_classes {
            match pseudo {
                PseudoClass::Where(_) => {}
                PseudoClass::Not(selectors) | PseudoClass::Is(selectors) => {
                    add_score(
                        &mut score,
                        selectors.iter().map(specificity).max().unwrap_or_default(),
                    );
                }
                PseudoClass::Has(selectors) => add_score(
                    &mut score,
                    selectors
                        .iter()
                        .map(|relative| specificity(&relative.selector))
                        .max()
                        .unwrap_or_default(),
                ),
                PseudoClass::NthChildOf(_, selectors)
                | PseudoClass::NthLastChildOf(_, selectors) => {
                    score.1 = score.1.saturating_add(1);
                    add_score(
                        &mut score,
                        selectors.iter().map(specificity).max().unwrap_or_default(),
                    );
                }
                _ => score.1 = score.1.saturating_add(1),
            }
        }
    }
    score.2 = score
        .2
        .saturating_add(u32::from(selector.pseudo_element.is_some()));
    score
}

fn add_score(score: &mut (u32, u32, u32), addition: (u32, u32, u32)) {
    score.0 = score.0.saturating_add(addition.0);
    score.1 = score.1.saturating_add(addition.1);
    score.2 = score.2.saturating_add(addition.2);
}

#[rustfmt::skip]
pub fn matches(selector: &Selector, path: &[&DomElement]) -> bool { selector.pseudo_element.is_none() && matches_selector(selector, path, 0) }

#[rustfmt::skip]
pub fn matches_for_pseudo(selector: &Selector, path: &[&DomElement], pseudo_element: PseudoElement) -> bool {
    selector.pseudo_element == Some(pseudo_element) && matches_selector(selector, path, 0)
}

#[rustfmt::skip]
#[derive(Clone, Copy)] struct MatchPath<'path, 'dom> { ancestors: &'path [&'dom DomElement], element: &'dom DomElement }

#[rustfmt::skip]
impl<'path, 'dom> MatchPath<'path, 'dom> {
    fn from_slice(path: &'path [&'dom DomElement]) -> Option<Self> {
        let (element, ancestors) = path.split_last()?;
        Some(Self { ancestors, element })
    }
    fn parent(self) -> Option<Self> {
        let (element, ancestors) = self.ancestors.split_last()?;
        Some(Self { ancestors, element })
    }
    fn with_element(self, element: &'dom DomElement) -> Self {
        Self { ancestors: self.ancestors, element }
    }
    fn to_vec(self) -> Vec<&'dom DomElement> {
        let mut path = Vec::with_capacity(self.ancestors.len() + 1);
        path.extend_from_slice(self.ancestors); path.push(self.element);
        path
    }
}

fn matches_selector(selector: &Selector, path: &[&DomElement], depth: usize) -> bool {
    MatchPath::from_slice(path).is_some_and(|path| matches_selector_path(selector, path, depth))
}

fn matches_selector_path(selector: &Selector, path: MatchPath<'_, '_>, depth: usize) -> bool {
    depth <= MAX_SELECTOR_NESTING
        && selector_shape_is_valid(selector)
        && matches_at(selector, selector.compounds.len() - 1, path, depth)
}

#[rustfmt::skip]
fn selector_shape_is_valid(selector: &Selector) -> bool {
    !selector.compounds.is_empty()
        && selector.compounds.len() <= MAX_SELECTOR_COMPOUNDS
        && selector.combinators.len() <= MAX_SELECTOR_COMBINATORS
        && selector.combinators.len() + 1 == selector.compounds.len()
        && selector.compounds.iter().all(|compound| simple_selector_count(compound) <= MAX_SIMPLE_SELECTORS_PER_COMPOUND)
}

#[rustfmt::skip]
fn simple_selector_count(compound: &CompoundSelector) -> usize { usize::from(compound.tag.is_some() || compound.universal) + usize::from(compound.id.is_some()) + compound.classes.len() + compound.attributes.len() + compound.pseudo_classes.len() }

fn matches_at(selector: &Selector, index: usize, path: MatchPath<'_, '_>, depth: usize) -> bool {
    if !compound_matches(&selector.compounds[index], path, depth) {
        return false;
    }
    if index == 0 {
        return true;
    }
    match selector.combinators[index - 1] {
        Combinator::Descendant => (0..path.ancestors.len()).rev().any(|at| {
            let ancestor_path = MatchPath {
                ancestors: &path.ancestors[..at],
                element: path.ancestors[at],
            };
            matches_at(selector, index - 1, ancestor_path, depth)
        }),
        Combinator::Child => path
            .parent()
            .is_some_and(|parent| matches_at(selector, index - 1, parent, depth)),
        Combinator::AdjacentSibling => sibling_context(path).is_some_and(|context| {
            context.preceding_elements().next().is_some_and(|previous| {
                matches_at(selector, index - 1, path.with_element(previous), depth)
            })
        }),
        Combinator::GeneralSibling => sibling_context(path).is_some_and(|context| {
            context
                .preceding_elements()
                .any(|previous| matches_at(selector, index - 1, path.with_element(previous), depth))
        }),
    }
}

fn compound_matches(selector: &CompoundSelector, path: MatchPath<'_, '_>, depth: usize) -> bool {
    let element = path.element;
    if selector
        .tag
        .as_deref()
        .is_some_and(|tag| tag != element.tag)
        || selector
            .id
            .as_deref()
            .is_some_and(|id| Some(id) != element.id())
    {
        return false;
    }
    let classes = element.classes();
    selector
        .classes
        .iter()
        .all(|class| classes.contains(&class.as_str()))
        && selector
            .attributes
            .iter()
            .all(|attribute| attribute_matches(attribute, element))
        && selector
            .pseudo_classes
            .iter()
            .all(|pseudo| pseudo_matches(pseudo, path, depth))
}

fn attribute_matches(selector: &AttributeSelector, element: &DomElement) -> bool {
    let Some(actual) = element.attr(&selector.name) else {
        return false;
    };
    if selector.operator == AttributeOperator::Exists {
        return true;
    }
    let expected = selector.value.as_deref().unwrap_or_default();
    let (actual, expected) = if selector.case_insensitive {
        (actual.to_ascii_lowercase(), expected.to_ascii_lowercase())
    } else {
        (actual.to_string(), expected.to_string())
    };
    match selector.operator {
        AttributeOperator::Exists => true,
        AttributeOperator::Equals => actual == expected,
        AttributeOperator::Includes => {
            !expected.is_empty() && actual.split_whitespace().any(|part| part == expected)
        }
        AttributeOperator::DashMatch => {
            actual == expected
                || (!expected.is_empty() && actual.starts_with(&format!("{expected}-")))
        }
        AttributeOperator::Prefix => !expected.is_empty() && actual.starts_with(&expected),
        AttributeOperator::Suffix => !expected.is_empty() && actual.ends_with(&expected),
        AttributeOperator::Substring => !expected.is_empty() && actual.contains(&expected),
    }
}

#[rustfmt::skip]
fn pseudo_matches(pseudo: &PseudoClass, path: MatchPath<'_, '_>, depth: usize) -> bool {
    let element = path.element;
    match pseudo {
        PseudoClass::Root => path.ancestors.is_empty(),
        PseudoClass::Empty => element.children.is_empty(),
        PseudoClass::FirstChild => sibling_position(path, false).is_some_and(|(at, _)| at == 1),
        PseudoClass::LastChild => sibling_position(path, false).is_some_and(|(at, total)| at == total),
        PseudoClass::OnlyChild => sibling_position(path, false).is_some_and(|(_, total)| total == 1),
        PseudoClass::FirstOfType => sibling_position(path, true).is_some_and(|(at, _)| at == 1),
        PseudoClass::LastOfType => sibling_position(path, true).is_some_and(|(at, total)| at == total),
        PseudoClass::OnlyOfType => sibling_position(path, true).is_some_and(|(_, total)| total == 1),
        PseudoClass::NthChild(expr) => sibling_position(path, false).is_some_and(|(at, _)| nth_matches(*expr, at)),
        PseudoClass::NthLastChild(expr) => sibling_position(path, false).is_some_and(|(at, total)| nth_matches(*expr, total - at + 1)),
        PseudoClass::NthChildOf(expr, selectors) => sibling_position_of(path, selectors, depth).is_some_and(|(at, _)| nth_matches(*expr, at)),
        PseudoClass::NthLastChildOf(expr, selectors) => sibling_position_of(path, selectors, depth).is_some_and(|(at, total)| nth_matches(*expr, total - at + 1)),
        PseudoClass::NthOfType(expr) => sibling_position(path, true).is_some_and(|(at, _)| nth_matches(*expr, at)),
        PseudoClass::NthLastOfType(expr) => sibling_position(path, true).is_some_and(|(at, total)| nth_matches(*expr, total - at + 1)),
        PseudoClass::Not(selectors) => selector_list_matches(selectors, path, depth).is_some_and(|matched| !matched),
        PseudoClass::Is(selectors) | PseudoClass::Where(selectors) => selector_list_matches(selectors, path, depth).unwrap_or(false),
        PseudoClass::Has(selectors) => relative_list_matches(selectors, path, depth).unwrap_or(false),
        PseudoClass::Lang(ranges) => language_matches(path, ranges),
        PseudoClass::Checked => is_checked(element),
        PseudoClass::Disabled => is_disableable(element) && has_attr(element, "disabled"),
        PseudoClass::Enabled => is_disableable(element) && !has_attr(element, "disabled"),
        PseudoClass::Required => accepts_required(element) && has_attr(element, "required"),
        PseudoClass::Optional => accepts_required(element) && !has_attr(element, "required"),
        PseudoClass::ReadOnly => is_text_control(element) && has_attr(element, "readonly"),
        PseudoClass::ReadWrite => is_text_control(element) && !has_attr(element, "readonly") && !has_attr(element, "disabled"),
        PseudoClass::PlaceholderShown => placeholder_shown(element),
        PseudoClass::Default => is_checked(element) || (element.tag == "button" && button_type(element) == "submit"),
        PseudoClass::Unsupported(_) => false,
    }
}

#[rustfmt::skip]
fn selector_list_matches(selectors: &[Selector], path: MatchPath<'_, '_>, depth: usize) -> Option<bool> {
    (selectors.len() <= MAX_SELECTOR_LIST_ITEMS).then(|| {
        selectors.iter().any(|selector| matches_selector_path(selector, path, depth.saturating_add(1)))
    })
}

#[rustfmt::skip]
fn relative_list_matches(selectors: &[RelativeSelector], path: MatchPath<'_, '_>, depth: usize) -> Option<bool> {
    (selectors.len() <= MAX_SELECTOR_LIST_ITEMS).then(|| {
        selectors.iter().any(|selector| relative_matches(selector, path, depth.saturating_add(1)))
    })
}

struct SiblingContext<'a> {
    parent: &'a DomElement,
    target_node_index: usize,
}

#[rustfmt::skip]
impl<'a> SiblingContext<'a> {
    fn preceding_elements(&self) -> impl Iterator<Item = &'a DomElement> + '_ {
        self.parent.children[..self.target_node_index].iter().rev().filter_map(element_ref)
    }
    fn all_elements(&self) -> impl Iterator<Item = &'a DomElement> + '_ {
        self.parent.children.iter().filter_map(element_ref)
    }
    fn following_elements(&self) -> impl Iterator<Item = &'a DomElement> + '_ {
        self.parent.children[self.target_node_index + 1..].iter().filter_map(element_ref)
    }
}

#[rustfmt::skip]
fn element_ref(node: &DomNode) -> Option<&DomElement> { match node { DomNode::Element(element) => Some(element), DomNode::Text(_) => None } }

fn sibling_context<'a>(path: MatchPath<'_, 'a>) -> Option<SiblingContext<'a>> {
    let parent = *path.ancestors.last()?;
    let target_node_index = parent.children.iter().position(
        |node| matches!(node, DomNode::Element(element) if std::ptr::eq(element, path.element)),
    )?;
    Some(SiblingContext {
        parent,
        target_node_index,
    })
}

#[rustfmt::skip]
fn sibling_position(path: MatchPath<'_, '_>, of_type: bool) -> Option<(i32, i32)> {
    let context = sibling_context(path)?;
    let mut position = None;
    let mut total = 0usize;
    for element in context.all_elements().filter(|element| !of_type || element.tag == path.element.tag) {
        total = total.checked_add(1)?;
        if std::ptr::eq(element, path.element) { position = Some(total); }
    }
    Some((i32::try_from(position?).ok()?, i32::try_from(total).ok()?))
}

fn sibling_position_of(
    path: MatchPath<'_, '_>,
    selectors: &[Selector],
    depth: usize,
) -> Option<(i32, i32)> {
    if selectors.len() > MAX_SELECTOR_LIST_ITEMS {
        return None;
    }
    let context = sibling_context(path)?;
    let mut position = None;
    let mut total = 0usize;
    for sibling in context.all_elements() {
        let sibling_path = path.with_element(sibling);
        if selectors
            .iter()
            .any(|selector| matches_selector_path(selector, sibling_path, depth.saturating_add(1)))
        {
            total = total.checked_add(1)?;
            if std::ptr::eq(sibling, path.element) {
                position = Some(total);
            }
        }
    }
    Some((i32::try_from(position?).ok()?, i32::try_from(total).ok()?))
}

fn relative_matches(
    selector: &RelativeSelector,
    anchor_path: MatchPath<'_, '_>,
    depth: usize,
) -> bool {
    if depth > MAX_SELECTOR_NESTING || !selector_shape_is_valid(&selector.selector) {
        return false;
    }
    let anchor_path = anchor_path.to_vec();
    related_paths(&anchor_path, selector.leading_combinator)
        .iter()
        .any(|path| relative_matches_at(&selector.selector, 0, path, depth))
}

fn relative_matches_at(
    selector: &Selector,
    index: usize,
    path: &[&DomElement],
    depth: usize,
) -> bool {
    let Some(match_path) = MatchPath::from_slice(path) else {
        return false;
    };
    if !compound_matches(&selector.compounds[index], match_path, depth) {
        return false;
    }
    if index + 1 == selector.compounds.len() {
        return true;
    }
    related_paths(path, selector.combinators[index])
        .iter()
        .any(|next_path| relative_matches_at(selector, index + 1, next_path, depth))
}

fn related_paths<'a>(path: &[&'a DomElement], combinator: Combinator) -> Vec<Vec<&'a DomElement>> {
    match combinator {
        Combinator::Descendant => {
            let mut descendants = Vec::new();
            collect_descendant_paths(path, &mut descendants);
            descendants
        }
        Combinator::Child => element_children(path),
        Combinator::AdjacentSibling | Combinator::GeneralSibling => {
            following_sibling_paths(path, combinator == Combinator::AdjacentSibling)
        }
    }
}

fn element_children<'a>(path: &[&'a DomElement]) -> Vec<Vec<&'a DomElement>> {
    let Some(parent) = path.last().copied() else {
        return Vec::new();
    };
    parent
        .children
        .iter()
        .filter_map(|child| match child {
            DomNode::Element(element) => {
                let mut child_path = path.to_vec();
                child_path.push(element);
                Some(child_path)
            }
            DomNode::Text(_) => None,
        })
        .collect()
}

fn collect_descendant_paths<'a>(path: &[&'a DomElement], output: &mut Vec<Vec<&'a DomElement>>) {
    for child_path in element_children(path) {
        output.push(child_path.clone());
        collect_descendant_paths(&child_path, output);
    }
}

fn following_sibling_paths<'a>(
    path: &[&'a DomElement],
    adjacent_only: bool,
) -> Vec<Vec<&'a DomElement>> {
    let Some(match_path) = MatchPath::from_slice(path) else {
        return Vec::new();
    };
    let Some(context) = sibling_context(match_path) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    for sibling in context.following_elements() {
        let mut sibling_path = path[..path.len() - 1].to_vec();
        sibling_path.push(sibling);
        paths.push(sibling_path);
        if adjacent_only {
            break;
        }
    }
    paths
}

fn language_matches(path: MatchPath<'_, '_>, ranges: &[String]) -> bool {
    let Some(language) = std::iter::once(path.element)
        .chain(path.ancestors.iter().rev().copied())
        .find_map(|element| element.attr("lang").or_else(|| element.attr("xml:lang")))
    else {
        return false;
    };
    !language.is_empty()
        && ranges
            .iter()
            .any(|range| language_range_matches(language, range))
}

fn language_range_matches(language: &str, range: &str) -> bool {
    let language: Vec<_> = language.split('-').collect();
    let range: Vec<_> = range.split('-').collect();
    let mut positions = vec![false; language.len() + 1];
    positions[0] = true;
    for range_part in range {
        let mut next = vec![false; language.len() + 1];
        if range_part == "*" {
            if let Some(first) = positions.iter().position(|possible| *possible) {
                next[first..].fill(true);
            }
        } else {
            for (at, language_part) in language.iter().enumerate() {
                if positions[at] && language_part.eq_ignore_ascii_case(range_part) {
                    next[at + 1] = true;
                }
            }
        }
        positions = next;
    }
    positions.into_iter().any(|matched| matched)
}

fn nth_matches(expr: NthExpr, position: i32) -> bool {
    let difference = i64::from(position) - i64::from(expr.b);
    let step = i64::from(expr.a);
    if step == 0 {
        difference == 0
    } else {
        difference % step == 0 && difference / step >= 0
    }
}

#[rustfmt::skip]
fn has_attr(element: &DomElement, name: &str) -> bool { element.attr(name).is_some() }

fn is_checked(element: &DomElement) -> bool {
    (element.tag == "input"
        && matches!(element.attr("type"), Some(kind) if kind.eq_ignore_ascii_case("checkbox") || kind.eq_ignore_ascii_case("radio"))
        && has_attr(element, "checked"))
        || (element.tag == "option" && has_attr(element, "selected"))
}

fn is_disableable(element: &DomElement) -> bool {
    matches!(
        element.tag.as_str(),
        "button" | "fieldset" | "input" | "optgroup" | "option" | "select" | "textarea"
    )
}

fn accepts_required(element: &DomElement) -> bool {
    matches!(element.tag.as_str(), "input" | "select" | "textarea")
        && !matches!(element.attr("type"), Some(kind) if matches!(kind.to_ascii_lowercase().as_str(), "hidden" | "button" | "submit" | "reset" | "image"))
}

fn is_text_control(element: &DomElement) -> bool {
    element.tag == "textarea"
        || (element.tag == "input"
            && !matches!(element.attr("type"), Some(kind) if matches!(kind.to_ascii_lowercase().as_str(), "checkbox" | "radio" | "range" | "color" | "file" | "hidden" | "button" | "submit" | "reset" | "image")))
}

fn placeholder_shown(element: &DomElement) -> bool {
    is_text_control(element)
        && element.attr("placeholder").is_some()
        && element.attr("value").unwrap_or_default().is_empty()
        && (element.tag != "textarea"
            || element
                .children
                .iter()
                .all(|child| !matches!(child, DomNode::Text(text) if !text.is_empty())))
}

fn button_type(element: &DomElement) -> &str {
    element.attr("type").unwrap_or("submit")
}

#[rustfmt::skip]
#[cfg(test)]
mod tests {
    use super::*;

    fn el(tag: &str, attrs: &[(&str, &str)], children: Vec<DomNode>) -> DomElement {
        DomElement {
            tag: tag.into(),
            attrs: attrs.iter().map(|(name, value)| ((*name).into(), (*value).into())).collect(),
            children,
        }
    }
    fn selected(input: &str) -> Selector { parse_selector_list(input).into_iter().next().unwrap() }

    #[test]
    fn parses_escapes_attributes_and_specificity() {
        let selector = selected(r#"main#hero.card\.wide[data-kind~="News" i][lang|=en]"#);
        assert_eq!(selector.compounds[0].classes, ["card.wide"]);
        assert_eq!(selector.compounds[0].attributes.len(), 2);
        assert_eq!(specificity(&selector), (1, 3, 1));
        let attrs = [("id", "hero"), ("class", "card.wide"),
            ("data-kind", "top NEWS"), ("lang", "en-US")];
        let element = el("main", &attrs, vec![]);
        assert!(matches(&selector, &[&element]));
        assert_eq!(selected(r".\31 23").compounds[0].classes, ["123"]);
    }

    #[test]
    fn matches_all_combinators() {
        let children = vec![
            DomNode::Element(el("h2", &[("class", "lead")], vec![])), DomNode::Text("gap".into()),
            DomNode::Element(el("p", &[("class", "one")], vec![])),
            DomNode::Element(el("p", &[("class", "two")], vec![]))];
        let parent = el("div", &[], children);
        let DomNode::Element(target) = &parent.children[3] else { unreachable!() };
        let path = [&parent, target];
        assert!(matches(&selected("div > p.two"), &path));
        assert!(matches(&selected(".one + .two"), &path));
        assert!(matches(&selected(".lead ~ .two"), &path));
        assert!(matches(&selected("div .two"), &path));
        assert!(!matches(&selected(".lead + .two"), &path));
    }

    #[test]
    fn matches_structural_and_selector_list_pseudos() {
        let children = vec![DomNode::Element(el("i", &[], vec![])),
            DomNode::Element(el("b", &[("class", "hot")], vec![])),
            DomNode::Element(el("b", &[], vec![]))];
        let parent = el("section", &[], children);
        let DomNode::Element(target) = &parent.children[2] else { unreachable!() };
        let path = [&parent, target];
        assert!(matches(&selected("b:last-child:nth-child(2n + 1):nth-of-type(2)"), &path));
        assert!(matches(&selected("b:is(.cold, b):not(.hot)"), &path));
        assert!(!matches(&selected("b:first-child"), &path));
        assert_eq!(specificity(&selected("article:where(#x, .y):is(.a, #b)")), (1, 0, 1));
    }

    #[test]
    fn static_states_match_but_dynamic_states_do_not() {
        let attrs = [("type", "checkbox"), ("checked", ""), ("required", "")];
        let input = el("input", &attrs, vec![]);
        assert!(matches(&selected("input:checked:enabled:required"), &[&input]));
        assert!(!matches(&selected("input:optional"), &[&input]));
        assert!(!matches(&selected("input:hover"), &[&input]));
        assert!(!matches(&selected("input:focus-visible"), &[&input]));
    }

    #[test]
    fn pseudo_elements_have_a_separate_matching_entry_point() {
        let element = el("div", &[("class", "hero")], vec![]);
        for source in [".hero::after", ".hero:after"] {
            let selector = selected(source);
            assert_eq!(selector.pseudo_element, Some(PseudoElement::After));
            assert!(!matches(&selector, &[&element]));
            assert!(matches_for_pseudo(&selector, &[&element], PseudoElement::After));
            assert!(!matches_for_pseudo(&selector, &[&element], PseudoElement::Before));
            assert_eq!(specificity(&selector), (0, 1, 1));
        }
        assert_eq!(selected(".hero:before").pseudo_element, Some(PseudoElement::Before));
    }

    #[test]
    fn root_empty_and_only_child_are_structural() {
        let root = el("main", &[], vec![]);
        assert!(matches(&selected(":root:empty"), &[&root]));
        let parent = el("div", &[], vec![DomNode::Element(el("span", &[], vec![]))]);
        let DomNode::Element(child) = &parent.children[0] else { unreachable!() };
        assert!(matches(&selected("span:only-child"), &[&parent, child]));
    }

    #[test]
    fn matches_all_of_type_positions() {
        let parent = el("div", &[], vec![
            DomNode::Element(el("b", &[], vec![])),
            DomNode::Element(el("i", &[], vec![])),
            DomNode::Element(el("b", &[], vec![])),
        ]);
        let DomNode::Element(first_b) = &parent.children[0] else { unreachable!() };
        let DomNode::Element(only_i) = &parent.children[1] else { unreachable!() };
        let DomNode::Element(last_b) = &parent.children[2] else { unreachable!() };
        assert!(matches(&selected("b:first-of-type:nth-last-of-type(2)"), &[&parent, first_b]));
        assert!(matches(&selected("i:only-of-type"), &[&parent, only_i]));
        assert!(matches(&selected("b:last-of-type:nth-last-of-type(1)"), &[&parent, last_b]));
        assert!(!matches(&selected("b:only-of-type"), &[&parent, last_b]));
    }

    #[test]
    fn nth_child_of_complex_selector_lists_filters_siblings() {
        let parent = el("ul", &[], vec![
            DomNode::Element(el("span", &[("class", "marker")], vec![])),
            DomNode::Element(el("li", &[("class", "hot")], vec![])),
            DomNode::Element(el("li", &[], vec![])),
            DomNode::Element(el("li", &[("class", "hot")], vec![])),
        ]);
        let DomNode::Element(cold) = &parent.children[2] else { unreachable!() };
        let DomNode::Element(target) = &parent.children[3] else { unreachable!() };
        assert!(matches(
            &selected("li:nth-child(2 of .marker ~ li.hot)"),
            &[&parent, target],
        ));
        assert!(matches(
            &selected("li:nth-last-child(1 of .marker ~ li:is(.hot, ???))"),
            &[&parent, target],
        ));
        assert!(!matches(
            &selected("li:nth-child(1 of .hot)"),
            &[&parent, cold],
        ));
        assert_eq!(
            specificity(&selected("li:nth-child(2n of #featured, .hot)")),
            (1, 1, 1),
        );
    }

    #[test]
    fn top_level_lists_are_strict_but_function_lists_are_forgiving() {
        assert!(parse_selector_list(".ok, ???").is_empty());
        assert!(parse_selector_list(".ok,").is_empty());
        assert!(parse_selector_list("li:nth-child(2 of .ok, ???)").is_empty());

        let selector = selected("div:is(.ok, ???, #winner):not(???, .blocked)");
        let element = el("div", &[("class", "ok")], vec![]);
        assert!(matches(&selector, &[&element]));
        assert_eq!(specificity(&selector), (1, 1, 1));
    }

    #[test]
    fn nested_selector_functions_have_a_depth_limit() {
        let mut shallow = ".ok".to_string();
        for _ in 0..8 {
            shallow = format!(":is({shallow})");
        }
        let element = el("div", &[("class", "ok")], vec![]);
        assert!(matches(&selected(&shallow), &[&element]));

        let mut excessive = ".ok".to_string();
        for _ in 0..64 {
            excessive = format!(":is({excessive})");
        }
        assert!(parse_selector_list(&excessive).is_empty());
        let rescued = selected(&format!(":is(.ok, {excessive})"));
        assert!(matches(&rescued, &[&element]));
    }

    #[test]
    fn matcher_rejects_oversized_and_malformed_selector_shapes() {
        let element = el("div", &[], vec![]);
        let source = (0..MAX_SELECTOR_COMPOUNDS).map(|_| "div").collect::<Vec<_>>().join(" ");
        let maximum = selected(&source);
        let path = vec![&element; MAX_SELECTOR_COMPOUNDS];
        assert!(matches(&maximum, &path));

        let compound = selected("div").compounds.remove(0);
        let oversized = Selector { compounds: vec![compound.clone(); MAX_SELECTOR_COMPOUNDS + 1], combinators: vec![Combinator::Descendant; MAX_SELECTOR_COMPOUNDS], pseudo_element: None };
        let malformed = Selector { compounds: vec![compound; 2], combinators: vec![], pseudo_element: None };
        assert!(!matches(&oversized, &[&element]));
        assert!(!matches(&malformed, &[&element]));
    }

    #[test]
    fn has_matches_descendant_child_and_following_sibling_relatives() {
        let section = el("section", &[], vec![DomNode::Element(el(
            "article",
            &[("class", "card")],
            vec![DomNode::Element(el("span", &[("class", "icon")], vec![]))],
        ))]);
        let root = el("main", &[], vec![
            DomNode::Element(section),
            DomNode::Element(el("aside", &[("class", "next")], vec![])),
        ]);
        let DomNode::Element(section) = &root.children[0] else { unreachable!() };
        let path = [&root, section];
        assert!(matches(
            &selected("section:has(> article.card > span.icon)"),
            &path,
        ));
        assert!(matches(&selected("section:has(article .icon)"), &path));
        assert!(matches(&selected("section:has(+ aside.next)"), &path));
        assert!(matches(&selected("section:has(~ .next)"), &path));
        assert!(matches(
            &selected("section:has(???, > article.card)"),
            &path,
        ));
        assert!(!matches(&selected("section:has(> .missing)"), &path));
        assert_eq!(
            specificity(&selected("section:has(> #featured, .card)")),
            (1, 0, 1),
        );
    }

    #[test]
    fn lang_matches_inherited_language_ranges() {
        let root = el("main", &[("lang", "zh-Hans-CN")], vec![DomNode::Element(el(
            "section",
            &[],
            vec![DomNode::Element(el("span", &[("lang", "en-US")], vec![]))],
        ))]);
        let DomNode::Element(section) = &root.children[0] else { unreachable!() };
        let DomNode::Element(span) = &section.children[0] else { unreachable!() };
        assert!(matches(
            &selected(":lang(en, 'ZH-hans')"),
            &[&root, section],
        ));
        assert!(matches(&selected(":lang(en)"), &[&root, section, span]));
        assert!(matches(&selected(":lang(*)"), &[&root, section, span]));
        assert!(matches(&selected(":lang('*-US')"), &[&root, section, span]));
        assert!(!matches(
            &selected(":lang(zh-Hans)"),
            &[&root, section, span],
        ));
        let unknown = el("div", &[("lang", "")], vec![]);
        assert!(!matches(&selected(":lang(*)"), &[&root, &unknown]));
        assert_eq!(specificity(&selected(":lang(en)")), (0, 1, 0));
    }
}
