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
        let mut taken: HashSet<String> = RESERVED.iter().map(|s| (*s).to_string()).collect();
        // The one emitted name that is not `$`-prefixed: §16.3.6 writes the
        // two-way sugar's listener as `e => set(e.target.value)` and the
        // worked emissions are golden-tested against it. Reserving the name
        // is what keeps the guarantee at the top of this file true — a
        // program declaring `state e` used to have its own signal shadowed
        // by that parameter inside every `Input`'s listener.
        taken.insert("e".to_string());
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
            let name = fresh(&def.name, &mut taken);
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
