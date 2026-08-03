use std::collections::HashMap;
use zdc_hir::LocalId;

/// A stack of lexical scopes for body-local bindings.
///
/// Top-level declarations are deliberately NOT held here. They are
/// order-independent — a signal may read one declared further down the
/// file, because the signal graph is a graph rather than a sequence — so
/// they live in a flat table built by a separate collection pass. Only
/// bindings with a lexical extent (parameters, loop variables, pattern
/// binders) are pushed and popped.
#[derive(Debug, Default)]
pub struct Scopes {
    frames: Vec<HashMap<String, LocalId>>,
}

impl Scopes {
    pub fn new() -> Self {
        Scopes { frames: Vec::new() }
    }

    pub fn push(&mut self) {
        self.frames.push(HashMap::new());
    }

    pub fn pop(&mut self) {
        self.frames.pop();
    }

    /// Bind a name in the innermost scope.
    ///
    /// Declaring with no scope open is a compiler bug rather than a
    /// program error: the binding would vanish and the name would later
    /// be reported as undefined, with the real cause nowhere in the
    /// message. Assert it in debug builds so a missing `push` fails where
    /// it happened.
    pub fn declare(&mut self, name: &str, id: LocalId) {
        debug_assert!(
            !self.frames.is_empty(),
            "a binding was declared with no scope open"
        );
        if let Some(frame) = self.frames.last_mut() {
            frame.insert(name.to_string(), id);
        }
    }

    /// Innermost binding wins, so an inner scope shadows an outer one.
    pub fn lookup(&self, name: &str) -> Option<LocalId> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.get(name).copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zdc_hir::{Arena, Local, LocalId};
    use zdc_lexer::Span;

    fn local(arena: &mut Arena<LocalId, Local>, name: &str) -> LocalId {
        arena.alloc(Local {
            name: name.to_string(),
            span: Span::new(0, 0),
        })
    }

    #[test]
    fn inner_scope_shadows_outer() {
        let mut arena = Arena::new();
        let outer = local(&mut arena, "x");
        let inner = local(&mut arena, "x");
        let mut scopes = Scopes::new();

        scopes.push();
        scopes.declare("x", outer);
        scopes.push();
        scopes.declare("x", inner);
        assert_eq!(scopes.lookup("x"), Some(inner));

        scopes.pop();
        assert_eq!(scopes.lookup("x"), Some(outer));
    }

    #[test]
    fn popping_removes_the_binding() {
        let mut arena = Arena::new();
        let x = local(&mut arena, "x");
        let mut scopes = Scopes::new();

        scopes.push();
        scopes.declare("x", x);
        scopes.pop();

        assert_eq!(scopes.lookup("x"), None);
    }

    #[test]
    fn unknown_names_are_not_found() {
        let scopes = Scopes::new();
        assert_eq!(scopes.lookup("nothing"), None);
    }

    #[test]
    fn a_sibling_scope_does_not_see_a_binding_from_the_one_before_it() {
        let mut arena = Arena::new();
        let x = local(&mut arena, "x");
        let mut scopes = Scopes::new();

        scopes.push();
        scopes.declare("x", x);
        scopes.pop();
        scopes.push();

        assert_eq!(scopes.lookup("x"), None);
    }
}
