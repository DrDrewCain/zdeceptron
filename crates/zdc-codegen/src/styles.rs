//! Generated scoped classes, per spec §6 and §16.3.11.
//!
//! A static style set folds into one generated class and costs nothing at
//! runtime; only a style that reads a signal becomes a `bindStyle`. This is
//! the fourth pipeline output §7.1 lists. This was written when it was the
//! first layer of M8 and the surface was two CSS properties. It is now 35
//! style arguments over a value grammar apiece, each taking any of seven
//! conditional prefixes or three keyframe steps, so a set is a set of
//! conditioned declarations and the interning property survived the
//! change. The layer is no longer the first one.
//!
//! A set now prints as two kinds of thing: rules, and — when it holds
//! steps — the one `@keyframes` block those steps are. That is the whole
//! of what #189 needed from this module, and it needed it because
//! `@keyframes` is a top-level at-rule rather than a declaration, so
//! there was nowhere in a rule to put it.

use zdc_runtime::BASE_CSS;

use crate::style::{Condition, Declaration, MOTION_QUERY};

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
    /// rule per generated class per condition, and one `@keyframes` block
    /// per class that animates.
    ///
    /// The generated rules follow `base.css`, and that is the answer to
    /// what happens when a program styles a `Row`: every rule here has one
    /// class of specificity, exactly as `.zd-row` does, so the later rule
    /// wins and `display is "block"` un-flexes a row rather than being
    /// silently ignored. `a_generated_rule_follows_the_base_class` pins it.
    ///
    /// # Where an at-rule goes
    ///
    /// `@keyframes` is not a declaration, so it cannot be printed where
    /// the declarations are — it is a top-level rule that happens to be
    /// *named*. The name is the answer to where it goes: a set's steps
    /// become `@keyframes zd-k{index}` beside the `.zd-s{index}` rules
    /// they belong to, so the sheet reads as one block per interned set
    /// and the output order is the set order, which is the order the
    /// emitter met them in. Nothing here iterates a map.
    pub fn stylesheet(&self) -> String {
        let mut out = BASE_CSS.to_string();
        if self.sets.is_empty() {
            return out;
        }
        out.push_str("\n/* generated — one class per distinct style set */\n");
        for (index, set) in self.sets.iter().enumerate() {
            // A step is not a rule, so it leaves here by a different door.
            let (steps, mut rules): (Vec<Declaration>, Vec<Declaration>) = set
                .iter()
                .cloned()
                .partition(|declaration| declaration.condition.offset().is_some());
            if !steps.is_empty() {
                out.push_str(&keyframes(index, &steps));
                let rest = completion(index, &rules);
                rules.extend(rest);
                // Sorted again, because `group_by_condition` folds runs
                // rather than grouping, and what was just appended belongs
                // in the middle of the set.
                rules.sort();
            }
            for (condition, group) in group_by_condition(&rules) {
                // unreached: `partition` above took every condition that
                // has no wrapping, so what is left is a circumstance.
                let Some(wrapping) = condition.wrapping() else {
                    continue;
                };
                out.push_str(&rule(index, wrapping, &group));
            }
        }
        out
    }
}

/// The parts of an animation that are not a choice the program made.
///
/// Three declarations, and none of them is a word a program can write.
/// **The name cannot be**, because the name *is* the identity of the
/// interned set: `zd-k{index}` is derived from the set, so putting it
/// inside the set would be asking what index a set has before deciding
/// which set it is. Everything a program does choose — the duration, the
/// repetition, the steps — is in the set, and this is the remainder.
///
/// `animation-fill-mode: both` is what makes an entrance an entrance. Left
/// off, an element fades in and then snaps back to its resting style the
/// instant the animation ends, which is the single most common way a
/// hand-written keyframe animation goes wrong.
///
/// The timing function follows the repetition rather than being a fourth
/// argument, because there is only one right answer either way: `ease`
/// starts and ends slowly, which is what an entrance wants and what a
/// *loop* cannot have — a loop eased at both ends lurches at the seam
/// where its end meets its start, sixty times a minute, forever. A
/// repeating animation is `linear` for the same reason a clock hand is.
fn completion(index: usize, rules: &[Declaration]) -> Vec<Declaration> {
    let loops = rules.iter().any(|declaration| {
        declaration.property == "animation-iteration-count" && declaration.value == "infinite"
    });
    let motion = |property: &str, value: &str| Declaration {
        condition: Condition::Motion,
        property: property.to_string(),
        value: value.to_string(),
    };
    vec![
        motion("animation-name", &format!("zd-k{index}")),
        motion("animation-fill-mode", "both"),
        motion(
            "animation-timing-function",
            if loops { "linear" } else { "ease" },
        ),
    ]
}

