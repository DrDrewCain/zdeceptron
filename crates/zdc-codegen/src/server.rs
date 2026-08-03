//! Function bundles — one file per emitted server root.
//!
//! A function bundle emits **zero import statements**. Its only external
//! references are `$env` and `$store`, injected by the platform adapter
//! (§8.2), which is what makes §16.3.12's invariant 4 a syntactic property
//! of the output rather than a claim about it.
//!
//! What goes in the file is not a decision made here. `members(r)` is the
//! split's answer, `hoisted[(d, r)]` says whether a member can live at
//! module scope or must be nested where the lifted parameters are in
//! lexical scope, and `params(r)` is the wire signature. This module is a
//! printer.
//!
//! # The two argument shapes
//!
//! A file emits one of two handler signatures, and which one is not
//! cosmetic — it is the wire contract §18.2 makes the mutation verb carry:
//!
//! ```text
//! value    export async function handler({ name })   ← named, wire order
//! command  export async function handler($args)      ← positional
//! ```
//!
//! A value endpoint recomputes a signal from inputs the browser named, so
//! its parameters are named. A command carries the right-hand side and the
//! indexes of the place being written, evaluated in the region that asked
//! (§17.2.7), and those have no names on the far side — the endpoint's own
//! name (`visits.incr`) is what identifies the operation.
//!
//! [`ServerFunction::kind`] records which, because a caller that guesses
//! wrong passes an array where an object is destructured and every input
//! silently arrives as `undefined`.

use zdc_graph::{EndpointKind, MemberForm, RootId, TierSplit};
use zdc_hir::{DefId, DefKind, Hir, HirExprKind};

use crate::expr::Emitter;
use crate::js;
use crate::names::Names;
use crate::stmt::Statements;

/// Which of the two handler signatures a file emitted.
///
/// The two endpoint kinds do not share a calling convention, and the
/// difference is not recoverable from the endpoint's name or its input
/// list — `visits.incr` has no declared inputs and still reads `$args[0]`.
/// Anything that dispatches to a handler has to be told which it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionKind {
    /// `handler({ a, b })` — a signal the browser reads. A value endpoint
    /// destructures a parameter object whose keys are
    /// [`ServerFunction::inputs`].
    Value,
    /// `handler($args)` — a mutation the browser asked for. A command takes
    /// the argument array positionally (§17.2.7): the right-hand side and
    /// every index were evaluated in the region that asked and arrive in
    /// wire order.
    Command,
}

impl FunctionKind {
    /// The word the manifest carries, so the host and the manifest cannot
    /// disagree about which shape to send.
    pub fn word(self) -> &'static str {
        match self {
            FunctionKind::Value => "value",
            FunctionKind::Command => "command",
        }
    }
}

/// One emitted server file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerFunction {
    /// Path relative to the output directory, such as
    /// `functions/greeting.js`.
    pub path: String,
    /// The endpoint name the client calls, such as `visits.incr`.
    pub name: String,
    /// The wire order of the endpoint's inputs. Empty for a command,
    /// whose arguments are positional and unnamed.
    pub inputs: Vec<String>,
    /// Which handler signature [`source`](ServerFunction::source) emitted,
    /// and so how it takes its arguments.
    pub kind: FunctionKind,
    pub source: String,
}

/// The file name an endpoint is emitted to. `.` is legal in a POSIX file
/// name and is what makes `visits.incr` and `visits.decr` distinct files
/// without an escaping scheme (§17.2.5 fatal 3).
pub fn file_name(endpoint: &str) -> String {
    format!("functions/{endpoint}.js")
}

