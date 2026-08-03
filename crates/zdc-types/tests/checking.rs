//! What the type checker accepts, what it refuses, and what it says.

use zdc_types::{Type, TypeTable};

fn hir(src: &str) -> zdc_hir::Hir {
    let program = zdc_parser::parse(src).expect("the source must parse");
    zdc_resolve::Resolver::new(&program)
        .resolve()
        .expect("the source must resolve")
}

/// The placement pass's answers, from the placement pass. §17.1.4's
/// interface is checked against the real thing here, not against a stub.
fn placements(hir: &zdc_hir::Hir) -> zdc_graph::TierSplit {
    zdc_graph::split(hir)
}

fn accept(src: &str) -> TypeTable {
    let hir = hir(src);
    let split = placements(&hir);
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
    let split = placements(&hir);
    zdc_types::check(&hir, &split)
        .expect_err("expected this to be rejected")
        .into_iter()
        .map(|error| error.message)
        .collect()
}

/// The one message every rejection test reads, so a test that meant to
/// find one error does not silently pass on a different one.
fn only(src: &str) -> String {
    let mut errors = reject(src);
    assert_eq!(errors.len(), 1, "expected one error, got: {errors:?}");
    errors.remove(0)
}

// --- the four required rejections -----------------------------------------

#[test]
fn a_whole_where_text_is_expected_is_rejected() {
    let message = only(
        "state n is client Whole starting 1\n\
         state s is client Text from n\n",
    );
    assert!(message.contains("Whole"), "{message}");
    assert!(message.contains("Text"), "{message}");
    assert!(message.contains('s'), "{message}");
}

/// A literal has no written type, so the diagnostic says what it *is*
/// rather than picking one of the two numeric types arbitrarily — and it
/// blames the value, not the declaration that rejected it.
#[test]
fn a_number_literal_where_text_is_expected_blames_the_literal() {
    let message = only("state name is client Text starting 1\n");
    assert!(message.contains("a number"), "{message}");
    assert!(message.contains("`Text` is expected"), "{message}");
    assert!(
        !message.contains("`Text`, but it has to be"),
        "the declaration is not the mistake: {message}"
    );
}

#[test]
fn a_when_missing_an_arm_is_rejected_and_the_missing_arm_is_named() {
    let message = only(
        "state visits is durable Whole starting 0\n\
         view\n\
         \x20   when visits\n\
         \x20       Loading          show Spinner\n\
         \x20       Ready with total show Text total\n",
    );
    assert!(message.contains("`Failed`"), "{message}");
    assert!(message.contains("Remote of Whole"), "{message}");
}

#[test]
fn an_arm_for_a_variant_that_does_not_exist_is_rejected() {
    let message = only(
        "state visits is durable Whole starting 0\n\
         view\n\
         \x20   when visits\n\
         \x20       Loading          show Spinner\n\
         \x20       Ready with total show Text total\n\
         \x20       Failed with e    show Spinner\n\
         \x20       Some with v      show Spinner\n",
    );
    assert!(message.contains("`Some`"), "{message}");
    assert!(message.contains("Remote of Whole"), "{message}");
    assert!(
        message.contains("`Loading`") && message.contains("`Ready`"),
        "the message must list the variants that do exist: {message}"
    );
}

#[test]
fn a_pattern_binding_more_names_than_the_variant_has_is_rejected() {
    let message = only(
        "state visits is durable Whole starting 0\n\
         view\n\
         \x20   when visits\n\
         \x20       Loading                show Spinner\n\
         \x20       Ready with total       show Text total\n\
         \x20       Failed with why, moment  show Spinner\n",
    );
    assert!(message.contains("1 field"), "{message}");
    assert!(message.contains('2'), "{message}");
}

// --- what must typecheck --------------------------------------------------

#[test]
fn the_smallest_program_typechecks() {
    accept(
        "state name is client Text starting \"world\"\n\
         view\n\
         \x20   Column\n\
         \x20       Heading \"Hello\"\n\
         \x20       Input name, hint is \"your name\"\n\
         \x20       Text name\n",
    );
}

