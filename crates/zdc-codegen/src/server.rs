//! Function bundles — one file per emitted server root.
//!
//! A function bundle emits **no import of a generated module**. It cannot
//! import another endpoint, the client, or `dom.js`: §16.3.12's assertion A
//! forbids the first two and invariant 4 the third, and between them that
//! is a syntactic property of the output rather than a claim about it. Its
//! only *injected* names remain `$env` and `$store`, which is what keeps
//! the adapter portable across the six targets.
//!
//! The one import it may write is a **user `foreign`** (#223). §14E.2 links
//! a foreign into whichever bundles actually call it, and an endpoint is
//! one of those bundles — so a `foreign … is server` that this file calls
//! is imported here, and [`ServerFunction::linked`] reports the module so
//! the caller can ship it beside the endpoint.
//!
//! That is not a hole in invariant 4. The invariant keeps *generated*
//! modules from importing each other, because a shared generated module is
//! the edge that would make the split analysable only through `import`
//! statements. An author's own JavaScript is the thing §14E exists to
//! admit, and the endpoint is already a module the platform loads — so
//! resolution is a property the target has, not one the adapter provides.
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

use std::collections::{BTreeMap, BTreeSet};

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
    /// `handler({ beat })` — a job the deployment runs on a schedule
    /// (§14G.4).
    ///
    /// Shaped like a value endpoint and **not routable like one**. It
    /// destructures a parameter object exactly as `Value` does, and its
    /// one input is the beat's start time in seconds since the Unix
    /// epoch, which the platform supplies because only the platform knows
    /// it — Cloudflare's `scheduledTime` and EventBridge's `time` are the
    /// *scheduled* instant, so a late beat still reports when it was due
    /// and §14G.4 revision 5's "a skipped beat is observable" holds.
    ///
    /// A third kind rather than a flag on `Value`, because the difference
    /// is exactly the one a router must not get wrong: nothing on the
    /// wire may start this. `_zd/endpoints.js` therefore does not carry
    /// it, and the platform entry dispatches it directly.
    Trigger(zdc_ast::Cadence),
}

impl FunctionKind {
    /// The word the manifest carries, so the host and the manifest cannot
    /// disagree about which shape to send.
    pub fn word(self) -> &'static str {
        match self {
            FunctionKind::Value => "value",
            FunctionKind::Command => "command",
            FunctionKind::Trigger(_) => "trigger",
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
    /// The `foreign` modules this endpoint imports by relative path, and
    /// where each has to land for the import to resolve (#223).
    ///
    /// An endpoint sits in `functions/`, so `./io.js` written in the source
    /// resolves to `functions/io.js` in the bundle.
    pub linked: Vec<crate::LinkedModule>,
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
            root_body(hir, split, names, emitter, root, Tail::Value(*def), &inputs),
        ),
        EndpointKind::Command(key) => (
            FunctionKind::Command,
            command_body(hir, emitter.types, names, key),
        ),
    };
    let reached = std::mem::replace(&mut emitter.used, outer);
    emitter.used.absorb(&reached);
    Some(assemble(
        hir,
        names,
        &reached,
        Assembled {
            name: endpoint.name.clone(),
            kind,
            // A command's arguments are positional, so naming them in the
            // manifest would invite a caller to send an object.
            inputs: match kind {
                FunctionKind::Value => inputs,
                FunctionKind::Command | FunctionKind::Trigger(_) => Vec::new(),
            },
            body,
        },
        source_path,
    ))
}