pub fn emit_one(
    hir: &Hir,
    split: &TierSplit,
    names: &Names,
    emitter: &mut Emitter<'_>,
    endpoint: &zdc_graph::Endpoint,
    source_path: &str,
) -> Option<ServerFunction> {
    let root = endpoint.root;
    let inputs: Vec<String> = endpoint
        .params
        .iter()
        .map(|param| names.def(*param).to_string())
        .collect();

    // This endpoint emits into an empty set, so what comes back is what
    // *it* reached rather than the emitter's running total. The total is
    // restored and the share folded into it, leaving the client's import
    // list exactly as it was.
    let outer = std::mem::take(&mut emitter.used);
    let (kind, body) = match &endpoint.kind {
        EndpointKind::Value(def) => (
            FunctionKind::Value,
            value_body(hir, split, names, emitter, root, *def, &inputs),
        ),
        EndpointKind::Command(key) => (
            FunctionKind::Command,
            command_body(hir, emitter.types, names, key),
        ),
    };
    let reached = std::mem::replace(&mut emitter.used, outer);
    emitter.used.absorb(&reached);

    let mut source = String::new();
    source.push_str(&format!(
        "// zdc {} · {source_path} · generated, do not edit\n",
        env!("CARGO_PKG_VERSION")
    ));
    source.push_str(
        "// No imports. `$env` and `$store` are injected by the platform adapter (§8.2).\n",
    );
    // §8.2's adapter injects `$env` and `$store` and nothing else, so a
    // handler that constructs a variant or reaches a prelude primitive
    // declares those itself — otherwise it throws a `ReferenceError` on
    // the first request, which is the same gap the build root had.
    let preamble = crate::intrinsics::preamble(&reached);
    if !preamble.is_empty() {
        source.push('\n');
        source.push_str(&preamble);
    }
    source.push_str(&body);

    Some(ServerFunction {
        path: file_name(&endpoint.name),
        name: endpoint.name.clone(),
        // A command's arguments are positional, so naming them in the
        // manifest would invite a caller to send an object.
        inputs: match kind {
            FunctionKind::Value => inputs,
            FunctionKind::Command => Vec::new(),
        },
        kind,
        source,
    })
}

/// A value endpoint: the browser reads a `server` or `durable` signal, and
/// this recomputes it from the inputs the browser supplied.
fn value_body(
    hir: &Hir,
    split: &TierSplit,
    names: &Names,
    emitter: &mut Emitter<'_>,
    root: RootId,
    result: DefId,
    inputs: &[String],
) -> String {
    let members: Vec<(DefId, MemberForm)> = split.members_of(root).collect();
    let hoisted = |def: DefId| split.hoisted.get(&(def, root)).copied().unwrap_or(true);
    // Copied out of the emitter so the store-read arm can consult the
    // checker's verdict while the emitter itself is borrowed mutably by the
    // arms around it.
    let types = emitter.types;

    let mut module = String::new();
    let mut nested = String::new();

    // Functions first, at whichever scope they belong to. A function that
    // needs no lifted value is emitted at module scope in every root, so
    // no instantiation ever changes arity (§17.2.8).
    for (def, form) in &members {
        if *form != MemberForm::Function {
            continue;
        }
        let text = function_text(hir, names, emitter, *def, if hoisted(*def) { 0 } else { 2 });
        if hoisted(*def) {
            module.push_str(&text);
        } else {
            nested.push_str(&text);
        }
    }

    // Then the signal bindings, dependencies first. `static_order` is the
    // topological order E0320 has just established exists.
    let mut signals: Vec<(DefId, MemberForm)> = members
        .iter()
        .filter(|(_, form)| matches!(form, MemberForm::Binding | MemberForm::StoreRead))
        .copied()
        .collect();
    // `static_order` is dependencies-first, which is the order a run of
    // `const` bindings has to be written in: a `const` referenced before
    // its declaration is a temporal-dead-zone `ReferenceError`, not a
    // hoisted `undefined`.
    signals.sort_by_key(|(def, _)| {
        split
            .static_order
            .iter()
            .position(|id| id == def)
            .unwrap_or(usize::MAX)
    });

    for (def, form) in &signals {
        let name = names.def(*def).to_string();
        match form {
            MemberForm::StoreRead => {
                let key = js::string(&hir.defs[*def].name);
                // A key nobody has written to yet reads as absent, and the
                // declaration says what it is before anyone writes: `state
                // visits is durable Whole starting 0` means the first
                // visitor sees 0, not `null`. Without this the demo renders
                // the word "null" until someone clicks.
                //
                // Only a literal default is emitted. The initializer of a
                // durable signal belongs to the build root (§17.2.8), so an
                // expression that named anything would name it out of
                // scope here — and a `ReferenceError` at the first read is
                // worse than the `null` it was meant to fix.
                match literal_default(hir, types, *def) {
                    Some(default) => nested.push_str(&format!(
                        "  const {name} = (await $store.get({key})) ?? {default};\n"
                    )),
                    None => {
                        nested.push_str(&format!("  const {name} = await $store.get({key});\n"))
                    }
                }
            }
            _ => {
                let DefKind::Signal(signal) = &hir.defs[*def].kind else {
                    continue;
                };
                let init = signal.init;
                let value = emitter.value(init).into_text();
                nested.push_str(&format!("  const {name} = {value};\n"));
            }
        }
    }

    let mut out = module;
    out.push_str(&format!(
        "\nexport async function handler({{ {} }}) {{\n",
        inputs.join(", ")
    ));
    out.push_str(&nested);
    out.push_str(&format!("  return {};\n}}\n", names.def(result)));
    out
}