#[test]
fn derived_client_state_and_handlers_typecheck() {
    accept(
        "state count   is client Whole starting 0\n\
         state doubled is client Whole from count * 2\n\
         view\n\
         \x20   Column\n\
         \x20       Text count\n\
         \x20       Text doubled\n\
         \x20       Button \"plus one\"\n\
         \x20           on click\n\
         \x20               add 1 to count\n",
    );
}

/// A number is shown as text without a conversion, because a text node
/// takes any base type (§16.3.6).
#[test]
fn a_number_may_be_shown_as_text() {
    accept("state n is client Whole starting 1\nview\n    Text n\n");
}

#[test]
fn a_list_may_not_be_shown_as_text() {
    let message = only(
        "state xs is client List of Text starting empty\n\
         view\n\
         \x20   Text xs\n",
    );
    assert!(message.contains("List of Text"), "{message}");
}

// --- §14G.1.4, the read table ---------------------------------------------

#[test]
fn reading_durable_state_from_the_view_yields_remote() {
    let table = accept(
        "state visits is durable Whole starting 0\n\
         view\n\
         \x20   when visits\n\
         \x20       Loading           show Spinner\n\
         \x20       Failed with error show ErrorBar message is error.message\n\
         \x20       Ready with total  show Text total\n",
    );
    let (_, choice) = table.whens().next().expect("the `when` records its choice");
    assert_eq!(choice.described, "Remote of Whole");
}

/// The same signal, read from a server derivation the view roots, is
/// plain `T`: the client hands it over as an RPC argument.
#[test]
fn a_view_rooted_server_derivation_reads_client_state_directly() {
    accept(
        "state who is client Text starting \"\"\n\
         state greeting is server Text from greet with who\n\
         function greet with name\n\
         \x20   give \"hello \" + name\n",
    );
}

/// Reading a `server` signal from a `client` signal without eliminating
/// the variant is exactly what Rule 1 exists to stop.
#[test]
fn a_client_signal_may_not_read_server_state_as_a_plain_value() {
    let message = only(
        "state who is client Text starting \"\"\n\
         state greeting is server Text from greet with who\n\
         state shown is client Text from greeting\n\
         function greet with name\n\
         \x20   give \"hello \" + name\n",
    );
    assert!(message.contains("Remote of Text"), "{message}");
}

/// A write does not become `Remote`: `add 1 to visits` sends a number.
#[test]
fn writing_durable_state_from_a_handler_sends_the_plain_value() {
    accept(
        "state visits is durable Whole starting 0\n\
         view\n\
         \x20   Button \"sign\"\n\
         \x20       on click\n\
         \x20           add 1 to visits\n",
    );
}

// --- operators -------------------------------------------------------------

#[test]
fn plus_joins_two_numbers_or_two_texts_but_not_one_of_each() {
    accept("state a is client Text starting \"x\" + \"y\"\n");
    accept("state a is client Whole starting 1 + 2\n");
    let message = only("state a is client Text starting \"x\" + 1\n");
    assert!(message.contains('+'), "{message}");
}

#[test]
fn plus_refuses_a_collection() {
    let message = only(
        "state xs is client List of Text starting empty\n\
         state a is client Text from xs + \"y\"\n",
    );
    assert!(message.contains("List of Text"), "{message}");
}

#[test]
fn add_works_on_numbers_and_not_on_lists() {
    let message = only(
        "state xs is client List of Text starting empty\n\
         state draft is client Text starting \"\"\n\
         view\n\
         \x20   Button \"add\"\n\
         \x20       on click\n\
         \x20           add draft to xs\n",
    );
    assert!(message.contains("append"), "{message}");
}

#[test]
fn is_compares_two_values_of_one_type() {
    accept("state a is client Truth from 1 is 2\n");
    let message = only("state a is client Truth from 1 is \"two\"\n");
    assert!(
        message.contains("Text") && message.contains("Whole"),
        "{message}"
    );
}

