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

/// **Known defect, unfixed.** A pipeline run is emitted as one block
/// ending in `return $p;` (`crates/zdc-codegen/src/stmt.rs`), and
/// `Statements::block` carries on emitting whatever follows it. A body
/// that ends `from … / keep … / give [99]` therefore emits
/// `return $p; return [99];`, and the answer the program computes is the
/// pipeline's — the `give` the programmer wrote is unreachable and
/// silent. Nothing refuses it: the checker takes `flow.pipeline` as the
/// body's result and stops asking.
///
/// This demonstrates a wrong answer rather than a crash, which is why it
/// is worth a name: the two spellings of "what this function gives" are
/// both written, both typecheck, and only one runs.
///
/// Left failing rather than fixed because which one is *meant* is a
/// language question. Refusing the body outright — a pipeline is the
/// whole of it or none of it — is the reading this test asserts; letting
/// a `give` after a pipeline win instead is a coherent alternative that
/// would need `block` to stop emitting the run's own `return`.
#[test]
#[ignore = "known defect: a `give` after a pipeline run typechecks and is emitted as unreachable code"]
fn a_give_after_a_pipeline_run_is_refused() {
    let message = only(
        "state xs is client List of Whole starting [3, 1, 2]\n\
         state ys is client List of Whole from f with xs\n\
         function f with all\n\
         \x20   from all\n\
         \x20   keep each x where x > 1\n\
         \x20   give [99]\n\
         view\n\
         \x20   Column\n\
         \x20       each y in ys\n\
         \x20           Text y\n",
    );
    assert!(message.contains("pipeline"), "{message}");
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

/// **Known defect, unfixed.** A component's own `state` is a `client`
/// source signal (§14D.1) and every other construct treats it as one —
/// `Text local` reads it, `set local to …` in a handler writes it, and
/// `zdc-codegen` allocates the pair `const [local, setLocal] = signal('')`
/// for it. `Input local` alone is refused, because `check_two_way`
/// (`crates/zdc-types/src/infer.rs`) matches `Res::Def` and a component's
/// state resolves to `Res::Local`. `zdc-codegen`'s `two_way`
/// (`crates/zdc-codegen/src/view.rs`) has the same shape, so both halves
/// need the local arm.
///
/// This demonstrates that the two-way rule reached the definition path
/// and not its sibling, the component-instance path: two `Field`s on one
/// page cannot each have their own text box.
///
/// Left failing rather than fixed because §14B.5 is written in terms of a
/// `state` *signal* and says nothing about a component's per-instance
/// cell; admitting it decides that the two are the same thing for the
/// purpose of writing back, which is a language decision rather than a
/// missing match arm.
#[test]
#[ignore = "known defect: `Input` cannot bind a component's own `state`, though a handler can write it"]
fn an_input_binds_a_components_own_state() {
    accept(
        "component Field with hintText\n\
         \x20   state local is client Text starting \"\"\n\
         \n\
         \x20   Column\n\
         \x20       Input local, hint is hintText\n\
         \x20       Text local\n\
         \n\
         view\n\
         \x20   Column\n\
         \x20       Field \"first\"\n\
         \x20       Field \"second\"\n",
    );
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

// --- `Code`, the built-in choice a `Failed` payload's `code` field has ----

/// A view whose `Failed` arm takes `error.code` apart with the arms
/// given, so each test below writes only the part it is about.
fn with_code_arms(arms: &str) -> String {
    format!(
        "state visits is durable Whole starting 0\n\
         view\n\
         \x20   when visits\n\
         \x20       Loading show Spinner\n\
         \x20       Failed with error\n\
         \x20           when error.code\n\
         {arms}\
         \x20       Ready with total show Text total\n"
    )
}

fn arm(name: &str) -> String {
    format!("\x20               {name} show ErrorBar message is \"{name}\"\n")
}

/// `error.code` is `Code`, and `when` eliminates it the way it eliminates
/// `Remote`. The choice the `when` records is the built-in one, so the
/// checker is dispatching on a type rather than on a string.
#[test]
fn a_when_over_a_failure_code_eliminates_the_builtin_choice() {
    let src = with_code_arms(&format!(
        "{}{}{}",
        arm("Unreachable"),
        arm("Timeout"),
        arm("Rejected")
    ));
    let table = accept(&src);
    let described: Vec<&str> = table
        .whens()
        .map(|(_, choice)| choice.described.as_str())
        .collect();
    assert!(
        described.contains(&"Code"),
        "no `when` eliminated `Code`: {described:?}"
    );
}

/// **The acceptance criterion.** All three arms, exactly as §14G.1.6
/// requires all three `Remote` arms — and each one is required
/// separately, so a program cannot cover two and call the third
/// unreachable.
#[test]
fn a_when_over_a_failure_code_requires_every_arm() {
    let mut checked = 0;
    for missing in ["Unreachable", "Timeout", "Rejected"] {
        let written: String = ["Unreachable", "Timeout", "Rejected"]
            .iter()
            .filter(|name| **name != missing)
            .map(|name| arm(name))
            .collect();
        let message = only(&with_code_arms(&written));
        assert!(
            message.contains("`Code` is missing"),
            "a `when` on `Code` missing `{missing}` was accepted or misreported: {message}"
        );
        assert!(
            message.contains(&format!("`{missing}`")),
            "the diagnostic must name the arm that is missing: {message}"
        );
        assert!(message.contains("Every arm must be written"), "{message}");
        checked += 1;
    }
    assert_eq!(checked, 3, "an arm was skipped");
}

/// There is no catch-all arm to fall back on, for `Code` or for anything
/// else: a `when` arm is `IDENT ["with" IDENT,…]` and the grammar has no
/// production for a wildcard. So an arm named `Otherwise` is simply a
/// variant name nothing declares, which is what the resolver says.
#[test]
fn there_is_no_catch_all_arm_to_write_instead() {
    let src = with_code_arms(&format!("{}{}", arm("Unreachable"), arm("Otherwise")));
    let program = zdc_parser::parse(&src).expect("the source must parse");
    let errors = zdc_resolve::Resolver::new(&program)
        .resolve()
        .expect_err("`Otherwise` names no variant");
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("`Otherwise` is not a variant name")),
        "{errors:?}"
    );
}