/// A command endpoint: the browser asked for a write it cannot perform.
///
/// Only the place resolution and the store operator run here; the
/// right-hand side and every index arrived as arguments, evaluated in the
/// region that asked (§17.2.7).
///
/// # The path is one ordered argument
///
/// A write through a path used to spread its indices across the argument
/// list and put its record fields in a trailing array —
/// `$store.incr('votes', $args[0], $args[1])`. Two things were wrong with
/// that and only one of them was cosmetic. The order of a mixed path was
/// unrecoverable, because the two halves were carried in different places;
/// and nothing in the call said an index was an index, so the `$store`
/// façade in `zdc-host` read `$args[1]` as no argument at all and wrote the
/// *whole key* as a number. `add 1 to votes at candidate` destroyed the
/// tally it was counting.
///
/// So the path is one argument in source order — `[['at', v], ['field',
/// 'done']]` — and it says which kind each segment is. §18.2 makes the
/// mutation verb the wire contract and §14B.2 closes the verb set at five;
/// what a verb is *applied to* is part of that contract, so it travels
/// with the verb rather than being reconstructed by each of the five store
/// implementations from argument positions.
///
/// The declared `starting` value follows it, because a path write on a key
/// nobody has written has to make the container the declaration named. A
/// first vote cannot know that `votes` is a `Map` unless the call says so.
///
/// A place with no path emits neither, so the common case is exactly the
/// bytes it always was.
fn command_body(
    hir: &Hir,
    types: &zdc_types::TypeTable,
    names: &Names,
    key: &zdc_graph::CommandKey,
) -> String {
    let _ = names;
    let store_key = js::string(&hir.defs[key.signal].name);
    // The same word the command's own name was rendered from, so the
    // endpoint and the store operation cannot disagree.
    let operator = key.op.word();

    let mut indices = 0usize;
    let steps: Vec<String> = key
        .path
        .iter()
        .map(|segment| match segment {
            zdc_graph::PathKeySeg::Index => {
                indices += 1;
                format!("['at', $args[{indices}]]")
            }
            zdc_graph::PathKeySeg::Field(field) => format!("['field', {}]", js::string(field)),
        })
        .collect();

    let path = if steps.is_empty() {
        String::new()
    } else {
        format!(
            ", [{}], {}",
            steps.join(", "),
            literal_default(hir, types, key.signal).unwrap_or_else(|| "undefined".to_string())
        )
    };

    format!(
        "\nexport async function handler($args) {{\n  return await $store.{operator}({store_key}, \
         $args[0]{path});\n}}\n"
    )
}