/// One scheduled job, as a file the platform's scheduler calls (§14G.4).
///
/// It goes through the same assembly as an endpoint — the same header, the
/// same `foreign` imports, the same intrinsics preamble — because it is
/// the same kind of artefact: a standalone module handed to a platform
/// adapter with `$env` and `$store` injected. What differs is that nothing
/// on the wire may reach it, which is recorded in its
/// [`FunctionKind`](FunctionKind::Trigger) and enforced by the endpoint
/// table not listing it.
pub fn emit_trigger(
    hir: &Hir,
    split: &TierSplit,
    names: &Names,
    emitter: &mut Emitter<'_>,
    trigger: &zdc_graph::Trigger,
    source_path: &str,
) -> ServerFunction {
    // The beat's start time, delivered to the cell the declaration named.
    let inputs = vec![names.def(trigger.def).to_string()];
    let cadence = trigger.cadence;
    let body = {
        let outer = std::mem::take(&mut emitter.used);
        let DefKind::Signal(signal) = &hir.defs[trigger.def].kind else {
            unreachable!("a trigger's definition is the scheduled signal")
        };
        let schedule = signal
            .schedule
            .as_ref()
            .expect("the split builds a trigger only from a scheduled signal");
        let body = root_body(
            hir,
            split,
            names,
            emitter,
            trigger.root,
            Tail::Job(schedule.body),
            &inputs,
        );
        let reached = std::mem::replace(&mut emitter.used, outer);
        emitter.used.absorb(&reached);
        (body, reached)
    };
    let (body, reached) = body;
    assemble(
        hir,
        names,
        &reached,
        Assembled {
            name: trigger.name.clone(),
            kind: FunctionKind::Trigger(cadence),
            inputs,
            body,
        },
        source_path,
    )
}

/// What [`assemble`] is given, so that adding a field is a compile error
/// at both call sites rather than a positional argument nobody notices.
struct Assembled {
    name: String,
    kind: FunctionKind,
    inputs: Vec<String>,
    body: String,
}

/// The file around a handler: the header, the `foreign` imports it needs,
/// the intrinsics preamble, and the handler itself.
fn assemble(
    hir: &Hir,
    names: &Names,
    reached: &crate::RuntimeImports,
    parts: Assembled,
    source_path: &str,
) -> ServerFunction {
    let mut source = String::new();
    let mut linked: Vec<crate::LinkedModule> = Vec::new();
    source.push_str(&format!(
        "// zdc {} · {source_path} · generated, do not edit\n",
        env!("CARGO_PKG_VERSION")
    ));
    // §14E.2 links a foreign into whichever bundles actually call it, and an
    // endpoint is one of those bundles. The header below used to be
    // unconditional, so a `foreign` reached from a `server` signal was called
    // and never imported — a `ReferenceError` on the first request, which is
    // the failure the intrinsics preamble already existed to prevent for
    // prelude primitives (#223).
    if reached.foreign.is_empty() {
        source.push_str(
            "// No imports. `$env` and `$store` are injected by the platform adapter (§8.2).\n",
        );
    } else {
        source.push_str("// `$env` and `$store` are injected by the platform adapter (§8.2).\n");
        for (def, (module, export)) in &reached.foreign {
            let local = names.def(*def);
            let export = crate::js::ident(export)
                .expect("the export was validated at parse time and again at emission");
            // A bare specifier is written as the target the project mapped
            // it to, rather than left bare as it is in `client.js` (#238).
            //
            // The client keeps the bare name because the *document* carries
            // an import map, and that is what makes several imports of one
            // package resolve to one module in the browser. An endpoint has
            // no document: it is a standalone file handed to a platform
            // adapter, so there is nowhere for a map to live and the bare
            // name would resolve only if the deploy target happened to have
            // a package manifest saying the same thing. Substituting here
            // is the same resolution reached the only way available on this
            // side of the wire.
            let specifier = match crate::foreign_target(hir, *def) {
                Some(zdc_hir::ModuleTarget::Mapped(target)) => target,
                // `None` is a method, which imports nothing and so cannot
                // appear in this loop; it shares the "as written" answer
                // rather than inventing a second one for a case that has
                // no specifier to substitute anyway.
                Some(zdc_hir::ModuleTarget::AsWritten) | None => module.clone(),
            };
            source.push_str(&format!(
                "import {{ {export} as {local} }} from {};\n",
                crate::js::string(&specifier)
            ));
            linked.extend(crate::linked_module(&specifier, "functions"));
        }
    }
    // §8.2's adapter injects `$env` and `$store` and nothing else, so a
    // handler that constructs a variant or reaches a prelude primitive
    // declares those itself — otherwise it throws a `ReferenceError` on
    // the first request, which is the same gap the build root had.
    let preamble = crate::intrinsics::preamble(reached);
    if !preamble.is_empty() {
        source.push('\n');
        source.push_str(&preamble);
    }
    source.push_str(&parts.body);

    ServerFunction {
        path: file_name(&parts.name),
        name: parts.name,
        inputs: parts.inputs,
        kind: parts.kind,
        source,
        linked,
    }
}