/// The hole this closed. `code` was `Text`, so a comparison against a
/// misspelled string was a well-typed expression that answered `no` for
/// ever. It is now a type error, and one that names both types.
#[test]
fn comparing_a_failure_code_against_text_is_refused() {
    let src = "state visits is durable Whole starting 0\n\
               state offline is client Truth starting no\n\
               view\n\
               \x20   when visits\n\
               \x20       Loading show Spinner\n\
               \x20       Failed with error show ErrorBar message is error.code\n\
               \x20       Ready with total show Text total\n";
    let messages = reject(src);
    assert!(
        messages.iter().any(|m| m.contains("Code")),
        "rendering a `Code` where `Text` is wanted must name the type: {messages:?}"
    );
}

/// `is` is not a second way to take a `Code` apart. It compares by value,
/// which the runtime can only do for a base type, so `when` stays the one
/// elimination form (§4.1: one phrasing per construct).
#[test]
fn a_failure_code_is_not_compared_with_is() {
    let src = "state visits is durable Whole starting 0\n\
               view\n\
               \x20   when visits\n\
               \x20       Loading show Spinner\n\
               \x20       Failed with error\n\
               \x20           if error.code is Timeout\n\
               \x20               Text \"slow\"\n\
               \x20           otherwise\n\
               \x20               Text \"other\"\n\
               \x20       Ready with total show Text total\n";
    let messages = reject(src);
    assert!(
        messages.iter().any(|m| m.contains("compares by value")),
        "{messages:?}"
    );
}

