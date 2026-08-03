//! A file the parser cannot handle must produce a diagnostic, not a
//! dead process.
//!
//! Recursive descent turns nesting into stack frames. Overflowing the
//! stack raises `SIGABRT`: it is not a panic, `catch_unwind` cannot
//! contain it, and no message is ever printed — `zdc parse` on a
//! truncated or binary file would simply die. Each of these inputs used
//! to abort the test process; each must now come back as an ordinary
//! parse error.

/// Deep enough to have overflowed an 8 MB stack before the limit existed
/// (measured: 800–1600 unbalanced parens in debug), and far beyond the
/// limit itself.
const DEEP: usize = 1000;

fn assert_rejected(label: &str, src: &str) -> String {
    match zdc_parser::parse(src) {
        Ok(_) => panic!("{label}: expected an error, but the file parsed"),
        Err(e) => {
            assert!(
                !e.message.is_empty(),
                "{label}: an error must carry a message"
            );
            e.message
        }
    }
}

#[test]
fn deeply_nested_parentheses_are_reported_not_fatal() {
    let src = format!("function f\n    give {}\n", "(".repeat(DEEP));
    let message = assert_rejected("unbalanced parens", &src);
    assert!(
        message.contains("nested more than"),
        "expected a nesting-depth message, got: {message}"
    );
}

#[test]
fn deeply_nested_balanced_parentheses_are_reported_not_fatal() {
    let src = format!(
        "function f\n    give {}1{}\n",
        "(".repeat(DEEP),
        ")".repeat(DEEP)
    );
    let message = assert_rejected("balanced parens", &src);
    assert!(
        message.contains("nested more than"),
        "expected a nesting-depth message, got: {message}"
    );
}

#[test]
fn a_deeply_nested_type_is_reported_not_fatal() {
    let src = format!(
        "state x is client {}Item starting empty\n",
        "List of ".repeat(DEEP)
    );
    let message = assert_rejected("nested type", &src);
    assert!(
        message.contains("nested more than"),
        "expected a nesting-depth message, got: {message}"
    );
}

#[test]
fn deeply_nested_indentation_is_reported_not_fatal() {
    let mut src = String::from("function f\n");
    for level in 1..=DEEP {
        src.push_str(&" ".repeat(level * 4));
        src.push_str("each item in items\n");
    }
    src.push_str(&" ".repeat((DEEP + 1) * 4));
    src.push_str("give 1\n");

    let message = assert_rejected("nested blocks", &src);
    assert!(
        message.contains("nested more than"),
        "expected a nesting-depth message, got: {message}"
    );
}

#[test]
fn deeply_nested_view_nodes_are_reported_not_fatal() {
    let mut src = String::from("view\n");
    for level in 1..=DEEP {
        src.push_str(&" ".repeat(level * 4));
        src.push_str("Column\n");
    }

    let message = assert_rejected("nested view nodes", &src);
    assert!(
        message.contains("nested more than"),
        "expected a nesting-depth message, got: {message}"
    );
}

#[test]
fn deeply_nested_prefix_operators_are_reported_not_fatal() {
    let src = format!("function f\n    give {}a\n", "not ".repeat(DEEP));
    let message = assert_rejected("prefix operators", &src);
    assert!(
        message.contains("nested more than"),
        "expected a nesting-depth message, got: {message}"
    );
}

/// The limit exists to catch pathological input, not to constrain
/// programs anyone writes. Nesting that a person would plausibly type
/// must still parse.
#[test]
fn ordinary_nesting_is_unaffected() {
    let src = "state x is client List of Map of Id to List of Item starting empty\n\
               function f\n    \
               give not ((a + b) * (c - d))\n\
               view\n    \
               Column\n        \
               Row\n            \
               Column\n                \
               Row\n                    \
               Text \"deep enough\"\n";
    zdc_parser::parse(src).expect("everyday nesting must still parse");
}

// ---------------------------------------------------------------------
// Spines: deep trees the parser builds without recursing.
// ---------------------------------------------------------------------

/// A left-associative operator loop grows the tree one level per
/// iteration and the stack not at all, so counting frames did not bound
/// it: `1 + 1 + …` twenty thousand times parsed at depth 1, and the
/// SIGABRT the limit exists to prevent moved out of the parser and into
/// the first pass that walked the result. `zdc check` died with no
/// diagnostic at all.
#[test]
fn a_long_infix_chain_is_reported_not_fatal() {
    let src = format!(
        "state x is client Whole starting {}1\n",
        "1 + ".repeat(DEEP)
    );
    let message = assert_rejected("infix chain", &src);
    assert!(
        message.contains("nested more than"),
        "expected a nesting-depth message, got: {message}"
    );
}

/// `.f.f.f…` is the same spine through the postfix loop.
#[test]
fn a_long_projection_chain_is_reported_not_fatal() {
    let src = format!("function f with x\n    give x{}\n", ".f".repeat(DEEP));
    let message = assert_rejected("projection chain", &src);
    assert!(
        message.contains("nested more than"),
        "expected a nesting-depth message, got: {message}"
    );
}

/// And `x at i at i…` is the third.
#[test]
fn a_long_index_chain_is_reported_not_fatal() {
    let src = format!("function f with x\n    give x{}\n", " at 0".repeat(DEEP));
    let message = assert_rejected("index chain", &src);
    assert!(
        message.contains("nested more than"),
        "expected a nesting-depth message, got: {message}"
    );
}

/// The spine budget is handed back once the spine is built, so a file
/// full of ordinary-length chains does not accumulate one file-wide
/// depth and start rejecting its own last line.
#[test]
fn many_ordinary_chains_do_not_accumulate() {
    let mut src = String::from("function f with x\n");
    for _ in 0..500 {
        src.push_str("    give 1 + 2 + 3 + 4 + 5 + x.a.b at 0\n");
    }
    zdc_parser::parse(&src).expect("chains must not accumulate across statements");
}

/// The two budgets are spent independently, so the real worst case is
/// both at once: indentation nested to its limit with an expression
/// nested to its limit inside the innermost block. Even that must come
/// back as a message.
#[test]
fn both_limits_reached_at_once_is_reported_not_fatal() {
    let mut src = String::from("function f\n");
    for level in 1..=31 {
        src.push_str(&" ".repeat(level * 4));
        src.push_str("each item in items\n");
    }
    src.push_str(&" ".repeat(32 * 4));
    src.push_str(&format!("give {}1\n", "(".repeat(DEEP)));

    let message = assert_rejected("blocks and expressions together", &src);
    assert!(
        message.contains("nested more than"),
        "expected a nesting-depth message, got: {message}"
    );
}
