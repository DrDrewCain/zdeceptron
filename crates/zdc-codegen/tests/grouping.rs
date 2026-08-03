//! What a no-op emission does to the grouping the source wrote.
//!
//! Every test here **runs** the emitted bundle and asserts the computed
//! value. That is the whole point of the file. The defect it was written
//! for emitted `r$ * 10 + d$ * 3` for `r * (decimalOf of (10 + (d * 3)))`:
//! it typechecks, it builds, it links, and it computes 32 where the source
//! says 44. No assertion over emitted *text* would have been written to
//! catch it, because the text is perfectly reasonable JavaScript — it is
//! just a different expression. A wrong number that typechecks is the
//! worst thing this compiler can produce, because nothing announces it.
//!
//! **The hazard, stated generally.** An operation whose emission is a
//! no-op — an identity conversion, a bare property access, a `static`
//! read inlined as a literal — puts its operand into the surrounding
//! expression *without any syntax of its own*. A call does not have this
//! problem: `f(a + b)` carries its own parentheses whatever `a + b` binds
//! at. A no-op carries nothing, so the operand's own precedence is the
//! only thing standing between the source's grouping and the surrounding
//! operator's. If the emitter mislabels that precedence — and it did, by
//! asserting `PRIMARY` for an argument whose real precedence it had
//! already computed and thrown away — the surrounding operator
//! reassociates the expression and the program computes a different
//! number.
//!
//! So each test below puts an operation's result inside an operator that
//! binds **tighter** than the operand does, which is the only arrangement
//! in which the mislabelling is observable. Where an operation turns out
//! to be safe, it is safe for a reason recorded beside it — safe by
//! construction is worth knowing, and is not the same as untested.

mod support;

use std::collections::BTreeMap;

use support::{compile_source, context, run, try_compile_with_statics};
use zdc_codegen::Bundle;