/// The `starting` value of a durable signal, when it is a literal.
///
/// `None` for anything else, deliberately. This is not a general
/// evaluator: it is the narrow case where the declared default can be
/// printed into a root that does not own the initializer, and every wider
/// case emits a name that root cannot see.
///
/// `empty` is a literal too, and the reason it was not one is that it has
/// no container of its own — `empty` is a `List` or a `Map` and the syntax
/// does not say which (§16.7 item 6). The checker does say, and it already
/// recorded the answer for the emitter that prints `empty` in expression
/// position, so this asks the same table rather than inventing a second
/// rule. Without it a `Map … starting empty` read `null` on a fresh store,
/// and `examples/voting-board.zd` threw `cannot convert 'null' or
/// 'undefined' to object` on its first page load.
fn literal_default(hir: &Hir, types: &zdc_types::TypeTable, def: DefId) -> Option<String> {
    let DefKind::Signal(signal) = &hir.defs[def].kind else {
        return None;
    };
    match &hir.exprs[signal.init].kind {
        HirExprKind::Number(value) => Some(js::number(*value)),
        HirExprKind::Text(text) => Some(js::string(text)),
        HirExprKind::Truth(value) => Some(value.to_string()),
        HirExprKind::Empty => match types.empty_kind(signal.init) {
            Some(zdc_types::EmptyKind::List) => Some("[]".to_string()),
            Some(zdc_types::EmptyKind::Map) => Some("new Map()".to_string()),
            // unreached: `zdc-types` reports an `empty` with no container
            // first, in its own words, and an unsettled program never
            // reaches codegen.
            None => None,
        },
        HirExprKind::Address
        | HirExprKind::Build { .. }
        | HirExprKind::List(_)
        | HirExprKind::Map(_)
        | HirExprKind::Ref(_)
        | HirExprKind::Call { .. }
        | HirExprKind::OfCall { .. }
        | HirExprKind::Operator { .. }
        | HirExprKind::Environment(_)
        | HirExprKind::Unary { .. }
        | HirExprKind::Binary { .. }
        | HirExprKind::Field { .. }
        | HirExprKind::Index { .. } => None,
    }
}

pub(crate) fn function_text(
    hir: &Hir,
    names: &Names,
    emitter: &mut Emitter<'_>,
    def: DefId,
    indent: usize,
) -> String {
    let DefKind::Function(function) = &hir.defs[def].kind else {
        return String::new();
    };
    let body = function.body;
    let params: Vec<String> = function
        .params
        .iter()
        .map(|param| names.local(*param).to_string())
        .collect();
    let name = names.def(def).to_string();

    let mut statements = String::new();
    Statements {
        emitter,
        temporaries: 0,
        awaited: false,
        commands: 0,
        writes: Vec::new(),
        loops: 0,
        unbounded: false,
    }
    .block(body, indent + 2, &mut statements);

    let pad = " ".repeat(indent);
    format!(
        "{pad}function {name}({}) {{\n{statements}{pad}}}\n",
        params.join(", ")
    )
}

#[cfg(test)]
mod tests {
    //! The pass that decides what this module prints has 26 split tests and
    //! 20 information-flow tests behind it. Until now this module — which
    //! turns those decisions into the only bytes that ever run on a server
    //! — had none, so a printer that dropped a default, destructured the
    //! wrong shape, or emitted an import was checked by nothing.
    //!
    //! These are unit tests of the *emitted text*. That the text also runs
    //! is a separate and stronger claim, and `zdc-host` makes it.

    use crate::{Inputs, Options};