#[test]
fn a_condition_must_be_a_truth() {
    let message = only(
        "state n is client Whole starting 1\n\
         function f\n\
         \x20   if n\n\
         \x20       give 1\n\
         \x20   give 2\n",
    );
    assert!(message.contains("Truth"), "{message}");
}

// --- collections -----------------------------------------------------------

/// §5.4: indexing is bounds-checked, so reading through `at` gives an
/// `Option of T` that has to be eliminated.
#[test]
fn reading_through_at_gives_an_option() {
    let message = only(
        "state scores is client Map of Text to Whole starting empty\n\
         state one is client Whole from scores at \"a\"\n",
    );
    assert!(message.contains("Option of Whole"), "{message}");
}

#[test]
fn an_option_from_an_index_is_eliminated_by_when() {
    accept(
        "state scores is client Map of Text to Whole starting empty\n\
         state one is client Whole from lookup with scores\n\
         function lookup with table\n\
         \x20   when table at \"a\"\n\
         \x20       Some with value\n\
         \x20           give value\n\
         \x20       None\n\
         \x20           give 0\n",
    );
}

#[test]
fn a_map_key_must_have_the_declared_key_type() {
    let message = only(
        "state scores is client Map of Text to Whole starting empty\n\
         state n is client Whole starting 1\n\
         state one is client Option of Whole from scores at n\n",
    );
    assert!(
        message.contains("Whole") && message.contains("Text"),
        "{message}"
    );
}

#[test]
fn a_list_is_indexed_by_position() {
    accept(
        "state xs is client List of Text starting empty\n\
         state head is client Option of Text from xs at 0\n",
    );
}

#[test]
fn empty_knows_which_collection_it_is() {
    let table = accept("state xs is client List of Text starting empty\n");
    let (_, kind) = table.empties().next().expect("`empty` records its kind");
    assert_eq!(kind, zdc_types::EmptyKind::List);

    let table = accept("state m is client Map of Text to Whole starting empty\n");
    let (_, kind) = table.empties().next().expect("`empty` records its kind");
    assert_eq!(kind, zdc_types::EmptyKind::Map);
}

#[test]
fn an_empty_with_nothing_to_say_which_collection_it_is_is_reported() {
    let message = only("function f\n    give empty\n");
    assert!(message.contains("empty"), "{message}");
}

// --- pipelines -------------------------------------------------------------

#[test]
fn a_pipeline_carries_its_element_type_through_every_clause() {
    accept(
        "state xs is client List of Whole starting empty\n\
         state top is client List of Whole from best with xs\n\
         function best with all\n\
         \x20   from all\n\
         \x20   keep each n where n > 0\n\
         \x20   sort each n by n\n\
         \x20   take first 5\n",
    );
}

#[test]
fn map_each_changes_the_element_type() {
    let message = only(
        "state xs is client List of Whole starting empty\n\
         state names is client List of Whole from labels with xs\n\
         function labels with all\n\
         \x20   from all\n\
         \x20   map each n to \"x\"\n",
    );
    assert!(message.contains("List of Text"), "{message}");
}

#[test]
fn a_pipeline_clause_without_a_from_is_reported() {
    let message = only("function f\n    keep each n where n is 1\n");
    assert!(message.contains("`from`"), "{message}");
}

// --- functions -------------------------------------------------------------

#[test]
fn a_function_must_give_a_value_on_every_path() {
    let message = only(
        "state flag is client Truth starting no\n\
         state n is client Whole from f with flag\n\
         function f with c\n\
         \x20   if c\n\
         \x20       give 1\n",
    );
    assert!(message.contains("give"), "{message}");
}

#[test]
fn both_halves_of_an_if_giving_is_enough() {
    accept(
        "state flag is client Truth starting no\n\
         state n is client Whole from f with flag\n\
         function f with c\n\
         \x20   if c\n\
         \x20       give 1\n\
         \x20   otherwise\n\
         \x20       give 2\n",
    );
}

