//! Proving the ZDeceptron arm is the compiler's output and not a flattering
//! transcription of it.
//!
//! `each` in the view is refused today (§16.5, M5b), so the benchmark's list
//! cannot come out of `zdc build`. What can, and does, is the row: its
//! template, the walk to the holes, and the sequence of bindings attached at
//! them. Those three things are what the row costs, so those three things
//! are extracted from both sides and compared.
//!
//! What is deliberately *not* compared is the last argument of each binding
//! — the getter. In the emission it reads a module signal; in the benchmark
//! it reads the row's item, which is what `each` would supply. That
//! substitution is the documented gap, and pinning everything around it is
//! what keeps the gap from widening unnoticed.

/// The parts of a row emission that determine what the row costs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowShape {
    /// The static HTML the region parses into, cloned per row.
    pub template: String,
    /// `const $nN = …;` — the compile-time offsets to each hole.
    pub walk: Vec<String>,
    /// Each binding, with its trailing getter or handler removed.
    pub bindings: Vec<String>,
}

/// The shape of the row the compiler emits for `bench/row.zd`.
pub fn emitted_row(client_js: &str) -> RowShape {
    let template = template_argument(client_js)
        .unwrap_or_else(|| panic!("the emitted module has no `template(…)` call:\n{client_js}"));
    let (walk, bindings) = statements(client_js.lines());
    RowShape {
        template,
        walk,
        bindings,
    }
}

/// The shape of the row the benchmark's ZDeceptron arm renders.
pub fn benchmark_row(benchmark_js: &str) -> RowShape {
    let template = row_html_constant(benchmark_js)
        .unwrap_or_else(|| panic!("`js/benchmark.js` has no `const ROW_HTML = '…';`"));
    let body = between(benchmark_js, "// ZDC-EMITTED-BEGIN", "// ZDC-EMITTED-END")
        .unwrap_or_else(|| panic!("`js/benchmark.js` has no ZDC-EMITTED-BEGIN/END region"));
    let (walk, bindings) = statements(body.lines());
    RowShape {
        template,
        walk,
        bindings,
    }
}

/// The single-quoted argument of the first `template(…)` call.
fn template_argument(source: &str) -> Option<String> {
    let start = source.find("template('")? + "template('".len();
    let end = source[start..].find("')")? + start;
    Some(source[start..end].to_string())
}

/// The single-quoted value of `const ROW_HTML = '…';`.
fn row_html_constant(source: &str) -> Option<String> {
    let line = source
        .lines()
        .find(|line| line.starts_with("const ROW_HTML = '"))?;
    let start = line.find('\'')? + 1;
    let end = line.rfind('\'')?;
    (end > start).then(|| line[start..end].to_string())
}

fn between<'a>(source: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = source.find(open)? + open.len();
    let end = source[start..].find(close)? + start;
    Some(&source[start..end])
}

/// Split a run of lines into the walk and the bindings.
///
/// `mount` is skipped: the emitted `main` mounts the region into a
/// container, and a row is inserted by `each` instead. That is the one
/// structural difference the gap forces, and naming it here keeps it from
/// being mistaken for drift.
fn statements<'a>(lines: impl Iterator<Item = &'a str>) -> (Vec<String>, Vec<String>) {
    let mut walk = Vec::new();
    let mut bindings = Vec::new();
    for line in lines {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("const $n") {
            walk.push(format!("const $n{rest}"));
            continue;
        }
        for name in ["bindText", "bindAttr", "bindStyle", "on"] {
            if line.starts_with(&format!("{name}(")) {
                bindings.push(without_last_argument(line));
            }
        }
    }
    (walk, bindings)
}

/// A call with its final argument dropped: `on($n2, 'click', () => …)`
/// becomes `on($n2, 'click')`.
fn without_last_argument(call: &str) -> String {
    let Some(open) = call.find('(') else {
        return call.to_string();
    };
    let name = &call[..open];
    let inner = &call[open + 1..];
    let mut arguments: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;

    for character in inner.chars() {
        if let Some(open_quote) = quote {
            current.push(character);
            if character == open_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => {
                quote = Some(character);
                current.push(character);
            }
            '(' | '[' | '{' => {
                depth += 1;
                current.push(character);
            }
            ')' if depth == 0 => break,
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                current.push(character);
            }
            ',' if depth == 0 => {
                arguments.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(character),
        }
    }
    if !current.trim().is_empty() {
        arguments.push(current.trim().to_string());
    }
    arguments.pop();
    format!("{name}({})", arguments.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_argument_is_dropped_whatever_it_contains() {
        assert_eq!(
            without_last_argument("on($n2, 'click', () => f(a, b));"),
            "on($n2, 'click')"
        );
        assert_eq!(
            without_last_argument("bindText($n1.firstChild, rowId);"),
            "bindText($n1.firstChild)"
        );
        assert_eq!(
            without_last_argument("bindAttr($n0, 'class', () => 'a, b' + (c)());"),
            "bindAttr($n0, 'class')"
        );
    }

    #[test]
    fn a_walk_and_its_bindings_are_separated() {
        let (walk, bindings) = statements(
            [
                "  const $n0 = $r.firstChild;",
                "  bindText($n0, x);",
                "  return mount($r, c);",
            ]
            .into_iter(),
        );
        assert_eq!(walk, vec!["const $n0 = $r.firstChild;"]);
        assert_eq!(bindings, vec!["bindText($n0)"]);
    }

    #[test]
    fn the_template_argument_is_read_out_of_an_emission() {
        assert_eq!(
            template_argument("const $t0 = template('<div></div>');\n").as_deref(),
            Some("<div></div>")
        );
    }

    #[test]
    fn a_missing_marker_is_not_silently_an_empty_region() {
        assert!(between("nothing here", "// A", "// B").is_none());
    }
}
