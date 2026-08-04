//! Generated scoped classes, per spec §6 and §16.3.11.
//!
//! A static style set folds into one generated class and costs nothing at
//! runtime; only a style that reads a signal becomes a `bindStyle`. This is
//! the fourth pipeline output §7.1 lists and the first layer of M8.

use zdc_runtime::BASE_CSS;

/// One generated class per *distinct* declaration set, so two elements that
/// style the same way share a class rather than each getting their own.
#[derive(Default)]
pub struct Styles {
    sets: Vec<Vec<(String, String)>>,
}

impl Styles {
    /// The class name for this declaration set, generating one if the set
    /// is new.
    pub fn intern(&mut self, mut declarations: Vec<(String, String)>) -> String {
        // Sorting first is what makes `padding is 8, weight is "bold"` and
        // `weight is "bold", padding is 8` one class rather than two.
        declarations.sort();
        declarations.dedup_by(|a, b| a.0 == b.0);
        match self.sets.iter().position(|set| *set == declarations) {
            Some(index) => format!("zd-s{index}"),
            None => {
                self.sets.push(declarations);
                format!("zd-s{}", self.sets.len() - 1)
            }
        }
    }

    /// The whole stylesheet: the base classes the built-ins carry, then one
    /// rule per generated class.
    pub fn stylesheet(&self) -> String {
        let mut out = BASE_CSS.to_string();
        if self.sets.is_empty() {
            return out;
        }
        out.push_str("\n/* generated — one class per distinct style set */\n");
        for (index, set) in self.sets.iter().enumerate() {
            out.push_str(&format!(".zd-s{index} {{"));
            for (property, value) in set {
                out.push_str(&format!(" {property}: {value};"));
            }
            out.push_str(" }\n");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_declarations_in_any_order_share_one_class() {
        let mut styles = Styles::default();
        let first = styles.intern(vec![
            ("padding".into(), "8px".into()),
            ("font-weight".into(), "bold".into()),
        ]);
        let second = styles.intern(vec![
            ("font-weight".into(), "bold".into()),
            ("padding".into(), "8px".into()),
        ]);
        assert_eq!(first, "zd-s0");
        assert_eq!(second, first);
    }

    #[test]
    fn distinct_declarations_get_distinct_classes() {
        let mut styles = Styles::default();
        assert_eq!(
            styles.intern(vec![("padding".into(), "8px".into())]),
            "zd-s0"
        );
        assert_eq!(
            styles.intern(vec![("padding".into(), "4px".into())]),
            "zd-s1"
        );
    }

    #[test]
    fn the_stylesheet_carries_the_base_classes_and_the_generated_ones() {
        let mut styles = Styles::default();
        styles.intern(vec![("padding".into(), "8px".into())]);
        let sheet = styles.stylesheet();
        assert!(sheet.contains(".zd-col"), "base classes must ship: {sheet}");
        assert!(sheet.contains(".zd-s0 { padding: 8px; }"), "{sheet}");
    }

    #[test]
    fn a_program_with_no_styles_still_ships_the_base() {
        let sheet = Styles::default().stylesheet();
        assert!(sheet.contains(".zd-row"));
        assert!(!sheet.contains(".zd-s0"));
    }
}