    /// Compile a source and return its server files, in emission order.
    ///
    /// The whole front end runs, exactly as `zdc build` runs it: a fixture
    /// that stubbed the split would be asserting against a decision this
    /// module does not make.
    fn functions(source: &str) -> Vec<super::ServerFunction> {
        let program = zdc_parser::parse(source).expect("the fixture parses");
        let hir = zdc_resolve::Resolver::new(&program)
            .resolve()
            .unwrap_or_else(|errors| panic!("the fixture resolves: {}", errors[0].message));
        let split = zdc_graph::split(&hir);
        assert!(
            !split.has_errors(),
            "the fixture must survive the split: {:?}",
            split
                .diagnostics
                .iter()
                .filter(|d| d.is_error())
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
        );
        let verdict = zdc_graph::ifc(&hir, &split);
        let table = zdc_types::check(&hir, &split).unwrap_or_default();
        let bundle = crate::compile(
            &Inputs {
                hir: &hir,
                split: &split,
                verdict: &verdict,
                table: &table,
            },
            &Options::new("test.zd", "test"),
        )
        .unwrap_or_else(|errors| panic!("the fixture emits: {}", errors[0].message));
        bundle.functions
    }

    fn named(source: &str, name: &str) -> super::ServerFunction {
        functions(source)
            .into_iter()
            .find(|function| function.name == name)
            .unwrap_or_else(|| panic!("no endpoint named `{name}` was emitted"))
    }

    /// A durable counter and the click that increments it: the smallest
    /// program with both an endpoint and a command.
    const COUNTER: &str = "\
state visits is durable Whole starting 0

view
    Column
        when visits
            Loading          show Spinner
            Failed with e    show ErrorBar message is e.message
            Ready with total show Text total
        Button \"count\"
            on click
                add 1 to visits
";

    /// A server signal computed from a client one and an environment
    /// secret — the shape `guestbook.zd` is built around.
    const GREETING: &str = "\
secret state apiKey is server Text from environment \"GREETING_API_KEY\"
state who is client Text starting \"\"
state greeting is server Text from politeGreeting with who, apiKey

function politeGreeting with name, key
    give \"Hello, \" + name + \".\"

view
    Column
        Input who, hint is \"name\"
        when greeting
            Loading         show Spinner
            Failed with e   show ErrorBar message is e.message
            Ready with text show Text text
";

    #[test]
    fn a_function_bundle_contains_no_import_statement() {
        // §16.3.12 invariant 4. Asserted as a property of the bytes rather
        // than trusted from the header comment that claims it, because the
        // comment is printed unconditionally and the imports would not be.
        for function in functions(COUNTER).into_iter().chain(functions(GREETING)) {
            for line in function.source.lines() {
                assert!(
                    !line.trim_start().starts_with("import "),
                    "{} emitted an import:\n{}",
                    function.name,
                    function.source
                );
            }
            assert!(
                !function.source.contains("require("),
                "{} emitted a CommonJS require",
                function.name
            );
        }
    }

    #[test]
    fn the_only_free_names_are_the_two_the_adapter_injects() {
        // The header comment promises `$env` and `$store` and nothing else.
        // A third `$`-prefixed free name would be a name no adapter binds,
        // and the failure would be a `ReferenceError` on the first request.
        let mut seen: Vec<String> = Vec::new();
        for function in functions(COUNTER).into_iter().chain(functions(GREETING)) {
            for (index, _) in function.source.match_indices('$') {
                let rest = &function.source[index..];
                let name: String = rest
                    .chars()
                    .take_while(|c| *c == '$' || c.is_alphanumeric() || *c == '_')
                    .collect();
                seen.push(name);
            }
        }
        seen.sort();
        seen.dedup();
        for name in &seen {
            assert!(
                matches!(name.as_str(), "$env" | "$store" | "$args"),
                "`{name}` is a free name no platform adapter binds; seen: {seen:?}"
            );
        }
    }

    #[test]
    fn a_value_endpoint_destructures_its_inputs_by_name() {
        let greeting = named(GREETING, "greeting");
        assert_eq!(greeting.kind, super::FunctionKind::Value);
        assert_eq!(greeting.inputs, vec!["who".to_string()]);
        assert!(
            greeting
                .source
                .contains("export async function handler({ who })"),
            "the wire signature is not the manifest's:\n{}",
            greeting.source
        );
    }