/// **A `Code` unifies with a `Code`.**
///
/// `unify`'s scalar arm listed `Text`, `Markup`, `Whole`, `Decimal`,
/// `Truth` and `Error` but not `Code`, so two of them fell to the `Shape`
/// wildcard and the checker refused a type against itself:
///
/// ```text
/// This list holds `Code`, but `Code` is expected here.
/// ```
///
/// A message naming one type twice is the shape of this bug, and no gate
/// could have caught it: `Type` is not in `check-wildcard-arms.sh`'s
/// guarded set, so the wildcard that swallowed it is legal.
///
/// The list is the smallest way to make two *already concrete* `Code`s
/// meet. A parameter would not do it — that unifies a variable with a
/// concrete type, which the arm above already handles, so the bug hides.
#[test]
fn two_code_values_unify_with_each_other() {
    accept(concat!(
        "state visits is durable Whole starting 0\n",
        "view\n",
        "    Column\n",
        "        when visits\n",
        "            Loading\n",
        "                Text \"...\"\n",
        "            Failed with error\n",
        "                each c in [error.code, error.code]\n",
        "                    Text \"x\"\n",
        "            Ready with n\n",
        "                Text n\n",
    ));
}

// --- handles (spec §14E.1, as `Handle` amends it) --------------------

/// A handle is opaque, so nothing is interchangeable with it in either
/// direction: it is not `Text`, and `Text` is not it.
#[test]
fn a_handle_is_not_any_other_type() {
    let errors = reject(
        "foreign make is client\n\
         \x20   from \"./m.js\" as \"F\"\n\
         \x20   gives new Handle\n\
         state name is client Text from make\n\
         view\n\
         \x20   Column\n\
         \x20       Text name\n",
    );
    assert!(
        errors.iter().any(|e| e.contains("Handle")),
        "a handle was accepted where `Text` was declared: {errors:?}"
    );
}

/// `new` on a class hands back a host object, so `Handle` is the only
/// result a constructing foreign can have.
#[test]
fn a_constructing_foreign_gives_a_handle_and_nothing_else() {
    let errors = reject(
        "foreign make is client\n\
         \x20   from \"./m.js\" as \"F\"\n\
         \x20   gives new Text\n\
         state name is client Text from make\n\
         view\n\
         \x20   Column\n\
         \x20       Text name\n",
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("`new` builds a host object")),
        "`gives new Text` was accepted: {errors:?}"
    );
}

/// A handle has nothing to show, so it cannot be a view element's text.
#[test]
fn a_handle_cannot_be_shown() {
    let errors = reject(
        "foreign make is client\n\
         \x20   from \"./m.js\" as \"F\"\n\
         \x20   gives new Handle\n\
         view\n\
         \x20   Column\n\
         \x20       Text make\n",
    );
    assert!(
        errors.iter().any(|e| e.contains("Handle")),
        "a handle reached a text node: {errors:?}"
    );
}

/// The shape the feature exists for typechecks: construct one, hand it to
/// another foreign, and take back a value the language understands.
#[test]
fn a_handle_may_be_made_and_handed_on() {
    accept(
        "foreign vector is client\n\
         \x20   from \"./three.module.js\" as \"Vector3\"\n\
         \x20   takes x is Decimal, y is Decimal, z is Decimal\n\
         \x20   gives new Handle\n\
         foreign lengthOf is client\n\
         \x20   from \"./three.module.js\" as \"Vector3\"\n\
         \x20   takes v is Handle\n\
         \x20   gives Decimal\n\
         state size is client Decimal from lengthOf with v is (vector with x is 3, y is 4, z is 0)\n\
         view\n\
         \x20   Column\n\
         \x20       Text size\n",
    );
}

/// A method's receiver is its first parameter, so a call to one is
/// checked exactly as any other call is — including the receiver's type.
#[test]
fn a_method_checks_its_receiver_like_any_other_argument() {
    let errors = reject(
        "foreign sizeOf is client\n\
         \x20   on Handle as \"size\"\n\
         \x20   takes of v is Handle\n\
         \x20   gives Whole\n\
         state n is client Whole from sizeOf of \"not a handle\"\n\
         view\n\
         \x20   Column\n\
         \x20       Text n\n",
    );
    assert!(
        errors.iter().any(|e| e.contains("Handle")),
        "a `Text` was accepted as a receiver: {errors:?}"
    );
}

