//! The standard library, compiled the way `zdc check` compiles it.
//!
//! §14F found that the language defined eight types and not one operation
//! on any of them. §14F.2 settled that the fix is written *in ZDeceptron*,
//! and §17.4.1 that the library is a compilation unit resolved into the
//! same arenas as the program. These tests are the acceptance criteria for
//! both claims, and they run the whole front end rather than inspecting
//! the prelude's syntax — a library that parses and does not typecheck is
//! not a library.

use zdc_types::{Type, TypeTable};

fn hir(src: &str) -> zdc_hir::Hir {
    let program = zdc_parser::parse(src).expect("the source must parse");
    zdc_resolve::Resolver::with_prelude(zdc_lib::load().program(), &program)
        .resolve()
        .unwrap_or_else(|errors| panic!("must resolve, got: {}", errors[0].message))
}

/// The placement pass's answers, from the placement pass. §17.1.4's
/// interface is checked against the real thing here, not against a stub.
fn accept(src: &str) -> TypeTable {
    let hir = hir(src);
    let split = zdc_graph::split(&hir);
    match zdc_types::check(&hir, &split) {
        Ok(table) => table,
        Err(errors) => {
            let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
            panic!("expected this to typecheck, got:\n{}", messages.join("\n"));
        }
    }
}

fn reject(src: &str) -> Vec<String> {
    let hir = hir(src);
    let split = zdc_graph::split(&hir);
    zdc_types::check(&hir, &split)
        .expect_err("expected this to be rejected")
        .into_iter()
        .map(|error| error.message)
        .collect()
}

fn only(src: &str) -> String {
    let mut errors = reject(src);
    assert_eq!(errors.len(), 1, "expected one error, got: {errors:?}");
    errors.remove(0)
}

/// The load-bearing one. Everything else here is downstream of the
/// library being a correct ZDeceptron program, and it is checked by the
/// compiler that compiles it, on every build, so it cannot drift from the
/// language.
#[test]
fn the_prelude_typechecks_on_its_own() {
    accept("");
}

/// §17.4.1 step 6, checked rather than assumed: no library definition is a
/// `state` or reaches one. It is what makes a library call unable to add
/// an edge to the signal graph, and therefore unable to change any
/// placement fact.
#[test]
fn no_library_definition_touches_a_signal() {
    let hir = hir("");
    // Counted: both assertions are inside the loop, so a `hir` with no
    // definitions at all — a prelude that failed to load — would pass
    // this while proving nothing.
    let mut scanned = 0;
    for (id, def) in hir.defs.iter() {
        scanned += 1;
        assert!(hir.is_prelude_def(id));
        assert!(
            !matches!(def.kind, zdc_hir::DefKind::Signal(_)),
            "`{}` is a signal, which would give the library a placement",
            def.name
        );
    }
    assert!(
        scanned > 17,
        "the prelude must contribute its primitives and the definitions written over them, \
         got {scanned}"
    );
}

// --- §14F's table of missing operations, one test per row ----------------

#[test]
fn text_operations_exist() {
    accept(
        "state s is client Text starting \"\"\n\
         state a is client Whole from length of s\n\
         state b is client Truth from s contains \"x\"\n\
         state c is client Text from uppercase of s\n\
         state d is client Text from lowercase of s\n\
         state e is client Text from trim of s\n\
         state f is client List of Text from split with value is s, using is \",\"\n\
         state g is client Truth from startsWith with value is s, prefix is \"a\"\n\
         state h is client Truth from endsWith with value is s, suffix is \"z\"\n\
         state i is client Truth from isBlank of s\n\
         state j is client Text from slice with value is s, start is 0, stop is 3\n\
         state k is client Option of Text from s at 0\n",
    );
}

/// The operations §14F's table did not reach, because they are what a
/// content site needs rather than what a type table lists: cutting at a
/// delimiter, stripping an affix, replacing, and the line separator the
/// lexer's string rule cannot contain.
#[test]
fn the_delimiter_operations_exist_and_have_the_types_they_claim() {
    accept(
        "state s is client Text starting \"\"\n\
         state a is client Text from before with value is s, delimiter is \"/\"\n\
         state b is client Text from after with value is s, delimiter is \"/\"\n\
         state c is client Text from beforeLast with value is s, delimiter is \"/\"\n\
         state d is client Text from afterLast with value is s, delimiter is \"/\"\n\
         state e is client Text from withoutPrefix with value is s, prefix is \"# \"\n\
         state f is client Text from withoutSuffix with value is s, suffix is \".md\"\n\
         state g is client Text from replace with value is s, old is \"&\", new is \"+\"\n\
         state h is client Option of Whole from indexOf with value is s, needle is \"x\"\n\
         state i is client List of Text from lines of s\n\
         state j is client Text from unlines of (lines of s)\n\
         state k is client Text from newline\n",
    );
}

