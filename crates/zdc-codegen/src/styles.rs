//! Generated scoped classes, per spec §6 and §16.3.11.
//!
//! A static style set folds into one generated class and costs nothing at
//! runtime; only a style that reads a signal becomes a `bindStyle`. This is
//! the fourth pipeline output §7.1 lists. This was written when it was the
//! first layer of M8 and the surface was two CSS properties. It is now 33
//! style arguments over a value grammar apiece, each taking any of seven
//! conditional prefixes, so a set is a set of conditioned declarations and
//! the interning property survived the change. The layer is no longer the
//! first one.

use zdc_runtime::BASE_CSS;

use crate::style::{Condition, Declaration};

/// One generated class per *distinct* declaration set, so two elements that
/// style the same way share a class rather than each getting their own.
///
/// A set is a set of *conditioned* declarations (`hover`, a breakpoint, the
/// dark colour scheme), not of plain ones. That is the whole change hover
/// and media queries needed. The interning property §16.3.11 asks for is
/// unchanged, one class per distinct set, because two elements that hover
/// the same way still share a class.
#[derive(Default)]
pub struct Styles {
    sets: Vec<Vec<Declaration>>,
}

impl Styles {
    /// The class name for this declaration set, generating one if the set
    /// is new.
    pub fn intern(&mut self, mut declarations: Vec<Declaration>) -> String {
        // Sorting first is what makes `padding is 8, weight is "bold"` and
        // `weight is "bold", padding is 8` one class rather than two.
        // `Declaration` orders by condition, then property, then value, so
        // a resting colour sorts before the `:hover` colour that overrides
        // it and the printed order below is the cascade order.
        declarations.sort();
        declarations.dedup_by(|a, b| a.condition == b.condition && a.property == b.property);
        match self.sets.iter().position(|set| *set == declarations) {
            Some(index) => format!("zd-s{index}"),
            None => {
                self.sets.push(declarations);
                format!("zd-s{}", self.sets.len() - 1)
            }
        }
    }

    /// The whole stylesheet: the base classes the built-ins carry, then one
    /// rule per generated class per condition.
    ///
    /// The generated rules follow `base.css`, and that is the answer to
    /// what happens when a program styles a `Row`: every rule here has one
    /// class of specificity, exactly as `.zd-row` does, so the later rule
    /// wins and `display is "block"` un-flexes a row rather than being
    /// silently ignored. `a_generated_rule_follows_the_base_class` pins it.
    pub fn stylesheet(&self) -> String {
        let mut out = BASE_CSS.to_string();
        if self.sets.is_empty() {
            return out;
        }
        out.push_str("\n/* generated — one class per distinct style set */\n");
        for (index, set) in self.sets.iter().enumerate() {
            for (condition, group) in group_by_condition(set) {
                out.push_str(&rule(index, condition, &group));
            }
        }
        out
    }
}

/// The declarations of one set, split into runs sharing a condition.
///
/// The set is already sorted by condition, so a run is contiguous and this
/// is a fold rather than a grouping pass over a map, which also means the
/// output order is the `Condition` order, and that order is the cascade.
fn group_by_condition(set: &[Declaration]) -> Vec<(Condition, Vec<&Declaration>)> {
    let mut out: Vec<(Condition, Vec<&Declaration>)> = Vec::new();
    for declaration in set {
        match out.last_mut() {
            Some((condition, group)) if *condition == declaration.condition => {
                group.push(declaration)
            }
            _ => out.push((declaration.condition, vec![declaration])),
        }
    }
    out
}

/// One printed rule, wrapped in its at-rule when it has one.
fn rule(index: usize, condition: Condition, group: &[&Declaration]) -> String {
    let (suffix, at_rule) = condition.wrapping();
    let mut body = format!(".zd-s{index}{suffix} {{");
    for declaration in group {
        body.push_str(&format!(
            " {}: {};",
            declaration.property, declaration.value
        ));
    }
    body.push_str(" }");
    match at_rule {
        Some(at_rule) => format!("{at_rule} {{ {body} }}\n"),
        None => format!("{body}\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_declarations_in_any_order_share_one_class() {
        let mut styles = Styles::default();
        let first = styles.intern(vec![
            Declaration::always("padding", "8px"),
            Declaration::always("font-weight", "bold"),
        ]);
        let second = styles.intern(vec![
            Declaration::always("font-weight", "bold"),
            Declaration::always("padding", "8px"),
        ]);
        assert_eq!(first, "zd-s0");
        assert_eq!(second, first);
    }

    #[test]
    fn distinct_declarations_get_distinct_classes() {
        let mut styles = Styles::default();
        assert_eq!(
            styles.intern(vec![Declaration::always("padding", "8px")]),
            "zd-s0"
        );
        assert_eq!(
            styles.intern(vec![Declaration::always("padding", "4px")]),
            "zd-s1"
        );
    }

    #[test]
    fn the_stylesheet_carries_the_base_classes_and_the_generated_ones() {
        let mut styles = Styles::default();
        styles.intern(vec![Declaration::always("padding", "8px")]);
        let sheet = styles.stylesheet();
        assert!(sheet.contains(".zd-col"), "base classes must ship: {sheet}");
        assert!(sheet.contains(".zd-s0 { padding: 8px; }"), "{sheet}");
    }

    #[test]
    fn the_stylesheet_balances_its_braces() {
        let mut styles = Styles::default();
        styles.intern(vec![
            Declaration::always("color", "red"),
            Declaration::always("padding", "8px"),
        ]);
        let sheet = styles.stylesheet();
        assert_eq!(
            sheet.matches('{').count(),
            sheet.matches('}').count(),
            "the stylesheet must balance:\n{sheet}"
        );
    }

    /// Every generated rule follows `base.css`, which is what makes
    /// `display is "block"` on a `Row` beat `.zd-row`'s `display: flex`.
    #[test]
    fn a_generated_rule_follows_the_base_class() {
        let mut styles = Styles::default();
        styles.intern(vec![Declaration::always("display", "block")]);
        let sheet = styles.stylesheet();
        let base = sheet.find(".zd-row").expect("the base classes ship");
        let generated = sheet.find(".zd-s0").expect("the generated class ships");
        assert!(base < generated, "{sheet}");
    }

    #[test]
    fn a_program_with_no_styles_still_ships_the_base() {
        let sheet = Styles::default().stylesheet();
        assert!(sheet.contains(".zd-row"));
        assert!(!sheet.contains(".zd-s0"));
    }
}