/// Render a bundle in the embedded engine and return the page's text.
fn rendered(bundle: &Bundle) -> String {
    let mut context = context(false);
    let page = run(
        &mut context,
        &bundle.client_js,
        "const $host = document.createElement('div');\nmain($host);\nserialize($host)",
    );
    let mut out = String::new();
    let mut inside_tag = false;
    for ch in page.chars() {
        match ch {
            '<' => inside_tag = true,
            '>' => inside_tag = false,
            _ if !inside_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Compile `declarations`, run the bundle, and give back what the one
/// `Text answer` node says.
///
/// The value has to survive the parser, the resolver, the split, the
/// checker, emission *and* execution for this to be an answer at all,
/// which is what makes it a test of the number rather than of the text.
fn computed(declarations: &str) -> String {
    let source = format!("{declarations}view\n    Text answer\n");
    rendered(&compile_source(&source))
}

// --- the reported defect, asserted on the computed value ------------------

/// The headline case, verbatim from the report.
///
/// `decimalOf` is a widening between two types that are both f64
/// (§14A.3), so its emission is the identity — and the identity dropped
/// the operand's parentheses along with the call, which let the
/// surrounding `*` bind into the `+`.
///
/// 2 × (10 + 4×3) is 2 × 22 is **44**. Reassociated to `2 * 10 + 4 * 3`
/// it is 20 + 12 is **32**, which is what this emitted before the fix.
#[test]
fn an_identity_conversion_keeps_the_grouping_the_source_wrote() {
    assert_eq!(
        computed(
            "state d is client Whole starting 4\n\
             state r is client Decimal from decimalOf of 2\n\
             state answer is client Text from text of (r * (decimalOf of (10 + (d * 3))))\n"
        ),
        "44",
        "`r * (decimalOf of (10 + (d * 3)))` is 2 × 22. A 32 means the \
         emitted `*` bound into the `+`, because the identity conversion \
         dropped the grouping."
    );
}

/// The same defect on the other side of the operator, where the sign of
/// the answer changes rather than its magnitude.
///
/// `0 - (10 + 4)` is −14; reassociated to `0 - 10 + 4` it is −6.
#[test]
fn an_identity_conversion_keeps_its_grouping_under_a_subtraction() {
    assert_eq!(
        computed(
            "state d is client Whole starting 4\n\
             state answer is client Text from text of (0 - (decimalOf of (10 + d)))\n"
        ),
        "-14"
    );
}

/// And under a division, where reassociation is not even commutative.
///
/// `(8 + 4) / 2` is 6; `8 + 4 / 2` is 10.
#[test]
fn an_identity_conversion_keeps_its_grouping_under_a_division() {
    assert_eq!(
        computed(
            "state d is client Whole starting 4\n\
             state r is client Decimal from decimalOf of 2\n\
             state answer is client Text from text of ((decimalOf of (8 + d)) / r)\n"
        ),
        "6"
    );
}

// --- the sibling sweep ----------------------------------------------------
//
// Everything else in the compiler whose emission is a no-op, a bare
// unwrap, or otherwise puts its operand into the surrounding expression
// unchanged. Each is computed inside a tighter-binding operator, because
// that is the only place the defect is visible.

/// `text of` a `Text` is the emitter's **other** [`JsForm::Identity`], and
/// it reaches emission by a different route: the `operator` walk, which
/// emits its operand with `value()` and keeps the `Expr` it gets back.
/// That route never had the defect — but "a different function does it
/// correctly" is exactly the kind of claim that stops being true silently.
#[test]
fn text_of_a_text_keeps_its_grouping() {
    assert_eq!(
        computed(
            "state t is client Text starting \"cd\"\n\
             state answer is client Text from text of (length of (text of (\"ab\" + t)))\n"
        ),
        "4"
    );
    assert_eq!(
        computed(
            "state t is client Text starting \"cd\"\n\
             state answer is client Text from \"[\" + (text of (\"ab\" + t)) + \"]\"\n"
        ),
        "[abcd]"
    );
}

/// `length of` a list and of a map are [`JsForm::Field`] — `.length` and
/// `.size`, a property access with no call at all, which is the other
/// form that appends to its operand rather than wrapping it.
///
/// These went through the same fabricated-`PRIMARY` line as `decimalOf`
/// and were **not** observably wrong, for a reason worth writing down: no
/// expression in this language produces a `List` or a `Map` at a
/// precedence below `MEMBER`. A list is a literal, an `append` chain, a
/// call, or a read — there is no `+` on lists — so `a + b.length` was not
/// constructible. That is a fact about today's grammar and not about this
/// emission, so the fix covers it and these pin it.
#[test]
fn a_property_access_keeps_its_operands_grouping() {
    assert_eq!(
        computed(
            "state xs is client List of Whole starting [1, 2]\n\
             state answer is client Text from text of (10 * (length of (append 3 to xs)))\n"
        ),
        "30"
    );
    assert_eq!(
        computed(
            "state xs is client List of Whole starting [1, 2]\n\
             state answer is client Text from text of (10 * (listLength of (append 3 to xs)))\n"
        ),
        "30"
    );
    assert_eq!(
        computed(
            "state m is client Map of Text to Whole starting [\"a\" to 1, \"b\" to 2]\n\
             state answer is client Text from text of (10 * (mapLength of m))\n"
        ),
        "20"
    );
}

/// `floor of` and `round of` are [`JsForm::Helper`]: `$floor(x)`, which
/// brings its own parentheses. Safe by construction — and the
/// construction is what is being pinned, because turning either into an
/// `Identity` (both are no-ops on a value that is already whole) would
/// silently reintroduce the defect.
#[test]
fn the_rounding_operations_keep_their_operands_grouping() {
    let program = |op: &str| {
        format!(
            "state r is client Decimal from decimalOf of 2\n\
             state answer is client Text from text of \
             (10 * (valueOr with maybe is ({op} of (r + (r * 3))), fallback is 0))\n"
        )
    };
    assert_eq!(computed(&program("floor")), "80");
    assert_eq!(computed(&program("round")), "80");
}

/// `valueOr` is the `Option` elimination in expression position (§14F.2a)
/// and is an ordinary ZDeceptron function, so it emits a call. Both its
/// arguments are inside an argument list, where a comma separates and the
/// call's own parentheses group.
#[test]
fn an_option_unwrap_keeps_both_its_operands_grouping() {
    assert_eq!(
        computed(
            "state d is client Whole starting 4\n\
             state answer is client Text from text of \
             (10 * (valueOr with maybe is (floor of 1.5), fallback is (10 + (d * 3))))\n"
        ),
        "10"
    );
    assert_eq!(
        computed(
            "state xs is client List of Whole starting [7, 2, 3]\n\
             state answer is client Text from text of \
             (10 * (valueOr with maybe is (xs at (0 + 0)), fallback is 0))\n"
        ),
        "70"
    );
}

/// `rest of`, `keys of` and `values of` are ZDeceptron folds rather than
/// primitives — they became ordinary functions when `append item to list`
/// gave the language a way to build a collection. A call groups.
#[test]
fn the_collection_folds_keep_their_operands_grouping() {
    assert_eq!(
        computed(
            "state xs is client List of Whole starting [1, 2, 3]\n\
             state answer is client Text from \
             text of (10 * (length of (rest of (append 4 to xs))))\n"
        ),
        "30"
    );
    let map = |op: &str| {
        format!(
            "state m is client Map of Text to Whole starting [\"a\" to 1, \"b\" to 2]\n\
             state answer is client Text from text of (10 * (length of ({op} of m)))\n"
        )
    };
    assert_eq!(computed(&map("keys")), "20");
    assert_eq!(computed(&map("values")), "20");
}

/// A record field read appends `.name` to its base, which is the same
/// shape as [`JsForm::Field`] and is emitted by the same rule. A record
/// built inline is the one base that is not already a name.
#[test]
fn a_field_read_keeps_its_bases_grouping() {
    assert_eq!(
        computed(
            "record P\n    n is Whole\n\
             state answer is client Text from text of (10 * (P with n is (1 + 2)).n)\n"
        ),
        "30"
    );
}

/// An ordinary call to a definition the bundle also carries — the
/// baseline the no-op forms are being compared against.
#[test]
fn an_ordinary_call_keeps_its_arguments_grouping() {
    assert_eq!(
        computed(
            "function twice with x\n    give x + x\n\
             state answer is client Text from text of (10 * (twice with x is (1 + 2)))\n"
        ),
        "60"
    );
    assert_eq!(
        computed(
            "state d is client Whole starting 4\n\
             state answer is client Text from text of (2 * (abs of (10 + (d * 3))))\n"
        ),
        "44"
    );
}

// --- a `static` read, which is inlined as a literal ------------------------

fn with_static(name: &str, json: &str, declarations: &str) -> Result<Bundle, String> {
    let mut statics = BTreeMap::new();
    statics.insert(name.to_string(), json.to_string());
    let source = format!("{declarations}view\n    Text answer\n");
    try_compile_with_statics(&source, "test.zd", statics).map_err(|e| e[0].message.clone())
}

/// §14C.3b: a `static` read *is* its value, inlined as the JSON the build
/// host printed. JSON's `-5` is not a JavaScript literal — it is unary
/// minus applied to `5` — so a bare one is a no-op emission with the same
/// hazard, and it had a live instance: `-n` for a `static Whole` of `-5`
/// inlined to `--5`, which is a decrement of a numeric literal and does
/// not parse. That is a build that succeeds and a page that is blank.
///
/// Asserted by **running** it, so a bundle that does not parse fails here
/// rather than in somebody's browser.
#[test]
fn a_negative_static_is_inlined_as_a_primary_expression() {
    let bundle = with_static(
        "n",
        "-5",
        "state n is static Whole starting 0\n\
         state m is client Whole from -n\n\
         state answer is client Text from text of m\n",
    )
    .expect("the program compiles");
    assert_eq!(
        rendered(&bundle),
        "5",
        "`-n` for a static of -5 is 5. `--5` does not parse at all."
    );
}

/// The arithmetic contexts a negative `static` was always reached from,
/// pinned so the parenthesisation above cannot be removed as cosmetic.
#[test]
fn a_negative_static_computes_the_same_in_every_operator() {
    for (expr, expected) in [
        ("text of (0 - n)", "5"),
        ("text of (2 * n)", "-10"),
        ("text of (0 - 0 - n)", "5"),
        ("text of (0 - (0 - n))", "-5"),
    ] {
        let bundle = with_static(
            "n",
            "-5",
            &format!(
                "state n is static Whole starting 0\n\
                 state answer is client Text from {expr}\n"
            ),
        )
        .expect("the program compiles");
        assert_eq!(rendered(&bundle), expected, "for `{expr}`");
    }
}