/// Let-polymorphism: one function, two argument types, no annotation.
#[test]
fn a_function_may_be_used_at_two_types() {
    let table = accept(
        "state a is client Text  from same with \"x\"\n\
         state b is client Whole from same with 1\n\
         function same with x\n\
         \x20   give x\n",
    );
    let _ = table;
}

#[test]
fn an_argument_of_the_wrong_type_names_the_parameter() {
    let message = only(
        "state a is client Text from shout with 1\n\
         function shout with word\n\
         \x20   give word + \"!\"\n",
    );
    assert!(message.contains("word"), "{message}");
    assert!(message.contains("shout"), "{message}");
}

#[test]
fn a_named_argument_binds_to_the_parameter_of_that_name() {
    accept(
        "state a is client Text from shout with word is \"hi\"\n\
         function shout with word\n\
         \x20   give word + \"!\"\n",
    );
}

#[test]
fn a_missing_argument_names_the_parameter() {
    let message = only(
        "state a is client Text from shout\n\
         function shout with word\n\
         \x20   give word + \"!\"\n",
    );
    assert!(message.contains("shout"), "{message}");
}

// --- two-way binding (§14B.5) ---------------------------------------------

#[test]
fn an_input_binds_a_client_text_signal() {
    accept(
        "state name is client Text starting \"\"\n\
         view\n\
         \x20   Input name, hint is \"your name\"\n",
    );
}

#[test]
fn an_input_may_not_bind_durable_state() {
    let message = only(
        "state name is durable Text starting \"\"\n\
         view\n\
         \x20   Input name\n",
    );
    assert!(message.contains("durable"), "{message}");
}

#[test]
fn an_input_may_not_bind_a_derived_signal() {
    let message = only(
        "state raw is client Text starting \"\"\n\
         state trimmed is client Text from raw\n\
         view\n\
         \x20   Input trimmed\n",
    );
    assert!(message.contains("from"), "{message}");
}

#[test]
fn a_checkbox_binds_a_truth() {
    let message = only(
        "state name is client Text starting \"\"\n\
         view\n\
         \x20   Checkbox name\n",
    );
    assert!(
        message.contains("Truth") && message.contains("Text"),
        "{message}"
    );
}

#[test]
fn a_hint_is_text_and_a_padding_is_a_number() {
    let message = only(
        "state name is client Text starting \"\"\n\
         view\n\
         \x20   Input name, hint is 8\n",
    );
    assert!(message.contains("hint"), "{message}");

    let message = only("view\n    Row \"x\", padding is \"wide\"\n");
    assert!(message.contains("padding"), "{message}");
}

#[test]
fn an_error_bar_needs_its_message() {
    let message = only("view\n    ErrorBar\n");
    assert!(message.contains("message"), "{message}");
}

// --- reporting -------------------------------------------------------------

#[test]
fn every_error_is_reported_not_just_the_first() {
    let errors = reject(
        "state a is client Text  starting 1\n\
         state b is client Whole starting \"two\"\n\
         state c is client Truth starting 3\n",
    );
    assert_eq!(errors.len(), 3, "{errors:?}");
}

#[test]
fn one_mistake_produces_one_diagnostic() {
    // The bad operand poisons everything downstream; only the operand is
    // reported.
    let errors = reject("state a is client Whole from \"x\" - 1 + 2 * 3\n");
    assert_eq!(errors.len(), 1, "{errors:?}");
}