/// The whole shape stage 2 exists for: construct, call a method, chain
/// the handle it returns into another, and take a value back.
#[test]
fn a_method_chain_over_handles_typechecks() {
    accept(
        "foreign vector is client\n\
         \x20   from \"./three.module.js\" as \"Vector3\"\n\
         \x20   takes x is Decimal, y is Decimal, z is Decimal\n\
         \x20   gives new Handle\n\
         foreign plus is client\n\
         \x20   on Handle as \"add\"\n\
         \x20   takes target is Handle, other is Handle\n\
         \x20   gives Handle\n\
         foreign lengthOf is client\n\
         \x20   on Handle as \"length\"\n\
         \x20   takes of v is Handle\n\
         \x20   gives Decimal\n\
         state size is client Decimal from lengthOf of (plus with target is (vector with x is 1, y is 2, z is 2), other is (vector with x is 2, y is 4, z is 4))\n\
         view\n\
         \x20   Column\n\
         \x20       Text size\n",
    );
}

// --- an absent result, and the statement that runs one -----------------

/// `gives nothing` types a call as `Nothing`, and `Nothing` goes nowhere.
///
/// The claim this pins is the *negative* one: every position in the
/// language that holds a value refuses it, so a `foreign` declared to hand
/// nothing back cannot be read as if it did. Without it `gives nothing`
/// would be a comment, and `undefined` would flow wherever the program
/// wrote the call.
#[test]
fn a_call_that_gives_nothing_is_not_a_value() {
    let errors = reject(
        "foreign draw is client\n\
         \x20   from \"./m.js\" as \"draw\"\n\
         \x20   takes n is Whole\n\
         \x20   gives nothing\n\
         state shown is client Whole from draw with n is 1\n\
         view\n\
         \x20   Column\n\
         \x20       Text shown\n",
    );
    assert!(
        errors.iter().any(|e| e.contains("nothing")),
        "a call that gives nothing was read as a value: {errors:?}"
    );
}

/// The same in the other direction: `do` admits `Nothing` and nothing
/// else, so it is the position an effect goes and not a way to throw a
/// result away.
#[test]
fn do_refuses_a_call_that_gives_a_value() {
    let errors = reject(
        "foreign twice is anywhere\n\
         \x20   from \"./m.js\" as \"twice\"\n\
         \x20   takes n is Whole\n\
         \x20   gives Whole\n\
         state n is client Whole starting 0\n\
         view\n\
         \x20   Column\n\
         \x20       Button \"go\"\n\
         \x20           on click\n\
         \x20               do twice with n is 1\n\
         \x20       Text n\n",
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("runs a call for its effect")),
        "`do` discarded a result: {errors:?}"
    );
}

/// A generic parameter is the quiet way `Nothing` would have escaped: a
/// `foreign` whose `takes` line names a type variable will accept whatever
/// it is offered, and `Constraint::Any` is the set that would have let an
/// absent value in. It does not.
#[test]
fn nothing_cannot_be_passed_to_a_generic_parameter() {
    let errors = reject(
        "foreign draw is client\n\
         \x20   from \"./m.js\" as \"draw\"\n\
         \x20   gives nothing\n\
         foreign echo is anywhere\n\
         \x20   from \"./m.js\" as \"echo\"\n\
         \x20   takes value is item\n\
         \x20   gives item\n\
         state shown is client Whole from echo with value is draw\n\
         view\n\
         \x20   Column\n\
         \x20       Text shown\n",
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("`value` of `echo` is `nothing`")),
        "an absent value was passed as a generic argument: {errors:?}"
    );
}

