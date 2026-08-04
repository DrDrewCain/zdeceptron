//! What the host was handed to run.
//!
//! This is deliberately a mirror of `zdc_codegen::ServerFunction` rather
//! than a re-export of it. The host executes *files*, and a file can reach
//! it from a `dist/` directory that some other build produced — so the
//! thing it runs must be describable without the compiler being present.
//! `zdc-dev` builds one of these straight from the emitter's output; a
//! deployment builds one from `manifest.json`.

use std::collections::BTreeMap;

/// The two handler signatures a build can emit.
///
/// Guessing between them is not a recoverable mistake: passing an array
/// where an object is destructured binds every input to `undefined` and
/// the handler returns a plausible wrong answer instead of failing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// `handler({ a, b })` — the browser reads a `server` or `durable`
    /// signal, and named its inputs.
    Value,
    /// `handler($args)` — the browser asked for a write. The arguments are
    /// the right-hand side and the indexes of the place, evaluated where
    /// the write was written (§17.2.7), and they have no names here.
    Command,
}

impl Shape {
    /// Read the word `manifest.json` carries.
    pub fn parse(word: &str) -> Option<Shape> {
        match word {
            "value" => Some(Shape::Value),
            "command" => Some(Shape::Command),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Shape::Value => "value",
            Shape::Command => "command",
        }
    }
}

/// One runnable server function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// The name the browser posts to, such as `visits.incr`.
    pub name: String,
    pub shape: Shape,
    /// The wire order of the inputs, for [`Shape::Value`]. Empty for a
    /// command.
    pub inputs: Vec<String>,
    /// The emitted JavaScript, verbatim. Not a path: the dev server never
    /// writes a `dist/`, and reading the bytes from disk when they are
    /// already in memory is a second source of truth.
    pub source: String,
}

/// Every endpoint a build emitted, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Endpoints {
    by_name: BTreeMap<String, Endpoint>,
}

impl Endpoints {
    pub fn insert(&mut self, endpoint: Endpoint) {
        self.by_name.insert(endpoint.name.clone(), endpoint);
    }

    pub fn get(&self, name: &str) -> Option<&Endpoint> {
        self.by_name.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }
}

impl FromIterator<Endpoint> for Endpoints {
    fn from_iter<I: IntoIterator<Item = Endpoint>>(iter: I) -> Endpoints {
        let mut endpoints = Endpoints::default();
        for endpoint in iter {
            endpoints.insert(endpoint);
        }
        endpoints
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shape_round_trips_through_the_word_the_manifest_carries() {
        for shape in [Shape::Value, Shape::Command] {
            assert_eq!(Shape::parse(shape.word()), Some(shape));
        }
    }

    #[test]
    fn an_unknown_shape_word_is_refused_rather_than_defaulted() {
        // Defaulting to `Value` would turn a manifest this host does not
        // understand into an endpoint that silently mis-binds its inputs.
        assert_eq!(Shape::parse("values"), None);
        assert_eq!(Shape::parse(""), None);
    }

    #[test]
    fn endpoints_are_addressed_by_the_name_the_browser_posts() {
        let endpoints = Endpoints::from_iter([Endpoint {
            name: "visits.incr".to_string(),
            shape: Shape::Command,
            inputs: Vec::new(),
            source: String::new(),
        }]);
        assert!(endpoints.get("visits.incr").is_some());
        assert!(endpoints.get("visits").is_none(), "not a prefix match");
    }
}