#[test]
fn list_operations_exist() {
    accept(
        "state xs is client List of Text starting []\n\
         state a is client Whole from length of xs\n\
         state b is client Option of Text from first of xs\n\
         state c is client Option of Text from last of xs\n\
         state d is client List of Text from reverse of xs\n\
         state e is client Truth from xs contains \"a\"\n\
         state f is client Truth from isEmpty of xs\n\
         state g is client Text from join with parts is xs, using is \", \"\n\
         state h is client Whole from sumOf of [1, 2, 3]\n",
    );
}

#[test]
fn map_operations_exist() {
    accept(
        "state m is client Map of Text to Whole starting empty\n\
         state a is client List of Text from keys of m\n\
         state b is client List of Whole from values of m\n\
         state c is client Truth from m contains \"a\"\n\
         state d is client Whole from length of m\n\
         state e is client Whole from atOr with table is m, key is \"a\", fallback is 0\n",
    );
}

#[test]
fn numeric_helpers_exist() {
    accept(
        "state a is client Whole from min with first is 1, second is 2\n\
         state b is client Whole from max with first is 1, second is 2\n\
         state c is client Whole from abs of 0 - 3\n\
         state d is client Whole from clamp with value is 9, low is 0, high is 5\n\
         state e is client Option of Whole from floor of 1.5\n\
         state f is client Option of Whole from round of 1.5\n\
         state g is client Decimal from decimalOf of 2\n\
         state h is client Text from text of 2\n",
    );
}

/// §14F.2a, closed. `Option` could only be eliminated by `when`, which is
/// a statement, so nothing could unwrap one inside an expression — the
/// implementer's verdict was "no program can read a map and use the
/// result". `valueOr` is that elimination, and it is an ordinary function.
#[test]
fn an_option_can_be_eliminated_in_expression_position() {
    accept(
        "state table is client Map of Text to Whole starting empty\n\
         state score is client Whole from valueOr with maybe is (table at \"a\"), fallback is 0\n\
         state known is client Truth from isSome of (table at \"a\")\n\
         state absent is client Truth from isNone of (table at \"a\")\n",
    );
}

/// **Two `Remote of T`s combine into one, and the caller writes three
/// arms rather than nine** (#20).
///
/// §17.7 recorded this as having no answer — "without records-in-the-
/// library or tuples, `bothOf` has no return type to give". A pair is
/// what it was waiting for, and `zip`'s comment in `list.zd` says the two
/// walls are one issue rather than two. `zip` got past it first.
///
/// The type is the whole test: `Remote of Pair of A to B` is what makes
/// the combinator expressible, and `.first` and `.second` are how the
/// caller reads it back out.
#[test]
fn two_remotes_combine_into_a_remote_pair() {
    accept(
        "state count is server Whole from 1\n\
         state label is server Text from \"x\"\n\
         state both is client Remote of Pair of Whole to Text \
         from bothOf with left is count, right is label\n\
         state pair is client Pair of Whole to Text \
         from readyOr with result is both, \
         fallback is (Pair with first is 0, second is \"\")\n\
         state shown is client Text from pair.second\n",
    );
}

#[test]
fn a_remote_can_be_eliminated_in_expression_position() {
    accept(
        "state loaded is server Text from \"x\"\n\
         state shown is client Text from readyOr with result is loaded, fallback is \"…\"\n\
         state ready is client Truth from isReady of loaded\n",
    );
}

// --- the properties that make the library usable -------------------------

/// §17.4.4: a variable carrying a built-in operand constraint has to be
/// generalisable, or `min` is pinned by its first call and unusable on the
/// other numeric type. Both uses are in one program deliberately.
#[test]
fn a_numeric_helper_works_on_both_numeric_types_in_one_program() {
    let types = accept(
        "state a is client Whole from min with first is 1, second is 2\n\
         state b is client Decimal from min with first is 1.5, second is 2.5\n",
    );
    let whole = types
        .expr_types()
        .find(|(_, ty)| **ty == Type::Whole)
        .is_some();
    assert!(whole, "the `Whole` use must stay `Whole`");
}

