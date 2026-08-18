//! Naming, per spec §16.3.2.
//!
//! ZDeceptron identifiers are UAX#31, so almost all of them are already
//! valid JavaScript. The two ways a name can go wrong are collision with a
//! JavaScript reserved word and collision with another emitted name — a
//! source signal `count` reserves `setCount` as well as `count`, and a
//! program is free to declare something called `setCount`.
//!
//! Every compiler-generated name begins with `$`, which is in neither
//! XID_Start nor XID_Continue. A generated name therefore cannot collide
//! with a user name and a user name cannot shadow a generated one, without
//! any bookkeeping at all.

use std::collections::{BTreeSet, HashMap, HashSet};

use zdc_hir::{DefId, DefKind, Hir, LocalId};

use crate::analysis::Analysis;

/// Names a program may not take, because JavaScript has them.
const RESERVED: &[&str] = &[
    "arguments",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "eval",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

/// Names a program may not take, because the emission already uses them
/// at module scope and they do not begin with `$`.
///
/// The guarantee at the top of this file — a generated name cannot collide
/// with a user name — holds for everything the *compiler* invents, because
/// all of it is `$`-prefixed. It never held for the names the emission
/// *imports*: `import { bindText } from './runtime/dom.js'` and
/// `const [bindText] = signal(1)` are two declarations of one binding, and
/// the bundle a program declaring `state bindText` produced would not load
/// at all.
///
/// `main` and `container` are here for the same reason and one is worse:
/// `export function main(container)` is the module's entry point, so
/// `state main` redeclares it, and `state container` is *shadowed* by the
/// parameter inside `main`'s body — a program showing it rendered the host
/// element with no diagnostic anywhere.
///
/// `rpc.js` and `store.js` are absent deliberately: every symbol from
/// those two is imported under a `$` alias (`call as $call`), so they
/// cannot collide and reserving them would spend names for nothing.
/// `the_reserved_set_covers_every_unaliased_runtime_import` is what keeps
/// that claim, and this list, honest.
const EMITTED: &[&str] = &[
    // The module's own entry point and its parameter.
    "main",
    "container",
    // `signal.js`, unaliased.
    "derived",
    // `$numberField` writes a property rather than an attribute, so it
    // allocates its own effect instead of going through `bindAttr`.
    "effect",
    "signal",
    // `dom.js`, unaliased.
    "anchors",
    "bindAttr",
    "bindMarkup",
    "bindStyle",
    "bindText",
    "markup",
    "mount",
    "on",
    "safeUrl",
    "template",
    "variant",
    // `branch.js`, unaliased.
    "ifInto",
    "whenInto",
    // `adopt.js`, unaliased.
    "adopt",
    // `list.js`, unaliased.
    "eachInto",
    // §16.3.6 writes the two-way sugar's listener as
    // `e => set(e.target.value)`, and the worked emissions are golden
    // tested against it. A program declaring `state e` used to have its
    // own signal shadowed by that parameter inside every `Input`'s
    // listener.
    "e",
];

/// Every JavaScript identifier the emission uses for a source-level name.
pub struct Names {
    defs: HashMap<DefId, String>,
    setters: HashMap<DefId, String>,
    locals: HashMap<LocalId, String>,
    local_setters: HashMap<LocalId, String>,
}

impl Names {
    /// Allocate one identifier per definition, one setter per definition
    /// that is written, and one per local.
    ///
    /// Locals get module-unique names rather than relying on JavaScript's
    /// scoping. Shadowing is legal in both languages, but a local that
    /// happens to share a spelling with a top-level definition it also
    /// *reads* would silently read itself. It is also what keeps two
    /// instances of one component apart: each instance's state is a
    /// distinct local, so `open` in one and `open` in the other are two
    /// identifiers and two signals (§14D.1).
    ///
    /// `client_members` is the split's client root. A signal outside it is
    /// written by a generated command rather than by the browser, so it
    /// has no cell here and needs no setter.
    pub fn new(hir: &Hir, analysis: &Analysis, client_members: &BTreeSet<DefId>) -> Names {
        let written = analysis.written();
        let mut taken: HashSet<String> = RESERVED
            .iter()
            .chain(EMITTED)
            .map(|s| (*s).to_string())
            .collect();
        let mut defs = HashMap::new();
        let mut setters = HashMap::new();
        let mut locals = HashMap::new();
        let mut local_setters = HashMap::new();

        // The program's own names are allocated before the library's, in
        // both passes. Both sets are in one arena with the library first
        // (§17.4.1), so naming in arena order would let a prelude binder
        // called `item` take `item` and push the programmer's own loop
        // variable to `item$` — in generated code they will read when
        // something goes wrong. A degraded name belongs on the definition
        // nobody wrote.
        let user_first = |a: DefId, b: DefId| {
            hir.is_prelude_def(a)
                .cmp(&hir.is_prelude_def(b))
                .then(a.cmp(&b))
        };
        let mut ordered: Vec<DefId> = hir.defs.iter().map(|(id, _)| id).collect();
        ordered.sort_by(|a, b| user_first(*a, *b));

        for id in ordered {
            let def = &hir.defs[id];
            if matches!(def.kind, DefKind::View(_) | DefKind::Component(_)) {
                // The view is a root, and a component is written out at
                // each of its call sites; neither is a referenced name.
                continue;
            }
            // A test's definition name is the sentence it claims, which is
            // prose and not an identifier — `fresh` would hand back
            // `doubling four gives eight` and the module would not parse
            // (issue #169). It gets a generated name instead, and the `$`
            // prefix does the same work it does everywhere else in this
            // file: it is outside XID, so no program can collide with it.
            //
            // Nothing refers to a test by name anyway; the `$tests` array
            // is keyed by position and carries the claim as a string.
            let name = if is_test(def) {
                let generated = format!("$test{}", defs.len());
                taken.insert(generated.clone());
                generated
            } else {
                fresh(&def.name, &mut taken)
            };
            if matches!(def.kind, DefKind::Signal(_))
                && written.contains(&id)
                && client_members.contains(&id)
            {
                let setter = fresh(&setter_of(&name), &mut taken);
                setters.insert(id, setter);
            }
            defs.insert(id, name);
        }

        // The same ordering, for the same reason: a prelude binder must
        // not take a name the programmer's own binder wanted.
        let mut binders: Vec<LocalId> = hir.locals.iter().map(|(id, _)| id).collect();
        binders.sort_by_key(|id| (hir.is_prelude_local(*id), *id));
        for id in binders {
            // A component declaration's own binders are never emitted:
            // instantiation copied them per call site. Naming them would
            // spend `count` on something nothing refers to and leave the
            // instance that is emitted calling itself `count$`.
            if analysis.is_declaration_local(id) {
                continue;
            }
            let name = fresh(&hir.locals[id].name, &mut taken);
            if analysis.is_local_signal(id) && analysis.written_locals().contains(&id) {
                let setter = fresh(&setter_of(&name), &mut taken);
                local_setters.insert(id, setter);
            }
            locals.insert(id, name);
        }

        Names {
            defs,
            setters,
            locals,
            local_setters,
        }
    }

    /// The identifier a definition is emitted under.
    ///
    /// Every `Res::Def` in the HIR came from resolution, so a missing entry
    /// is a compiler bug rather than a program error.
    pub fn def(&self, id: DefId) -> &str {
        self.defs
            .get(&id)
            .map(String::as_str)
            .expect("every referenced definition was named")
    }

    /// The identifier a definition's setter is emitted under, if it is
    /// ever written.
    pub fn setter(&self, id: DefId) -> Option<&str> {
        self.setters.get(&id).map(String::as_str)
    }

    pub fn local(&self, id: LocalId) -> &str {
        self.locals
            .get(&id)
            .map(String::as_str)
            .expect("every binder was named")
    }

    /// The setter for a component's own state, if anything writes to it.
    pub fn local_setter(&self, id: LocalId) -> Option<&str> {
        self.local_setters.get(&id).map(String::as_str)
    }
}

/// Whether this definition came from a `test` declaration — issue #169.
fn is_test(def: &zdc_hir::Def) -> bool {
    match &def.kind {
        DefKind::Signal(signal) => signal.expectation.is_some(),
        DefKind::Function(_)
        | DefKind::View(_)
        | DefKind::Record(_)
        | DefKind::Choice(_)
        | DefKind::Component(_)
        | DefKind::Foreign(_)
        | DefKind::Release(_) => false,
    }
}

/// `count` -> `setCount`, per §16.3.2.
fn setter_of(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        None => "set".to_string(),
        Some(first) => format!(
            "set{}{}",
            first.to_uppercase().collect::<String>(),
            chars.as_str()
        ),
    }
}