/// Spec §7.3, and the rule the resolver already keeps: nothing internal
/// leaks into a message a programmer reads.
#[test]
fn no_message_names_a_rust_type() {
    let sources = [
        "state a is client Text starting 1\n",
        "state xs is client List of Text starting empty\nstate b is client Text from xs + \"y\"\n",
        "state v is durable Whole starting 0\nview\n    when v\n        Loading show Spinner\n",
        "function f\n    keep each n where n is 1\n",
        "state m is client Map of Text to Whole starting empty\nstate n is client Whole from m at \"a\"\n",
        "record Todo\n    id is Whole\nstate one is client Todo starting Todo\n",
        "record Todo\n    id is Whole\nstate one is client Todo starting Todo with id is \"x\"\n",
        "choice S\n    A\n    B with why is Text\nstate s is client S starting B\n",
        "choice S\n    A\n    B with why is Text\nstate s is client S starting A\nfunction f with e\n    when e\n        A show 1\nstate n is client Whole from f with s\n",
        "state tags is client List of Text starting []\nview\n    Button \"go\"\n        on click\n            add 1 to tags\n",
        "state n is client Whole starting 0\nview\n    Button \"go\"\n        on click\n            append 1 to n\n",
        "state tags is client List of Text starting [\"a\", 1]\n",
        "choice S\n    A\nstate s is client S starting A\nstate t is client Text from s.why\n",
    ];
    let forbidden = [
        "TypeExpr",
        "HirExpr",
        "ExprId",
        "DefId",
        "LocalId",
        "Type::",
        "Constraint",
        "Mismatch",
        "Vec<",
        "Option<",
        "Some(",
        "None)",
        "TyVar",
        "HirStmt",
        "unwrap",
    ];

    // Every source has to *be* rejected, or the loop below reads no
    // messages and the test says nothing about any of them.
    let mut inspected = 0;
    for src in sources {
        let messages = reject(src);
        assert!(!messages.is_empty(), "{src:?} is no longer rejected");
        for message in messages {
            inspected += 1;
            for needle in forbidden {
                assert!(
                    !message.contains(needle),
                    "message for {src:?} leaked `{needle}`: {message}"
                );
            }
        }
    }
    assert!(inspected >= sources.len(), "read {inspected} messages");
}

// --- what codegen asks for (§16.7) ----------------------------------------

#[test]
fn the_table_records_the_type_of_every_expression() {
    let table = accept("state a is client Whole starting 1 + 2\n");
    let types: Vec<&Type> = table.expr_types().map(|(_, ty)| ty).collect();
    assert!(
        types.iter().all(|ty| ty.is_settled()),
        "every recorded type must be settled: {types:?}"
    );
    assert!(types.contains(&&Type::Whole), "{types:?}");
}

#[test]
fn the_table_records_whether_at_indexes_a_list_or_a_map() {
    let table = accept(
        "state m is client Map of Text to Whole starting empty\n\
         state v is client Option of Whole from m at \"a\"\n",
    );
    let (_, kind) = table
        .indexes()
        .next()
        .expect("the index records its container");
    assert_eq!(kind, zdc_types::IndexKind::Map);
}

#[test]
fn the_table_records_every_variants_field_arity() {
    let table = accept(
        "state visits is durable Whole starting 0\n\
         view\n\
         \x20   when visits\n\
         \x20       Loading           show Spinner\n\
         \x20       Failed with error show ErrorBar message is error.message\n\
         \x20       Ready with total  show Text total\n",
    );
    let (_, choice) = table.whens().next().expect("the `when` records its choice");
    let arities: Vec<usize> = choice
        .variants
        .iter()
        .map(|variant| variant.fields.len())
        .collect();
    assert_eq!(arities, [0, 1, 1], "`whenInto` needs the declared arity");
}

// --- record and choice declarations (spec §14B.1, §14G.1.2) ---------------

const TODO: &str = "record Todo\n\
                    \x20   id    is Whole\n\
                    \x20   title is Text\n\
                    \x20   done  is Truth\n";

/// A record is a nominal product type: its fields have the declared types
/// wherever the value came from.
#[test]
fn a_records_field_has_the_type_it_was_declared_with() {
    accept(&format!(
        "{TODO}\
         state one is client Todo starting Todo with id is 1, title is \"x\", done is no\n\
         state shown is client Text from one.title\n"
    ));
}