/// One set's steps, as the `@keyframes` block the rules above name.
///
/// **Inside the motion query, and that is not belt and braces.** The rule
/// that names this block is conditioned on `prefers-reduced-motion:
/// no-preference` and so is the block, so a reader who asked for less
/// motion has no animation *and* no definition of one: there is nothing
/// left for a later argument, or a later contributor, to accidentally
/// reference from outside the query. `@keyframes` inside `@media` is
/// plain CSS conditional-rules and has been in every engine for a decade.
///
/// One line, like every other rule here, so that a test can assert that
/// every line mentioning motion is inside the query by reading lines.
fn keyframes(index: usize, steps: &[Declaration]) -> String {
    let mut body = format!("@keyframes zd-k{index} {{");
    for (condition, group) in group_by_condition(steps) {
        // unreached: every declaration here was partitioned *by* `offset`.
        let Some(offset) = condition.offset() else {
            continue;
        };
        body.push_str(&format!(" {offset} {{"));
        for declaration in group {
            body.push_str(&format!(
                " {}: {};",
                declaration.property, declaration.value
            ));
        }
        body.push_str(" }");
    }
    body.push_str(" }");
    format!("{MOTION_QUERY} {{ {body} }}\n")
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
fn rule(
    index: usize,
    (suffix, at_rule): (&'static str, Option<String>),
    group: &[&Declaration],
) -> String {
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

    /// One step, written the way the emitter writes one.
    fn step(condition: Condition, property: &str, value: &str) -> Declaration {
        Declaration {
            condition,
            property: property.to_string(),
            value: value.to_string(),
        }
    }

    /// A fade-in: two steps and a duration, which is the whole of what a
    /// program writes.
    fn fade() -> Vec<Declaration> {
        vec![
            step(Condition::From, "opacity", "0"),
            step(Condition::To, "opacity", "1"),
            step(Condition::Motion, "animation-duration", "200ms"),
        ]
    }

    #[test]
    fn a_set_with_steps_prints_them_as_a_keyframes_block() {
        let mut styles = Styles::default();
        assert_eq!(styles.intern(fade()), "zd-s0");
        let sheet = styles.stylesheet();
        assert!(
            sheet.contains(
                "@media (prefers-reduced-motion: no-preference) { @keyframes zd-k0 \
                 { from { opacity: 0; } to { opacity: 1; } } }"
            ),
            "{sheet}"
        );
    }

    /// The block's name is the set's index, so the rule that names it and
    /// the block itself cannot disagree.
    #[test]
    fn the_rule_names_the_block_the_same_set_produced() {
        let mut styles = Styles::default();
        styles.intern(vec![Declaration::always("padding", "8px")]);
        assert_eq!(styles.intern(fade()), "zd-s1");
        let sheet = styles.stylesheet();
        assert!(sheet.contains("@keyframes zd-k1 "), "{sheet}");
        assert!(sheet.contains("animation-name: zd-k1;"), "{sheet}");
        assert!(!sheet.contains("zd-k0"), "{sheet}");
    }

    /// The interning property, over the thing that made it a question: a
    /// keyframe block is part of the set, so two elements that animate the
    /// same way share the class *and* the block.
    #[test]
    fn two_elements_that_animate_alike_share_one_block() {
        let mut styles = Styles::default();
        assert_eq!(styles.intern(fade()), "zd-s0");
        assert_eq!(styles.intern(fade()), "zd-s0");
        let sheet = styles.stylesheet();
        assert_eq!(sheet.matches("@keyframes").count(), 1, "{sheet}");
    }

    #[test]
    fn an_animation_that_repeats_is_linear_and_one_that_does_not_is_eased() {
        let mut styles = Styles::default();
        styles.intern(fade());
        let mut looping = fade();
        looping.push(step(
            Condition::Motion,
            "animation-iteration-count",
            "infinite",
        ));
        styles.intern(looping);
        let sheet = styles.stylesheet();
        assert!(
            sheet.contains(
                ".zd-s0 { animation-duration: 200ms; animation-fill-mode: both; \
                 animation-name: zd-k0; animation-timing-function: ease; }"
            ),
            "{sheet}"
        );
        assert!(
            sheet.contains(
                ".zd-s1 { animation-duration: 200ms; animation-fill-mode: both; \
                 animation-iteration-count: infinite; animation-name: zd-k1; \
                 animation-timing-function: linear; }"
            ),
            "{sheet}"
        );
    }

    /// **The accessibility property, asserted over the whole sheet.**
    ///
    /// Not "the animation is inside the query" but "nothing about the
    /// animation is outside it": the declarations, the block that defines
    /// the steps, and the name that joins them. A reader who asked for
    /// less motion gets a page where the animation does not exist.
    #[test]
    fn nothing_about_an_animation_is_declared_outside_the_motion_query() {
        let mut styles = Styles::default();
        let mut set = fade();
        set.push(Declaration::always("color", "red"));
        styles.intern(set);
        let sheet = styles.stylesheet();
        let mut checked = 0;
        for line in sheet.lines() {
            if line.contains("animation") || line.contains("@keyframes") {
                checked += 1;
                assert!(
                    line.contains("prefers-reduced-motion: no-preference"),
                    "motion escaped the query:\n{line}"
                );
            }
        }
        assert_eq!(checked, 2, "the rule and the block must both be there");
        // And the styling that is not motion is untouched by any of it.
        assert!(sheet.contains(".zd-s0 { color: red; }"), "{sheet}");
    }

    #[test]
    fn a_sheet_with_keyframes_balances_its_braces() {
        let mut styles = Styles::default();
        let mut set = fade();
        set.push(step(Condition::Mid, "opacity", "0.4"));
        styles.intern(set);
        let sheet = styles.stylesheet();
        assert_eq!(
            sheet.matches('{').count(),
            sheet.matches('}').count(),
            "the sheet does not balance:\n{sheet}"
        );
        assert!(sheet.contains("50% { opacity: 0.4; }"), "{sheet}");
    }
}