/// The first form of `candidate` nobody has taken: `x`, then `x$`, then
/// `x$2`, `x$3`, …
///
/// `$` is outside XID, so a suffixed name can never be one a program could
/// have written, and the search therefore always terminates on a fresh
/// name.
///
/// The counter matters now that the prelude is compiled with every program
/// (§17.4.1). Fifteen of its functions take a parameter called `value`, and
/// appending one `$` per collision made the fifteenth `value` plus fourteen
/// dollar signs — quadratic in the bundle and unreadable in a stack trace.
fn fresh(candidate: &str, taken: &mut HashSet<String>) -> String {
    if taken.insert(candidate.to_string()) {
        return candidate.to_string();
    }
    let marked = format!("{candidate}$");
    if taken.insert(marked.clone()) {
        return marked;
    }
    for suffix in 2.. {
        let name = format!("{candidate}${suffix}");
        if taken.insert(name.clone()) {
            return name;
        }
    }
    unreachable!("the suffix range is unbounded")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setter_upper_cases_the_first_character_only() {
        assert_eq!(setter_of("count"), "setCount");
        assert_eq!(setter_of("myValue"), "setMyValue");
        assert_eq!(setter_of("x"), "setX");
    }

    #[test]
    fn a_reserved_word_is_suffixed_rather_than_emitted() {
        let mut taken: HashSet<String> = RESERVED.iter().map(|s| (*s).to_string()).collect();
        assert_eq!(fresh("class", &mut taken), "class$");
        assert_eq!(fresh("class", &mut taken), "class$2");
        assert_eq!(fresh("class", &mut taken), "class$3");
        assert_eq!(fresh("count", &mut taken), "count");
    }

    /// Every symbol the emission asks for by an unaliased name is
    /// reserved, and every symbol it asks for under an alias is aliased
    /// with `$`.
    ///
    /// The set is read out of the emitter's own modules rather than
    /// restated here, because the failure this exists to catch is a *new*
    /// emission site importing a *new* name: that site writes one more
    /// `used.dom.insert("…")` and nothing else in the compiler changes.
    /// A list maintained by hand would still say what it said yesterday.
    #[test]
    fn the_reserved_set_covers_every_unaliased_runtime_import() {
        let mut scanned = 0;
        for (file, source) in emitter_sources() {
            for line in source.lines() {
                // A doc comment naming the marker is prose about the
                // scan, not a site the scan is about.
                if line.trim_start().starts_with("//") {
                    continue;
                }
                for (module, aliased) in [
                    ("signal", false),
                    ("dom", false),
                    ("rpc", true),
                    ("store", true),
                ] {
                    let marker = format!(".{module}.insert(\"");
                    let Some((_, rest)) = line.split_once(&marker) else {
                        continue;
                    };
                    let Some((symbol, _)) = rest.split_once('"') else {
                        continue;
                    };
                    scanned += 1;
                    match aliased {
                        // `call as $call`: a `$` alias cannot collide with
                        // a UAX#31 name, so it needs no reservation — but
                        // a plain one would be a hole this list misses.
                        true => assert!(
                            symbol.contains(" as $"),
                            "{file} imports `{symbol}` from `{module}.js` unaliased and \
                             unreserved"
                        ),
                        false => assert!(
                            EMITTED.contains(&symbol),
                            "{file} imports `{symbol}` from `{module}.js`, which a program \
                             could declare"
                        ),
                    }
                }
            }
        }
        assert!(
            scanned >= 15,
            "only {scanned} runtime imports were found; the scan read nothing"
        );
    }

    /// Every name this list reserves from a runtime module is one that
    /// module really exports, so a typo reserves nothing and says so.
    #[test]
    fn every_reserved_runtime_name_is_a_runtime_export() {
        // **Every module a bundle may import, not the two it used to be.**
        // `bindMarkup` was reserved here and exported by `dom.js` until the
        // size gate moved it to `markup.js`, and this scan — which named
        // its modules one by one — went on reading the two it knew and
        // reported a reserved name the runtime no longer exported. A
        // module list written out by hand is a list that stops being the
        // whole list; this one is the same set `Bundle::runtime` links,
        // so a new runtime module has to be added in both places or the
        // emission fails rather than this test.
        let exported: Vec<String> = [
            zdc_runtime::SIGNAL_JS,
            zdc_runtime::DOM_JS,
            zdc_runtime::FOREIGN_JS,
            zdc_runtime::MARKUP_JS,
            zdc_runtime::LIST_JS,
            zdc_runtime::BRANCH_JS,
            zdc_runtime::ADOPT_JS,
            zdc_runtime::REQUEST_JS,
            zdc_runtime::RPC_JS,
            zdc_runtime::WIRE_JS,
            zdc_runtime::STORE_JS,
        ]
        .iter()
        .flat_map(|source| {
            source.lines().filter_map(|line| {
                let rest = line.strip_prefix("export function ")?;
                let name: String = rest
                    .chars()
                    .take_while(char::is_ascii_alphanumeric)
                    .collect();
                (!name.is_empty()).then_some(name)
            })
        })
        .collect();
        assert!(exported.len() >= 20, "the export scan read nothing");
        for name in EMITTED {
            // `main`, `container` and `e` are the emission's own, not the
            // runtime's.
            if matches!(*name, "main" | "container" | "e") {
                continue;
            }
            assert!(
                exported.iter().any(|found| found == name),
                "`{name}` is reserved as a runtime import but no runtime module exports it"
            );
        }
    }

    fn emitter_sources() -> Vec<(String, String)> {
        let directory = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
        let mut found = Vec::new();
        for entry in std::fs::read_dir(directory).expect("the emitter's own sources") {
            let path = entry.expect("a directory entry").path();
            if path.extension().is_some_and(|ext| ext == "rs") {
                let name = path.display().to_string();
                found.push((name, std::fs::read_to_string(&path).expect("a source file")));
            }
        }
        assert!(found.len() >= 10, "the emitter has more modules than this");
        found
    }

    /// A signal `count` reserves `setCount` as well as `count`, so a
    /// program declaring `setCount` gets a distinct identifier rather than
    /// two declarations of the same one.
    #[test]
    fn a_setter_and_a_program_name_never_collide() {
        let mut taken = HashSet::new();
        let count = fresh("count", &mut taken);
        let setter = fresh(&setter_of(&count), &mut taken);
        let user = fresh("setCount", &mut taken);
        assert_eq!(setter, "setCount");
        assert_ne!(user, setter);
    }
}