#[test]
fn a_record_field_of_the_wrong_type_is_reported_by_name() {
    let message = only(&format!(
        "{TODO}\
         state one is client Todo starting Todo with id is \"1\", title is \"x\", done is no\n"
    ));
    assert!(message.contains("`id` of `Todo`"), "{message}");
    assert!(message.contains("`Whole`"), "{message}");
}

/// Construction is by name, and every field is given a value: there is no
/// value in ZDeceptron that stands for nothing.
#[test]
fn a_record_missing_a_field_names_the_ones_left_out() {
    let message = only(&format!(
        "{TODO}state one is client Todo starting Todo with id is 1\n"
    ));
    assert!(message.contains("`title`"), "{message}");
    assert!(message.contains("`done`"), "{message}");
}

#[test]
fn a_field_a_record_does_not_declare_is_reported() {
    let message = only(&format!(
        "{TODO}\
         state one is client Todo starting Todo with id is 1, title is \"x\", done is no, \
         colour is \"red\"\n"
    ));
    assert!(message.contains("no field named `colour`"), "{message}");
    assert!(message.contains("`title`"), "{message}");
}

#[test]
fn reading_a_field_a_record_does_not_declare_is_reported() {
    let message = only(&format!(
        "{TODO}\
         state one is client Todo starting Todo with id is 1, title is \"x\", done is no\n\
         state shown is client Text from one.name\n"
    ));
    assert!(
        message.contains("`Todo` has no field named `name`"),
        "{message}"
    );
    assert!(message.contains("`title`"), "{message}");
}

/// §14G.1.2: elimination is by position over the declared fields, so a
/// pattern binds one fresh name per field.
#[test]
fn a_when_over_a_declared_choice_binds_its_fields_positionally() {
    accept(
        "choice Status\n\
         \x20   Active\n\
         \x20   Archived with reason is Text, moment is Whole\n\
         state s is client Status starting Archived with reason is \"old\", moment is 1\n\
         function describe with entry\n\
         \x20   when entry\n\
         \x20       Active show \"active\"\n\
         \x20       Archived with why, moment show why\n\
         state shown is client Text from describe with s\n",
    );
}

/// The acceptance criterion: a missing arm is a type error naming the
/// variant that was left out (§14G.1.6).
#[test]
fn a_when_over_a_declared_choice_must_write_every_arm() {
    let message = only(
        "choice Status\n\
         \x20   Active\n\
         \x20   Archived with reason is Text\n\
         state s is client Status starting Active\n\
         function describe with entry\n\
         \x20   when entry\n\
         \x20       Active show \"active\"\n\
         state shown is client Text from describe with s\n",
    );
    assert!(message.contains("`Archived`"), "{message}");
    assert!(message.contains("missing"), "{message}");
    assert!(message.contains("`Status`"), "{message}");
}

#[test]
fn a_variant_that_carries_fields_cannot_be_written_bare() {
    let message = only(
        "choice Status\n\
         \x20   Active\n\
         \x20   Archived with reason is Text\n\
         state s is client Status starting Archived\n",
    );
    assert!(message.contains("reason is"), "{message}");
}

#[test]
fn a_record_name_is_not_a_value_on_its_own() {
    let message = only(&format!("{TODO}state one is client Todo starting Todo\n"));
    assert!(message.contains("Todo with"), "{message}");
}

#[test]
fn a_choice_is_taken_apart_with_when_rather_than_read_from() {
    let message = only(
        "choice Status\n\
         \x20   Active\n\
         state s is client Status starting Active\n\
         state shown is client Text from s.reason\n",
    );
    assert!(message.contains("taken apart with `when`"), "{message}");
}

// --- collection literals (spec §14B.4) ------------------------------------

#[test]
fn the_literals_take_the_types_of_what_is_in_them() {
    accept(
        "state tags is client List of Text starting [\"red\", \"green\"]\n\
         state scores is client Map of Text to Whole starting [\"a\" to 1, \"b\" to 2]\n",
    );
}

#[test]
fn a_list_literal_of_mixed_types_is_reported() {
    let message = only("state tags is client List of Text starting [\"red\", 1]\n");
    assert!(message.contains("This list holds"), "{message}");
}