    #[test]
    fn a_command_endpoint_takes_one_positional_array() {
        // §17.2.7: the right-hand side was evaluated in the region that
        // asked and arrives as an argument. There is nothing to name.
        let incr = named(COUNTER, "visits.incr");
        assert_eq!(incr.kind, super::FunctionKind::Command);
        assert!(
            incr.inputs.is_empty(),
            "a command's arguments are positional, so naming them invites an object: {:?}",
            incr.inputs
        );
        assert!(
            incr.source.contains("export async function handler($args)"),
            "not the positional signature:\n{}",
            incr.source
        );
        assert!(
            incr.source.contains("$store.incr('visits', $args[0])"),
            "the operator or the key drifted from the endpoint name:\n{}",
            incr.source
        );
    }

    #[test]
    fn the_endpoint_name_and_the_store_operator_cannot_disagree() {
        // `visits.incr` is one word rendered twice — once into the file
        // name the browser posts to and once into the call this file makes.
        // If they were computed separately, a rename would move only one.
        let incr = named(COUNTER, "visits.incr");
        let operator = incr
            .name
            .rsplit_once('.')
            .map(|(_, verb)| verb.to_string())
            .expect("a command name carries its verb");
        assert!(
            incr.source.contains(&format!("$store.{operator}(")),
            "`{}` does not call `$store.{operator}`:\n{}",
            incr.name,
            incr.source
        );
    }

    #[test]
    fn a_durable_read_falls_back_to_the_declared_starting_value() {
        // A key nobody has written yet is absent, and `starting 0` is the
        // declaration that says what the first visitor sees. Without this
        // the page renders `null` until somebody clicks.
        let visits = named(COUNTER, "visits");
        assert!(
            visits.source.contains("(await $store.get('visits')) ?? 0"),
            "the declared default was dropped:\n{}",
            visits.source
        );
    }

    /// A durable map written through an index, and read back.
    const PATH_COMMAND: &str = "\
state scores is durable Map of Text to Whole starting empty
state label is client Text starting \"\"

view
    Column
        Input label, hint is \"what\"
        when scores
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with counts show Text \"ok\"
        Button \"vote\"
            on click
                add 1 to scores at label
";

    #[test]
    fn a_path_command_carries_its_place_as_one_ordered_argument() {
        // The index used to be spread into the argument list and record
        // fields into a trailing array, so nothing in the call said which
        // was which and the store façade dropped the index entirely — the
        // whole key became a number. The place is one argument, in source
        // order, and each segment says what kind it is.
        let incr = named(PATH_COMMAND, "scores.incr.at");
        assert!(
            incr.source
                .contains("$store.incr('scores', $args[0], [['at', $args[1]]], new Map())"),
            "the place the write names is not in the call:\n{}",
            incr.source
        );
    }

    #[test]
    fn a_place_with_no_path_still_emits_the_two_argument_call() {
        // The common case pays nothing for the one above. `counter.zd` and
        // `guestbook.zd` emit exactly the bytes they always did.
        let incr = named(COUNTER, "visits.incr");
        assert!(
            incr.source.contains("$store.incr('visits', $args[0]);"),
            "a pathless command grew an argument:\n{}",
            incr.source
        );
    }

    #[test]
    fn a_durable_read_falls_back_to_the_declared_empty_container() {
        // `starting empty` is a literal default too. Which container it is
        // comes off the checker's verdict (§16.7 item 6), the same table
        // that decides what `empty` prints in expression position — not off
        // the syntax, which does not say.
        let maps = "\
state scores is durable Map of Text to Whole starting empty

view
    Column
        when scores
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with value  show Text \"ok\"
";
        assert!(
            named(maps, "scores")
                .source
                .contains("(await $store.get('scores')) ?? new Map()"),
            "a `Map … starting empty` read as `null` on a fresh store"
        );

        let lists = "\
state names is durable List of Text starting empty

view
    Column
        when names
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with value  show Text \"ok\"
";
        assert!(
            named(lists, "names")
                .source
                .contains("(await $store.get('names')) ?? []"),
            "a `List … starting empty` read as `null` on a fresh store"
        );
    }