/// The whole shape blocker 2 exists for: a method that hands nothing back,
/// called for what it does.
#[test]
fn an_effect_on_a_handle_typechecks() {
    accept(
        "foreign scene is client\n\
         \x20   from \"./three.module.js\" as \"Scene\"\n\
         \x20   gives new Handle\n\
         foreign mesh is client\n\
         \x20   from \"./three.module.js\" as \"Mesh\"\n\
         \x20   gives new Handle\n\
         foreign addTo is client\n\
         \x20   on Handle as \"add\"\n\
         \x20   takes parent is Handle, child is Handle\n\
         \x20   gives nothing\n\
         state n is client Whole starting 0\n\
         view\n\
         \x20   Column\n\
         \x20       Button \"grow\"\n\
         \x20           on click\n\
         \x20               do addTo with parent is scene, child is mesh\n\
         \x20               add 1 to n\n\
         \x20       Text n\n",
    );
}

// --- the clock (#19) ------------------------------------------------------

/// **A clock's type is the compiler's, not the program's.**
///
/// Nothing in the source produces the value, so there is no expression to
/// unify the annotation with — and leaving it to the resting `0` would not
/// do, because an integer literal unifies happily with `Whole` and a
/// `Whole` cell holding `16.67` is a lie the type system had signed off on.
#[test]
fn a_clock_signal_must_be_declared_with_the_type_its_clause_gives() {
    accept(
        "state elapsed is client Decimal every \"250ms\"\n\
         state motion is client Decimal every frame\n\
         state ready is client Truth after \"2s\"\n\
         view\n\
         \x20   Column\n\
         \x20       Text elapsed\n\
         \x20       Text motion\n\
         \x20       Text ready\n",
    );

    let message = only(
        "state elapsed is client Whole every \"250ms\"\n\
         view\n\
         \x20   Column\n\
         \x20       Text elapsed\n",
    );
    assert!(message.contains("`Whole`"), "{message}");
    assert!(message.contains("`Decimal`"), "{message}");
    assert!(message.contains("every \"250ms\""), "{message}");

    let message = only(
        "state ready is client Decimal after \"2s\"\n\
         view\n\
         \x20   Column\n\
         \x20       Text ready\n",
    );
    assert!(message.contains("`Truth`"), "{message}");
    assert!(message.contains("after \"2s\""), "{message}");
}

/// **Nothing in the program may write a cell the clock owns**, and the
/// refusal names the clause rather than saying only that the write is
/// illegal. This is what keeps the construct from being an escape hatch: a
/// tick cannot cause anything the declaration does not already say.
#[test]
fn a_clock_signal_refuses_every_write() {
    let message = only(
        "state elapsed is client Decimal every \"250ms\"\n\
         view\n\
         \x20   Column\n\
         \x20       Button \"go\"\n\
         \x20           on click\n\
         \x20               set elapsed to 0\n",
    );
    assert!(message.contains("written by the clock"), "{message}");
    assert!(message.contains("every \"250ms\""), "{message}");

    // And a two-way binding is a write, so it is refused too — with the
    // clause named, rather than with `from`'s sentence.
    let message = only(
        "state ready is client Truth after \"2s\"\n\
         view\n\
         \x20   Column\n\
         \x20       Checkbox ready, label is \"ready\"\n",
    );
    assert!(message.contains("after \"2s\""), "{message}");
}

/// A component instance's own clock is checked the same way, and it is the
/// position where the cleanup rule has something to clean up.
#[test]
fn a_component_local_clock_is_checked_like_a_top_level_one() {
    accept(
        "component Pulse\n\
         \x20   state beat is client Decimal every \"500ms\"\n\
         \x20   Row\n\
         \x20       Text beat\n\
         view\n\
         \x20   Column\n\
         \x20       Pulse\n",
    );

    let message = only(
        "component Pulse\n\
         \x20   state beat is client Text every \"500ms\"\n\
         \x20   Row\n\
         \x20       Text beat\n\
         view\n\
         \x20   Column\n\
         \x20       Pulse\n",
    );
    assert!(message.contains("`Text`"), "{message}");
    assert!(message.contains("`Decimal`"), "{message}");
}

// --- the two binder forms (#33, #103, #104) -------------------------------
//
// Neither makes a function a value, so neither needs a function type, and
// what the checker has to do instead is written out here: one equation for
// the fold, two rules and a refusal for the payload transform.