/// `[` cannot introduce two things at once, so the empty map keeps `empty`
/// and its written type.
#[test]
fn an_empty_bracket_pair_is_a_list_and_not_a_map() {
    let message = only("state scores is client Map of Text to Whole starting []\n");
    assert!(message.contains("List of"), "{message}");
}

// --- append and remove (spec §14B.2) --------------------------------------

#[test]
fn append_and_remove_work_on_the_elements_of_a_list() {
    accept(
        "state tags is client List of Text starting []\n\
         view\n\
         \x20   Column\n\
         \x20       Button \"add\"\n\
         \x20           on click\n\
         \x20               append \"red\" to tags\n\
         \x20       Button \"drop\"\n\
         \x20           on click\n\
         \x20               remove \"red\" from tags\n",
    );
}

#[test]
fn append_on_a_number_names_the_arithmetic_forms() {
    let message = only(
        "state count is client Whole starting 0\n\
         view\n\
         \x20   Button \"go\"\n\
         \x20       on click\n\
         \x20           append 1 to count\n",
    );
    assert!(message.contains("`add` and `subtract`"), "{message}");
}

#[test]
fn add_on_a_collection_names_the_membership_forms() {
    let message = only(
        "state tags is client List of Text starting []\n\
         view\n\
         \x20   Button \"go\"\n\
         \x20       on click\n\
         \x20           add 1 to tags\n",
    );
    assert!(message.contains("`append` and `remove`"), "{message}");
}

#[test]
fn appending_the_wrong_element_type_is_reported() {
    let message = only(
        "state tags is client List of Text starting []\n\
         view\n\
         \x20   Button \"go\"\n\
         \x20       on click\n\
         \x20           append 1 to tags\n",
    );
    assert!(
        message.contains("`append` works on the elements"),
        "{message}"
    );
}

/// A map entry cannot be added without saying where, so `append` has no
/// meaning on one; `remove` takes the key.
#[test]
fn append_to_a_map_names_the_form_that_does_work() {
    let message = only(
        "state scores is client Map of Text to Whole starting empty\n\
         view\n\
         \x20   Button \"go\"\n\
         \x20       on click\n\
         \x20           append 1 to scores\n",
    );
    assert!(message.contains("set … at"), "{message}");
}

#[test]
fn removing_from_a_map_takes_a_key() {
    accept(
        "state scores is client Map of Text to Whole starting empty\n\
         view\n\
         \x20   Button \"go\"\n\
         \x20       on click\n\
         \x20           remove \"a\" from scores\n",
    );
    let message = only(
        "state scores is client Map of Text to Whole starting empty\n\
         view\n\
         \x20   Button \"go\"\n\
         \x20       on click\n\
         \x20           remove 1 from scores\n",
    );
    assert!(message.contains("key of the entry"), "{message}");
}

// --- what `is` can compare (§16.7 item 2, §17.4.4) ------------------------

/// `===` answers value equality for a base type and *identity* for
/// everything else, and the runtime has no structural comparison to fall
/// back on. The comparison is refused here rather than at emission, so the
/// diagnostic points at the `is` the programmer wrote (spec §7.3).
///
/// It is also what lets a library function be polymorphic *and* compare
/// its elements: `listContains` gets `List of a` with `a` restricted to
/// what `is` can answer for, rather than a variable codegen could not
/// decide about at all.
#[test]
fn comparing_two_records_is_refused_rather_than_compared_by_identity() {
    let message = only(
        "record Point\n\
         \x20   x is Whole\n\
         state a is client Point starting Point with x is 1\n\
         state same is client Truth from a is a\n",
    );
    assert!(message.contains("compares by value"), "{message}");
    assert!(message.contains("`Point`"), "{message}");
}

#[test]
fn comparing_two_lists_is_refused_for_the_same_reason() {
    let message = only(
        "state xs is client List of Whole starting []\n\
         state same is client Truth from xs is xs\n",
    );
    assert!(message.contains("compares by value"), "{message}");
}