    #[test]
    fn a_starting_value_that_is_not_a_literal_emits_no_default() {
        // The initializer of a durable signal belongs to the build root, so
        // a name printed here would be out of scope. `null` is wrong;
        // `ReferenceError` on every read is worse.
        let source = "\
state seed is client Whole starting 7
state total is durable Whole starting 0

view
    Column
        when total
            Loading          show Spinner
            Failed with e    show ErrorBar message is e.message
            Ready with value show Text value
        Button \"go\"
            on click
                add 1 to total
";
        let total = named(source, "total");
        assert!(
            total.source.contains("await $store.get('total')"),
            "the store read went missing:\n{}",
            total.source
        );
    }

    #[test]
    fn an_environment_key_is_read_through_the_injected_accessor() {
        // §16.3.12 assertion C: the key name may appear in a server file
        // and never in the manifest or the client bundle. The value never
        // appears anywhere — it is fetched at invocation time.
        let greeting = named(GREETING, "greeting");
        assert!(
            greeting.source.contains("$env('GREETING_API_KEY')"),
            "the secret is not read through `$env`:\n{}",
            greeting.source
        );
    }

    #[test]
    fn the_handler_is_async_because_every_store_operation_is_awaited() {
        // A synchronous handler that returned a promise would hand the
        // adapter an unresolved value and the browser would render `{}`.
        for function in functions(COUNTER) {
            assert!(
                function.source.contains("export async function handler"),
                "{} is not async:\n{}",
                function.name,
                function.source
            );
        }
    }

    #[test]
    fn a_helper_the_endpoint_calls_is_emitted_beside_it() {
        // The bundle has no imports, so a function the handler calls has to
        // be in the same file or it is not anywhere.
        let greeting = named(GREETING, "greeting");
        assert!(
            greeting.source.contains("function politeGreeting("),
            "the helper was not emitted into the bundle that calls it:\n{}",
            greeting.source
        );
        assert!(
            greeting.source.contains("politeGreeting(who, apiKey)"),
            "the call and the definition disagree:\n{}",
            greeting.source
        );
    }

    #[test]
    fn every_endpoint_gets_its_own_file_named_after_it() {
        // `.` is legal in a POSIX file name, which is what makes
        // `visits.incr` and `visits.decr` distinct files with no escaping
        // scheme (§17.2.5 fatal 3).
        let emitted = functions(COUNTER);
        let mut paths: Vec<&str> = emitted.iter().map(|f| f.path.as_str()).collect();
        paths.sort();
        assert_eq!(
            paths,
            vec!["functions/visits.incr.js", "functions/visits.js"]
        );
        for function in &emitted {
            assert_eq!(function.path, super::file_name(&function.name));
        }
    }

    #[test]
    fn a_client_only_program_emits_no_server_file_at_all() {
        // §16.3.1: a bundle ships nothing it does not use, and "nothing"
        // has to include the server half.
        let source = "\
state count is client Whole starting 0

view
    Column
        Text count
        Button \"plus\"
            on click
                add 1 to count
";
        assert!(functions(source).is_empty());
    }

    #[test]
    fn the_header_names_the_source_file_it_was_generated_from() {
        // A generated file that does not say where it came from gets edited.
        for function in functions(COUNTER) {
            let first = function.source.lines().next().unwrap_or_default();
            assert!(
                first.contains("test.zd") && first.contains("generated, do not edit"),
                "{} has no provenance line: {first}",
                function.name
            );
        }
    }
}