/// The same property over an ordinary type variable: `valueOr` is used at
/// two different payloads in one program.
#[test]
fn option_elimination_is_polymorphic() {
    accept(
        "state names is client Map of Text to Text starting empty\n\
         state ages is client Map of Text to Whole starting empty\n\
         state name is client Text from valueOr with maybe is (names at \"a\"), fallback is \"\"\n\
         state age is client Whole from valueOr with maybe is (ages at \"a\"), fallback is 0\n",
    );
}

/// §17.4.3's dispatched set is chosen by the head constructor of the
/// principal operand, and the wrong one is an error rather than a silent
/// coercion.
#[test]
fn length_of_refuses_something_it_cannot_count() {
    let message = only(
        "state n is client Whole starting 1\n\
         state c is client Whole from length of n\n",
    );
    assert!(message.contains("`length of`"), "{message}");
    assert!(message.contains("`Whole`"), "{message}");
}

#[test]
fn contains_refuses_something_it_cannot_look_inside() {
    let message = only(
        "state n is client Whole starting 1\n\
         state c is client Truth from n contains 1\n",
    );
    assert!(message.contains("`contains`"), "{message}");
}

/// The dispatch is over the *container*, and the value has to match what
/// that container holds.
#[test]
fn contains_checks_the_value_against_what_the_container_holds() {
    let message = only(
        "state xs is client List of Text starting []\n\
         state c is client Truth from xs contains 1\n",
    );
    assert!(message.contains("`contains`"), "{message}");
}

/// §17.4.3 puts `Text` in the `at` row, which §5.4's bounds check then
/// applies to it like any other sequence.
#[test]
fn indexing_a_text_is_bounds_checked_like_everything_else() {
    let message = only(
        "state s is client Text starting \"\"\n\
         state c is client Text from s at 0\n",
    );
    assert!(message.contains("Option of Text"), "{message}");
}

/// §14G.7.7 rule 1: a user declaration that shadows a library one is a
/// redeclaration error, and the message says which library name it means
/// rather than pointing at a line the programmer cannot see (§7.3).
#[test]
fn a_program_may_not_redeclare_a_library_name() {
    let program =
        zdc_parser::parse("function join with parts, using\n    give \"\"\n").expect("parses");
    let errors = zdc_resolve::Resolver::with_prelude(zdc_lib::load().program(), &program)
        .resolve()
        .expect_err("expected a redeclaration error");
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0].message.contains("standard-library"),
        "got: {}",
        errors[0].message
    );
    assert!(
        errors[0].message.contains("`join`"),
        "got: {}",
        errors[0].message
    );
}

/// §7.3: a type error in a call to a library function points at the
/// argument the programmer wrote, not into a file they have never seen.
#[test]
fn a_type_error_in_a_library_call_points_at_the_users_own_argument() {
    let src = "state n is client Whole from length of 1\n";
    let hir = hir(src);
    let split = zdc_graph::split(&hir);
    let errors = zdc_types::check(&hir, &split).expect_err("expected an error");
    assert_eq!(errors.len(), 1);
    let span = errors[0].span;
    assert!(
        (span.start as usize) < src.len() && (span.end as usize) <= src.len(),
        "the span must address this file, got {span:?} against {} bytes",
        src.len()
    );
    assert_eq!(&src[span.start as usize..span.end as usize], "length of 1");
}

/// And a call with the wrong argument *name* names the library function's
/// own parameters, which is the whole reason a `foreign` binds them.
#[test]
fn a_library_call_names_the_parameters_it_actually_has() {
    let messages = reject(
        "state xs is client List of Text starting []\n\
         state j is client Text from join with parts is xs, separator is \",\"\n",
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("no parameter named `separator`") && m.contains("`using`")),
        "{messages:?}"
    );
}

// --- §4.2 of the 2026-08-03 re-measurement: the numeric half ------------

/// The soundness fix, at the type level.
///
/// `Whole / Whole` used to be `Whole`, and emitted JavaScript `/`, so a
/// signal whose type said integer held `2.3333333333333335`. `/` now gives
/// a `Decimal` whatever it divides, which is the only answer that does not
/// give one spelling two meanings (§14B.2) and the only one that does not
/// depend on which unification happens to resolve the operands first.
#[test]
fn division_gives_a_decimal_whatever_it_divides() {
    accept(
        "state a is client Decimal from 7 / 3\n\
         state b is client Decimal from 7.5 / 2.5\n\
         state c is client Decimal from 8 / 2\n",
    );
}

