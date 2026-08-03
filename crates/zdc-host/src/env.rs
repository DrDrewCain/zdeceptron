//! `$env` — the other name a function bundle leaves free.
//!
//! # Why this is a lookup and not a substitution
//!
//! `secret state apiKey is server Text from environment "GREETING_API_KEY"`
//! emits `$env('GREETING_API_KEY')` into the server file. It would have
//! been less work to substitute the *value* at build time, and that is
//! precisely the mistake this type exists to make impossible:
//!
//! - A build artifact would then contain the secret, and artifacts are
//!   committed, cached, uploaded to registries and copied into images.
//! - Rotating the key would require a rebuild.
//! - §16.3.12 assertion C says the manifest may carry endpoint names,
//!   input orders and durable keys — **never an `environment` key name**,
//!   let alone a value. The key name appears only in the server file; the
//!   value appears in no file at all.
//!
//! So the compiler emits the *name*, and this resolves it at invocation
//! time from configuration the deployment owns.
//!
//! # An absent key is an error, not an empty string
//!
//! A missing secret that reads as `""` produces a request that is
//! well-formed, unauthorised, and blamed on the upstream service. It
//! throws instead, and the throw becomes a `Failed` in the browser's
//! `Remote of T` — which is exactly where a developer will look.

use std::collections::BTreeMap;

/// Where `$env` reads from.
#[derive(Debug, Clone, Default)]
pub struct Environment {
    values: BTreeMap<String, String>,
}

impl Environment {
    /// An environment with nothing in it. Every `$env` call fails.
    pub fn empty() -> Environment {
        Environment::default()
    }

    /// Configuration supplied by the caller — a `.env` file the dev server
    /// read, or a deployment's secret store.
    pub fn from_pairs(
        pairs: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Environment {
        Environment {
            values: pairs
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        }
    }

    /// The process environment, which is what a serverless platform sets.
    ///
    /// Every variable is taken rather than a filtered subset: the filter
    /// is the program, which can only ask for the keys it declared, and a
    /// second allow-list here would be one more place for a deployment to
    /// be wrong in a way that only shows up in production.
    pub fn from_process() -> Environment {
        Environment {
            values: std::env::vars().collect(),
        }
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.values.insert(key.into(), value.into());
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_configured_key_reads_back() {
        let env = Environment::from_pairs([("GREETING_API_KEY", "sk-test")]);
        assert_eq!(env.get("GREETING_API_KEY"), Some("sk-test"));
    }

    #[test]
    fn an_absent_key_is_absent_rather_than_empty() {
        // The distinction is the whole point: `Some("")` would let an
        // unconfigured deployment send unauthorised requests that look
        // like the upstream service's fault.
        let env = Environment::empty();
        assert_eq!(env.get("GREETING_API_KEY"), None);
    }
}