/// What the emitted handler does after its members are bound.
///
/// Two arms and one printer, because everything before the last line is
/// the same question — which definitions are members of this root, in
/// which order, at which scope — and answering it twice is how the two
/// answers drift. What differs is only whether the handler ends by
/// returning a value or by running a block.
enum Tail {
    /// A value endpoint: the browser reads a signal, and this recomputes
    /// it from the inputs the browser supplied.
    Value(DefId),
    /// A scheduled job: the deployment's scheduler runs the block, and
    /// nothing is returned to anybody (§14G.4 revision 3 — a handler
    /// eliminates every `Remote` it produces and delegates nothing to the
    /// platform, so there is no result for a `return` to carry).
    Job(zdc_hir::BlockId),
}

/// One emitted server root: its members, then its tail.
fn root_body(
    hir: &Hir,
    split: &TierSplit,
    names: &Names,
    emitter: &mut Emitter<'_>,
    root: RootId,
    tail: Tail,
    inputs: &[String],
) -> String {
    // §17.4.5's prelude closure, for *this* root. A `server` derivation
    // reaches the library through a type-directed operator exactly as the
    // view does — `contains` inside one names `textContains`, which the
    // split could not follow because it ran before the checker — and each
    // endpoint is its own bundle, so the seed is this root's members and
    // not the program's.
    let reached: BTreeSet<DefId> = split.members_of(root).map(|(def, _)| def).collect();
    let members: BTreeMap<DefId, MemberForm> = split
        .members_of(root)
        .chain(
            emitter
                .analysis
                .operator_closure(hir, &reached)
                .into_iter()
                .filter_map(|def| library_member(hir, def)),
        )
        .collect();
    let members: Vec<(DefId, MemberForm)> = members.into_iter().collect();
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
    // A cycle is trampolined only when all of it lands at one scope, so
    // the two scopes are counted separately (#198).
    let groups = crate::tailgroup::TailGroups::find(hir);
    let (at_module, in_closure): (BTreeSet<DefId>, BTreeSet<DefId>) = members
        .iter()
        .filter(|(_, form)| *form == MemberForm::Function)
        .map(|(def, _)| *def)
        .partition(|def| hoisted(*def));

    for (def, form) in &members {
        if *form != MemberForm::Function {
            continue;
        }
        let present = if hoisted(*def) {
            &at_module
        } else {
            &in_closure
        };
        let text = function_text(
            hir,
            names,
            emitter,
            *def,
            if hoisted(*def) { 0 } else { 2 },
            &groups,
            present,
        );
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
        // A scheduled signal is its root's entry *and* its handler's one
        // parameter: the beat's start time is the platform's to supply,
        // not the program's to compute. Binding it here as well would
        // emit `const hourly = 0` beside `handler({ hourly })`, which is
        // a redeclaration and a `SyntaxError` in the deployed file.
        .filter(|(def, _)| !inputs.iter().any(|input| input == names.def(*def)))
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
            // A claim has no business in a server bundle, and two
            // independent things stop it reaching one: it is seeded only
            // into the build root, and `signals` above admits only
            // `Binding` and `StoreRead`. Its own arm rather than a place
            // in the list below, because falling into that list would
            // emit `const <claim> = <expectation>` into a deployed
            // function — the exact shipping accident `MemberForm::Test`
            // exists to make impossible (issue #169).
            MemberForm::Test => unreachable!(
                "a test's expectation is a member of the build root and of nothing else"
            ),
            // `signals` was filtered to these two forms above, so this is
            // the `Binding` case. Named rather than wildcarded: a new
            // member form silently emitting `const x = <init>` in a server
            // bundle is a `ReferenceError` at run time, not a compile
            // error here.
            MemberForm::Binding | MemberForm::Function | MemberForm::Inlined | MemberForm::View => {
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
    match tail {
        Tail::Value(result) => out.push_str(&format!("  return {};\n", names.def(result))),
        Tail::Job(body) => {
            let mut statements = String::new();
            crate::stmt::Statements {
                emitter,
                temporaries: 0,
                awaited: false,
                commands: 0,
                writes: Vec::new(),
                loops: 0,
                unbounded: false,
                tail: None,
                bounce: None,
            }
            .block(body, 2, &mut statements);
            out.push_str(&statements);
        }
    }
    out.push_str("}\n");
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
        HirExprKind::Text(text) => Some(js::string(text).to_string()),
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
        | HirExprKind::Media(_)
        | HirExprKind::Scroll
        | HirExprKind::Build { .. }
        // A request is `client`-placed and never reaches a store, so it
        // has no stored default to be.
        | HirExprKind::Outbound { .. }
        // Not a literal, so it has no constant form to inline into an
        // endpoint's own module. It is emitted where it is read.
        | HirExprKind::Conditional { .. }
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
        | HirExprKind::Index { .. }
        | HirExprKind::Append { .. }
        | HirExprKind::Insert { .. }
        | HirExprKind::MapInside { .. } => None,
    }
}

/// What form a definition §17.4.5's closure added takes in a root.
///
/// Everything the closure can reach is a function, and this is where that
/// is checked rather than asserted in a comment. `operator_target` names a
/// prelude function, `sites_of` records a call edge only to a function,
/// and the Phase-0 invariant (§17.4.1) says no prelude definition is or
/// reaches a signal — so the remaining arms are unreachable. They yield
/// `None` rather than panicking: a wrong answer here should leave the
/// emission short a symbol the surrounding assertions already check for,
/// not abort a build.
fn library_member(hir: &Hir, def: DefId) -> Option<(DefId, MemberForm)> {
    match &hir.defs[def].kind {
        DefKind::Function(_) => Some((def, MemberForm::Function)),
        DefKind::Signal(_)
        | DefKind::View(_)
        | DefKind::Record(_)
        | DefKind::Choice(_)
        | DefKind::Component(_)
        | DefKind::Foreign(_)
        // §17.4.5's closure runs over the prelude, and a `release` is a
        // program's own declaration — it is a member of its root already
        // and never arrives through the closure.
        | DefKind::Release(_) => None,
    }
}

/// One function, at the scope it belongs to.
///
/// `groups` and `present` are the mutual tail-recursion rewrite (#198).
/// `present` is the set of functions emitted into *this* scope, and the
/// trampoline is applied only when a whole cycle is inside it: a member's
/// body bounces to its siblings' `$step$` functions by name, so a sibling
/// at module scope cannot be reached from one nested inside a root's
/// closure, nor the other way round.
pub(crate) fn function_text(
    hir: &Hir,
    names: &Names,
    emitter: &mut Emitter<'_>,
    def: DefId,
    indent: usize,
    groups: &crate::tailgroup::TailGroups,
    present: &BTreeSet<DefId>,
) -> String {
    // **A `release` emits here too, and used to emit nothing.** Its body is
    // a function of its parameters in exactly the way an ordinary one is —
    // §19 adds a bandwidth declaration, an endorsement vector and a budget,
    // and none of those changes what the body *is*. The `let ... else` that
    // stood here matched `Function` alone and returned the empty string for
    // everything else, so a root reaching a release emitted the call and not
    // the definition: `functions/result.js` called `judge(...)` with no
    // `judge` in the module and threw `ReferenceError` on the first request.
    // The manual's own §19 example did it.
    //
    // Written out rather than `_`, so the next declaration that can be
    // reached from a root is a compile error here instead of an empty
    // string.
    let (locals, body) = match &hir.defs[def].kind {
        DefKind::Function(function) => (&function.params, function.body),
        DefKind::Release(release) => (&release.params, release.body),
        DefKind::Signal(_)
        | DefKind::View(_)
        | DefKind::Record(_)
        | DefKind::Choice(_)
        | DefKind::Component(_)
        | DefKind::Foreign(_) => return String::new(),
    };
    let params: Vec<String> = locals
        .iter()
        .map(|param| names.local(*param).to_string())
        .collect();
    let name = names.def(def).to_string();

    // The same rewrite the client path applies, for the same reason. A
    // server root gets its own copy of the closure (§17.4.5) and therefore
    // its own copy of the prelude's folds, and every one of those is
    // written to call itself in tail position. Emitting them here as plain
    // recursion would give the server the stack depth the rewrite exists
    // to remove — a `lines of` over a document would run the host out of
    // stack on the server while working on the client.
    let tail = crate::stmt::gives_a_self_call(hir, def, body).then(|| crate::stmt::TailSelfCall {
        def,
        params: locals.clone(),
    });
    let looped = tail.is_some();

    // And the same trampoline, for the same reason again: a server root
    // gets its own copy of the closure, so a cycle of mutually
    // tail-recursive functions would have on the server exactly the depth
    // #198 records removing on the client.
    let bounce = groups
        .group_of(def)
        .filter(|group| group.iter().all(|member| present.contains(member)))
        .map(|group| crate::stmt::BounceGroup {
            members: group.clone(),
        });
    let stepped = bounce.is_some();

    let mut statements = String::new();
    Statements {
        emitter,
        temporaries: 0,
        awaited: false,
        commands: 0,
        writes: Vec::new(),
        loops: 0,
        unbounded: false,
        tail,
        bounce,
    }
    .block(body, indent + if looped { 4 } else { 2 }, &mut statements);

    let pad = " ".repeat(indent);
    let emitted = if stepped {
        crate::tailgroup::step_name(&name)
    } else {
        name.clone()
    };
    let mut out = if looped {
        format!(
            "{pad}function {emitted}({}) {{\n{pad}  $tail: while (true) {{\n{statements}{pad}  }}\n{pad}}}\n",
            params.join(", ")
        )
    } else {
        format!(
            "{pad}function {emitted}({}) {{\n{statements}{pad}}}\n",
            params.join(", ")
        )
    };
    if stepped {
        emitter.use_helper("$bounce");
        out.push_str(&format!(
            "{pad}function {name}({0}) {{\n{pad}  return $bounce({emitted}({0}));\n{pad}}}\n",
            params.join(", ")
        ));
    }
    out
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
        let cleared = verdict
            .clearance()
            .expect("the fixture is cleared by the flow pass");
        let bundle = crate::compile(
            &Inputs {
                hir: &hir,
                split: &split,
                verdict: &verdict,
                table: &table,
                cleared,
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
    ///
    /// The `Failed` arm renders a constant and not `e.message`, because
    /// §14G.1.3(d) makes the payload of an endpoint that read a `secret`
    /// secret in turn. That is the rule's cost, and it lands on every
    /// fixture in this repository shaped like the flagship example.
    /// A `release` — §19's bounded disclosure, as `docs/reference.md`
    /// writes it. Its body is the only definition of `judge`, which is what
    /// makes it the fixture that catches an emission dropping it.
    const RELEASE: &str = "\
state answer is durable Text starting \"cabbage\"
state guess is client Text starting \"\"
state result is server Option of Truth from judge with guess, answer

release judge with guess, answer
    gives Truth
    trusted guess
    trusted answer
    limit 20 per visitor
    give guess is answer

view
    Column
        Input guess, hint is \"your guess\"
        when result
            Loading show Text \"...\"
            Failed with error show Text \"unavailable\"
            Ready with verdict
                when verdict
                    None
                        Text \"no guesses left\"
                    Some with right
                        Text right
";

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
            Failed with e   show ErrorBar message is \"the greeting service did not answer\"
            Ready with text show Text text
";

    #[test]
    fn a_function_bundle_contains_no_import_statement() {
        // §16.3.12 invariant 4. Asserted as a property of the bytes rather
        // than trusted from the header comment that claims it, because the
        // comment is printed unconditionally and the imports would not be.
        let mut scanned = 0;
        for function in functions(COUNTER).into_iter().chain(functions(GREETING)) {
            scanned += 1;
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
        // Two fixtures, both of which emit at least one endpoint. A
        // change that stopped emitting them would otherwise pass this
        // test over an empty list.
        assert!(scanned >= 2, "only {scanned} endpoints were read");
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
        // A bundle with no `$` in it at all would satisfy the loop below
        // over nothing: the two fixtures between them read the store and
        // the environment, so both names must be present.
        assert!(
            seen.contains(&"$store".to_string()) && seen.contains(&"$env".to_string()),
            "the fixtures no longer reach the store and the environment: {seen:?}"
        );
        for name in &seen {
            assert!(
                matches!(name.as_str(), "$env" | "$store" | "$args"),
                "`{name}` is a free name no platform adapter binds; seen: {seen:?}"
            );
        }
    }

    /// **A `release` body is emitted into the endpoint that calls it.**
    ///
    /// It was not. `function_text` matched `DefKind::Function` alone and
    /// returned the empty string for anything else, so a root reaching a
    /// release emitted `judge(...)` and no `judge`: `ReferenceError` on the
    /// first request, from the example `docs/reference.md` §19 prints.
    ///
    /// The test above could not see it. That one scans `$`-prefixed names,
    /// because the free names it was written for are the adapter's; a
    /// release is called by the name the program gave it, which has no `$`.
    #[test]
    fn a_release_body_travels_with_the_endpoint_that_calls_it() {
        let result = named(RELEASE, "result");
        assert!(
            result.source.contains("function judge("),
            "the release body was not emitted beside its call:\n{}",
            result.source
        );
        // The call, so that a change emitting the body and losing the call
        // fails here rather than passing on half the pair.
        assert!(result.source.contains("judge("), "{}", result.source);
    }

    /// Every name an endpoint uses is one it defines, one it imports, or
    /// one §8.2 injects — for **any** name, not only the `$`-prefixed ones.
    ///
    /// The narrower test above stood while `functions/result.js` called a
    /// `judge` that was nowhere in the file, because `judge` has no `$` in
    /// it. This asks the question the header comment actually promises.
    #[test]
    fn every_called_name_is_defined_imported_or_injected() {
        // Counted, because every assertion below is inside a loop: an
        // emission that produced no endpoints, or one whose bodies made no
        // calls, would satisfy the loop over nothing and report a pass. The
        // three fixtures between them emit four endpoints, and the release
        // one calls `judge` — so a floor that a vacuous run cannot clear.
        let mut checked = 0usize;
        let mut endpoints = 0usize;
        for source in [COUNTER, GREETING, RELEASE] {
            for function in functions(source) {
                endpoints += 1;
                let text = &function.source;
                // Names in call position: `foo(` not preceded by `.`, and
                // not a JavaScript keyword that takes a parenthesis.
                for (index, _) in text.match_indices('(') {
                    let head = &text[..index];
                    let name: String = head
                        .chars()
                        .rev()
                        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    if name.is_empty() {
                        continue;
                    }
                    let dotted = head.ends_with(&format!(".{name}"));
                    if dotted
                        || matches!(
                            name.as_str(),
                            "if" | "for"
                                | "while"
                                | "switch"
                                | "catch"
                                | "function"
                                | "return"
                                | "typeof"
                                | "await"
                                | "handler"
                                | "String"
                                | "Number"
                                | "Boolean"
                                | "Object"
                                | "Array"
                                | "Math"
                                | "JSON"
                                | "Error"
                                | "Map"
                                | "Set"
                                | "BigInt"
                                | "Promise"
                        )
                    {
                        continue;
                    }
                    let defined = text.contains(&format!("function {name}("))
                        || text.contains(&format!("const {name} ="))
                        || text.contains(&format!("let {name} ="))
                        || text.contains(&format!(" as {name} "))
                        || text.contains(&format!("{name},"))
                        || matches!(name.as_str(), "$env" | "$store" | "$args");
                    assert!(
                        defined,
                        "`{}` calls `{name}` and neither defines, imports nor \
                         is injected it, so the first request throws \
                         `ReferenceError`:\n{text}",
                        function.path
                    );
                    checked += 1;
                }
            }
        }
        assert!(
            endpoints >= 4,
            "the fixtures emitted {endpoints} endpoints, so this ran over \
             nearly nothing"
        );
        assert!(
            checked >= 4,
            "only {checked} call sites were examined, so the scan stopped \
             working rather than the emissions losing their calls"
        );
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
        let mut scanned = 0;
        for function in functions(COUNTER) {
            scanned += 1;
            assert!(
                function.source.contains("export async function handler"),
                "{} is not async:\n{}",
                function.name,
                function.source
            );
        }
        assert!(scanned >= 1, "the counter fixture emitted no endpoint");
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
        let mut scanned = 0;
        for function in functions(COUNTER) {
            scanned += 1;
            let first = function.source.lines().next().unwrap_or_default();
            assert!(
                first.contains("test.zd") && first.contains("generated, do not edit"),
                "{} has no provenance line: {first}",
                function.name
            );
        }
        assert!(scanned >= 1, "the counter fixture emitted no endpoint");
    }
}