/// **A fold's step gives what its seed gave.** The one equation the clause
/// needs, and the reason it can be checked in a language with no arrow
/// type: there is no function here, only two expressions that have to
/// agree.
#[test]
fn a_folds_step_must_give_what_its_seed_gave() {
    accept(
        "function totalOf of ns\n    \
         from ns\n    \
         fold each n into total starting 0 to total + n\n",
    );
    let message = only(
        "function totalOf of ns\n    \
         from ns\n    \
         fold each n into total starting 0 to \"nope\"\n",
    );
    assert_eq!(
        message,
        "Each step of `fold each` gives `Text`, but it has to be `Whole` or `Decimal`."
    );
}

/// **A fold ends its pipeline.** Every other clause takes a sequence and
/// gives one; this takes a sequence and gives a value, so a clause after
/// it has nothing to walk. Said by name here, because the alternative is
/// `.filter` against a number in the browser.
#[test]
fn nothing_may_follow_a_fold_in_a_pipeline() {
    let message = only(
        "function totalOf of ns\n    \
         from ns\n    \
         fold each n into total starting 0 to total + n\n    \
         keep each n where n > 1\n",
    );
    assert!(message.contains("ends a pipeline"), "{message}");
    assert!(message.contains("from"), "{message}");
}

/// And a second `from` starts a new one, so the rule is about the
/// sequence rather than about the block.
#[test]
fn a_from_after_a_fold_starts_a_new_pipeline() {
    accept(
        "function shape of ns\n    \
         from ns\n    \
         fold each n into total starting 0 to total + n\n    \
         from [1, 2, 3]\n    \
         keep each n where n > 1\n",
    );
}

/// **`map each … in` walks an `Option`.**
#[test]
fn a_payload_map_over_an_option_gives_an_option() {
    let table = accept(
        "state maybe is client Option of Whole starting None\n\
         state doubled is client Option of Whole from map each n in maybe to n * 2\n",
    );
    let _ = table;
}

/// **And a `Remote`, keeping all three arms.**
#[test]
fn a_payload_map_over_a_remote_gives_a_remote() {
    accept(
        "state who is client Text starting \"a\"\n\
         state greeting is server Text from echo of who\n\
         state shouted is client Remote of Text from map each line in greeting to line + \"!\"\n\n\
         function echo of name\n    give name\n",
    );
}

/// **A `List` is refused, and the refusal names the pipeline.**
///
/// This is the §4.1 boundary the form is drawn against rather than a
/// limitation: the language already has a phrase for walking a sequence,
/// and one construct may not have two spellings. The checker enforces it;
/// it is not left to convention.
#[test]
fn a_payload_map_over_a_list_is_refused_and_names_the_pipeline() {
    let message = only(
        "state rows is client List of Whole starting [1, 2]\n\
         state doubled is client List of Whole from map each n in rows to n * 2\n",
    );
    assert!(
        message.contains("list is walked by a pipeline"),
        "{message}"
    );
    assert!(message.contains("from xs map each x to"), "{message}");
}

/// Anything else is refused and says what it found.
#[test]
fn a_payload_map_over_a_plain_value_is_refused() {
    let message = only(
        "state n is client Whole starting 1\n\
         state doubled is client Option of Whole from map each x in n to x * 2\n",
    );
    assert!(message.contains("Option"), "{message}");
    assert!(message.contains("Remote"), "{message}");
    assert!(message.contains("Whole"), "{message}");
}

/// The binder is the payload, not the container: `n` here is the `Whole`
/// inside an `Option of Whole`, so a mistake in the body is reported
/// against the payload's type and at the expression that made it — which
/// is what the checker's ordering buys and would be lost if the binder
/// took a fresh variable when the container is already known.
#[test]
fn a_payload_maps_binder_is_the_payload() {
    let errors = reject(
        "state maybe is client Option of Whole starting None\n\
         state wrong is client Option of Text from map each n in maybe to n + \"x\"\n",
    );
    assert_eq!(
        errors[0],
        "The right side of this `+` is `Text`, but `Whole` is expected here."
    );
}