/// The old behaviour, now impossible. `8 / 2` is exactly 4 and it is still
/// not a `Whole`: if the rule bent for the cases that happen to divide
/// evenly it would be a rule about values, and the checker does not have
/// the values.
#[test]
fn a_quotient_may_not_be_stored_in_a_whole() {
    let message = only("state q is client Whole from 7 / 3\n");
    assert!(message.contains("`Decimal`"), "{message}");
    assert!(message.contains("`Whole`"), "{message}");

    let exact = only("state q is client Whole from 8 / 2\n");
    assert!(exact.contains("`Decimal`"), "{exact}");
}

/// §7.3: being refused is half the job, and the other half is being told
/// the one spelling that works.
#[test]
fn refusing_a_quotient_names_the_integer_division_that_works() {
    let hir = hir("state q is client Whole from 7 / 3\n");
    let split = zdc_graph::split(&hir);
    let errors = zdc_types::check(&hir, &split).expect_err("expected an error");
    let help = errors[0].help.as_deref().unwrap_or("");
    assert!(help.contains("quotient"), "{help}");
    assert!(help.contains("mod"), "{help}");
    assert!(help.contains("floor of"), "{help}");
}

/// Integer division exists, and what it gives is an `Option of Whole`.
///
/// §14A.3 rules that a `Whole` is a finite integral f64, so the
/// `Decimal`-to-`Whole` narrowing in `floor of` is partial and these two
/// inherit its `Option`. `valueOr` is the elimination, in expression
/// position, so the ergonomic cost is one call rather than the `when`
/// §14F.2a was worried about.
#[test]
fn integer_division_and_its_remainder_give_an_option_of_whole() {
    accept(
        "state a is client Option of Whole from quotient with value is 7, divisor is 3\n\
         state b is client Option of Whole from mod with value is 7, divisor is 3\n\
         state c is client Whole from valueOr with maybe is \
         (quotient with value is 7, divisor is 3), fallback is 0\n",
    );
}

/// The zero divisor, at the type level. A `Whole` cannot be written from a
/// narrowing without eliminating the `Option`, which is what makes
/// §14A.3's ruling a property of the checker rather than a convention.
#[test]
fn a_narrowing_may_not_be_stored_in_a_whole() {
    for source in [
        "state q is client Whole from floor of (1 / 0)\n",
        "state q is client Whole from round of 1.5\n",
        "state q is client Whole from quotient with value is 1, divisor is 0\n",
        "state q is client Whole from mod with value is 1, divisor is 0\n",
        // The reported case, refused rather than silently `NaN`: a game
        // drawing an empty cell from a full board passes `bound is 0`,
        // and the old library handed back a `NaN` typed `Whole` that
        // skipped the spawn with no diagnostic at all.
        "state cell is client Whole starting 0\n\
         state pick is client Whole from randomBelow with seed is cell, bound is 0\n",
    ] {
        let message = only(source);
        assert!(message.contains("`Option of Whole`"), "{message}");
        assert!(message.contains("`Whole`"), "{message}");
    }
}

#[test]
fn the_bitwise_window_exists_and_is_whole_to_whole() {
    accept(
        "state a is client Whole from bitAnd with left is 12, right is 10\n\
         state b is client Whole from bitOr with left is 12, right is 10\n\
         state c is client Whole from bitXor with left is 12, right is 10\n\
         state d is client Whole from shiftLeft with value is 1, places is 4\n\
         state e is client Whole from shiftRight with value is 256, places is 4\n\
         state f is client Whole from wrappingProduct with left is 65535, right is 65535\n\
         state g is client Whole from toUnsigned32 of 0 - 1\n",
    );
}

/// The generator is ordinary ZDeceptron and its seed is an ordinary
/// `Whole`, so there is no new placement rule and no new primitive to
/// keep `static` honest about.
#[test]
fn the_seeded_generator_is_ordinary_zdeceptron() {
    accept(
        "state seed is client Whole starting 12345\n\
         state next is client Whole from nextSeed of seed\n\
         state bits is client Whole from randomBits of seed\n\
         state roll is client Whole from valueOr with maybe is \
         (randomBelow with seed is seed, bound is 6), fallback is 0\n\
         state unit is client Decimal from randomDecimal of seed\n",
    );
}

/// And it is computable at build time, which is the question §17.4.7 and
/// §17.4.8 actually care about: a `static` value that differs between two
/// builds is what would break "a build that fails, fails everywhere", and
/// a pure function of a literal seed cannot.
#[test]
fn a_seeded_draw_is_available_in_static_placement() {
    accept(
        "state seed is static Whole starting 12345\n\
         state pick is static Whole from valueOr with maybe is \
         (randomBelow with seed is seed, bound is 100), fallback is 0\n",
    );
}