#[test]
fn every_base_type_is_still_comparable() {
    accept(
        "state a is client Truth from \"x\" is \"y\"\n\
         state b is client Truth from 1 is 2\n\
         state c is client Truth from 1.5 is 2.5\n\
         state d is client Truth from yes is no\n",
    );
}

// --- local bindings (§17.4.10) --------------------------------------------

/// A binding carries no annotation and needs none: the value's type is
/// the name's type, inferred like every other expression.
#[test]
fn a_binding_takes_its_type_from_its_value_with_no_annotation() {
    let table = accept(
        "function greet with name\n\
        \x20   with greeting is \"hello \" + name\n\
        \x20   give greeting\n\
         state who is client Text starting \"world\"\n\
         state message is client Text from greet with name is who\n",
    );
    let bound = table
        .expr_types()
        .find(|(_, ty)| matches!(ty, Type::Text))
        .map(|(_, ty)| ty.clone());
    assert_eq!(bound, Some(Type::Text));
}

/// Inference runs *through* a binding: nothing about `count` is written
/// down, and the `Whole` it must be comes back out of the function's
/// declared result.
#[test]
fn inference_flows_through_a_binding_in_both_directions() {
    accept(
        "function twice with n\n\
        \x20   with doubled is n + n\n\
        \x20   give doubled\n\
         state seed is client Whole starting 2\n\
         state answer is client Whole from twice with n is seed\n",
    );
}

/// And a binding used at the wrong type is refused at the use, not at the
/// binding: the binding never claimed anything.
#[test]
fn a_binding_used_at_the_wrong_type_is_rejected() {
    let message = only(
        "function f\n\
        \x20   with n is 1\n\
        \x20   give n\n\
         state s is client Text from f\n",
    );
    assert!(message.contains("Text"), "{message}");
}

// --- `append item to list`, the construction form --------------------------

/// The element's type and the list's element type are one type, inferred
/// in whichever direction the program writes them down.
#[test]
fn append_unifies_the_element_with_what_the_list_holds() {
    let source = "state xs is client List of Whole starting [1]\n\
                  state ys is client List of Whole from append 2 to xs\n";
    let table = accept(source);
    let hir = hir(source);
    let ys = hir
        .defs
        .iter()
        .find(|(_, def)| def.name == "ys")
        .map(|(id, _)| id)
        .expect("`ys` is declared");
    assert_eq!(table.def(ys), Some(&Type::list(Type::Whole)));
}

/// Inference runs the other way too: nothing says what `empty` is a list
/// of, and the element decides.
#[test]
fn the_element_decides_what_an_empty_list_holds() {
    accept("state ys is client List of Text from append \"a\" to empty\n");
}

/// An element of the wrong type is refused, and the message is about the
/// list rather than about the element, because the list is the operand
/// whose head constructor the form demands.
#[test]
fn appending_the_wrong_element_type_is_rejected() {
    let messages = reject(
        "state xs is client List of Whole starting [1]\n\
         state ys is client List of Whole from append \"a\" to xs\n",
    );
    assert!(
        messages[0].contains("The element `append` puts into this list is `Text`"),
        "{messages:?}"
    );
}

/// Only a list can be grown. A `Map` entry is a pair and this form names
/// one value, and a `Text` is not a collection the language can extend at
/// all, so both are refused rather than quietly dispatched.
#[test]
fn only_a_list_can_be_appended_to() {
    let messages = reject(
        "state m is client Map of Text to Whole starting [\"a\" to 1]\n\
         state n is client Map of Text to Whole from append 1 to m\n",
    );
    assert!(
        messages[0].contains("`append` grows a list, and this is `Map of Text to Whole`"),
        "{messages:?}"
    );

    let messages = reject(
        "state s is client Text starting \"a\"\n\
         state t is client Text from append \"b\" to s\n",
    );
    assert!(
        messages[0].contains("`append` grows a list, and this is `Text`"),
        "{messages:?}"
    );
}
