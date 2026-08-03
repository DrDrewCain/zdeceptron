//! Binding `$env` and `$store`, and running the handler.
//!
//! # How Rust state reaches a native function without `unsafe`
//!
//! `#![forbid(unsafe_code)]` is not negotiable here (§7.5 calls the
//! commitment "mechanical, not aspirational"), and the engine's only safe
//! way to build a native function is [`NativeFunction::from_copy_closure`],
//! whose `Copy` bound rules out capturing an `Arc<dyn DurableStore>`. The
//! `unsafe` constructors that would take one are exactly what is
//! forbidden.
//!
//! So the closures capture a `u64` — which is `Copy` — and that number is
//! a ticket into a registry the invocation owns. [`Ticket`] puts the entry
//! in on the way in and takes it out on the way out, including on a panic,
//! so a failed invocation cannot leave a store behind for the next one.
//!
//! # Why the store shim is written in JavaScript
//!
//! Only five operations cross into Rust: `get`, `set`, `incr`, `delete`
//! and reading an environment key. `$store.decr`, `$store.append` and
//! `$store.remove` — three of §14B.2's five mutation verbs — are derived
//! from those in the prelude below. That is the same division the store
//! crate documents: §7.4's five operations are the interface every backing
//! store must implement, and §18.2's five verbs are the wire contract. A
//! `decr` in Rust would be a sixth thing DynamoDB, Deno KV and a Durable
//! Object each have to grow.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use boa_engine::{Context, JsNativeError, JsString, JsValue, NativeFunction, Source};
use zdc_store::{DurableStore, Json, Number, StoreError};

use crate::endpoint::{Endpoint, Shape};
use crate::env::Environment;
use crate::HostError;

/// What one invocation's native functions can reach.
struct Bound {
    store: Arc<dyn DurableStore>,
    env: Environment,
}

/// The live invocations, keyed by ticket.
///
/// A `Mutex<HashMap<..>>` rather than a thread-local, because a request
/// runs on whichever thread the server handed it and two requests run at
/// once; a thread-local would work today and break the moment an engine
/// job moved.
fn registry() -> &'static Mutex<HashMap<u64, Arc<Bound>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<u64, Arc<Bound>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// An entry in the registry that removes itself.
struct Ticket(u64);

impl Ticket {
    fn issue(bound: Bound) -> Ticket {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        registry()
            .lock()
            .expect("the host registry is poisoned")
            .insert(id, Arc::new(bound));
        Ticket(id)
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        registry()
            .lock()
            .expect("the host registry is poisoned")
            .remove(&self.0);
    }
}

/// The bindings for a ticket, or a JavaScript error if the invocation is
/// already over — which can only happen if a handler stashed a reference
/// to `$store` somewhere that outlived the request.
fn bound(id: u64) -> Result<Arc<Bound>, boa_engine::JsError> {
    registry()
        .lock()
        .expect("the host registry is poisoned")
        .get(&id)
        .cloned()
        .ok_or_else(|| {
            JsNativeError::error()
                .with_message("this invocation has already finished")
                .into()
        })
}

/// A store failure, in the words the store chose.
///
/// `NotANumber` and `OutOfRange` name the key and what was found; flattening
/// them to "store error" here would throw away the only part a developer
/// can act on.
fn store_failure(error: StoreError) -> boa_engine::JsError {
    JsNativeError::error()
        .with_message(error.to_string())
        .into()
}

/// The `n`th argument, or `undefined` — the same thing JavaScript itself
/// hands a parameter the caller omitted.
fn argument(args: &[JsValue], index: usize) -> JsValue {
    args.get(index).cloned().unwrap_or(JsValue::undefined())
}

fn as_key(value: &JsValue, context: &mut Context) -> Result<String, boa_engine::JsError> {
    Ok(value.to_string(context)?.to_std_string_escaped())
}

/// Bind the two names a function bundle leaves free.
fn install(context: &mut Context, ticket: u64) -> Result<(), boa_engine::JsError> {
    context.register_global_callable(
        JsString::from("$zdEnv"),
        1,
        NativeFunction::from_copy_closure(move |_this, args, context| {
            let bound = bound(ticket)?;
            let key = as_key(&argument(args, 0), context)?;
            match bound.env.get(&key) {
                Some(value) => Ok(JsValue::from(JsString::from(value))),
                // Not the empty string. A missing secret that reads as ""
                // produces a well-formed unauthorised request and the
                // upstream service gets the blame.
                None => Err(JsNativeError::error()
                    .with_message(format!(
                        "`{key}` is not set in this environment; the program declares it with \
                         `from environment {key:?}`"
                    ))
                    .into()),
            }
        }),
    )?;

    context.register_global_callable(
        JsString::from("$zdGet"),
        1,
        NativeFunction::from_copy_closure(move |_this, args, context| {
            let bound = bound(ticket)?;
            let key = as_key(&argument(args, 0), context)?;
            match bound.store.get(&key).map_err(store_failure)? {
                Some(json) => Ok(JsValue::from(JsString::from(json.as_str()))),
                None => Ok(JsValue::null()),
            }
        }),
    )?;

    context.register_global_callable(
        JsString::from("$zdSet"),
        2,
        NativeFunction::from_copy_closure(move |_this, args, context| {
            let bound = bound(ticket)?;
            let key = as_key(&argument(args, 0), context)?;
            let json = as_key(&argument(args, 1), context)?;
            bound
                .store
                .set(&key, Json::from_text(json))
                .map_err(store_failure)?;
            Ok(JsValue::undefined())
        }),
    )?;

    context.register_global_callable(
        JsString::from("$zdIncr"),
        2,
        NativeFunction::from_copy_closure(move |_this, args, context| {
            let bound = bound(ticket)?;
            let key = as_key(&argument(args, 0), context)?;
            let delta = argument(args, 1).to_number(context)?;
            let (value, _) = bound
                .store
                .incr(&key, Number::new(delta))
                .map_err(store_failure)?;
            Ok(JsValue::from(value.as_f64()))
        }),
    )?;

    context.register_global_callable(
        JsString::from("$zdDelete"),
        1,
        NativeFunction::from_copy_closure(move |_this, args, context| {
            let bound = bound(ticket)?;
            let key = as_key(&argument(args, 0), context)?;
            bound.store.delete(&key).map_err(store_failure)?;
            Ok(JsValue::undefined())
        }),
    )?;

    Ok(())
}

/// `$env` and `$store`, spelled the way the emitted code spells them.
///
/// The five mutation verbs of §18.2 all appear, because all five are names
/// `zdc-codegen` can print; three of them are derived rather than crossing
/// into Rust. `append` and `remove` read before they write, which is a
/// race the store cannot close for them — see the note in `PRELUDE`
/// itself.
const PRELUDE: &str = r#"
const $env = (key) => $zdEnv(key);

const $store = {
  get(key) {
    const text = $zdGet(key);
    return text === null ? undefined : JSON.parse(text);
  },
  set(key, value) {
    $zdSet(key, JSON.stringify(value === undefined ? null : value));
    return value;
  },
  incr(key, delta) {
    return $zdIncr(key, delta);
  },
  // `decr` is `incr` of a negation. The store carries five operations, not
  // six (§7.4), and the five wire verbs (§18.2) are a different five.
  decr(key, delta) {
    return $zdIncr(key, -delta);
  },
  // Read-modify-write, and therefore NOT atomic the way `incr` is: two
  // concurrent appends to one key can lose one. `incr` is atomic because
  // the store implements it as one transaction; a general list append
  // would need either a compare-and-set loop or an operation every backing
  // store has to grow. This is a real limit, and it is written here rather
  // than discovered later.
  append(key, item) {
    const current = $store.get(key);
    const list = current === undefined ? [] : current;
    if (!Array.isArray(list)) {
      throw new Error('`' + key + '` does not hold a list, so `append` cannot add to it');
    }
    const next = list.concat([item]);
    $zdSet(key, JSON.stringify(next));
    return next;
  },
  remove(key, item) {
    const current = $store.get(key);
    const list = current === undefined ? [] : current;
    if (!Array.isArray(list)) {
      throw new Error('`' + key + '` does not hold a list, so `remove` cannot take from it');
    }
    const next = list.filter((entry) => entry !== item);
    $zdSet(key, JSON.stringify(next));
    return next;
  },
  delete(key) {
    $zdDelete(key);
  },
};
"#;

/// A JavaScript single-quoted string literal.
///
/// The request body is spliced into the driver as a *literal* and parsed
/// by `JSON.parse` there, never evaluated as source. U+2028 and U+2029 are
/// escaped because they end a line in JavaScript even inside a string, so
/// a body containing one would close the literal and the rest of the body
/// would be parsed as program text.
fn js_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for c in value.chars() {
        if c == '\\' {
            out.push_str("\\\\");
        } else if c == '\'' {
            out.push_str("\\'");
        } else if c == '\n' {
            out.push_str("\\n");
        } else if c == '\r' {
            out.push_str("\\r");
        } else if c == '\u{2028}' {
            out.push_str("\\u2028");
        } else if c == '\u{2029}' {
            out.push_str("\\u2029");
        } else if (c as u32) < 0x20 {
            out.push_str(&format!("\\u{:04x}", c as u32));
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Strip `export ` so a module evaluates as a script.
///
/// A function bundle has no imports by construction (§16.3.12 invariant
/// 4), so this is the whole of the module syntax it can contain — which is
/// what lets the exact shipped bytes run here rather than a rewritten
/// copy.
fn as_script(source: &str) -> String {
    source
        .lines()
        .map(|line| line.strip_prefix("export ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The call expression that shapes the request body into the handler's
/// one parameter.
fn call_expression(endpoint: &Endpoint) -> String {
    match endpoint.shape {
        Shape::Command => "handler($zdArgs)".to_string(),
        Shape::Value => {
            let fields: Vec<String> = endpoint
                .inputs
                .iter()
                .enumerate()
                .map(|(index, name)| format!("{}: $zdArgs[{index}]", js_property(name)))
                .collect();
            format!("handler({{ {} }})", fields.join(", "))
        }
    }
}

/// A property name, quoted. The emitter's identifiers are already legal
/// JavaScript, but quoting costs nothing and means a future renaming
/// scheme cannot produce a syntax error here.
fn js_property(name: &str) -> String {
    js_string(name)
}

/// Run one endpoint.
pub fn run(
    endpoint: &Endpoint,
    store: &Arc<dyn DurableStore>,
    env: &Environment,
    arguments_json: &str,
) -> Result<String, HostError> {
    let ticket = Ticket::issue(Bound {
        store: Arc::clone(store),
        env: env.clone(),
    });

    let mut context = Context::default();
    let failed = |message: String| HostError::Failed {
        endpoint: endpoint.name.clone(),
        message,
    };

    install(&mut context, ticket.0).map_err(|e| failed(e.to_string()))?;
    context
        .eval(Source::from_bytes(PRELUDE.as_bytes()))
        .map_err(|e| failed(format!("the adapter prelude did not evaluate: {e}")))?;
    context
        .eval(Source::from_bytes(as_script(&endpoint.source).as_bytes()))
        .map_err(|e| failed(format!("the emitted function did not evaluate: {e}")))?;

    let driver = format!(
        r#"
globalThis.$zdOut = null;
globalThis.$zdErr = null;
globalThis.$zdBad = null;
globalThis.$zdSettled = false;
(function () {{
  const $zdMessage = (e) => String(e && e.message ? e.message : e);
  let $zdArgs;
  try {{
    $zdArgs = JSON.parse({body});
  }} catch (e) {{
    $zdBad = 'the request body is not JSON';
    return;
  }}
  if (!Array.isArray($zdArgs)) {{
    $zdBad = 'the request body must be an array of arguments';
    return;
  }}
  if ({checks_arity} && $zdArgs.length !== {arity}) {{
    $zdBad = 'this endpoint takes {arity} argument(s), and the request carried ' + $zdArgs.length;
    return;
  }}
  let $zdPending;
  try {{
    $zdPending = {call};
  }} catch (e) {{
    $zdSettled = true;
    $zdErr = $zdMessage(e);
    return;
  }}
  Promise.resolve($zdPending).then(
    (value) => {{
      $zdSettled = true;
      const text = JSON.stringify(value === undefined ? null : value);
      $zdOut = text === undefined ? 'null' : text;
    }},
    (e) => {{
      $zdSettled = true;
      $zdErr = $zdMessage(e);
    }}
  );
}})();
"#,
        body = js_string(arguments_json),
        call = call_expression(endpoint),
        arity = endpoint.inputs.len(),
        // A command's arity is not fixed by the manifest: the number of
        // index arguments comes from the place being written, and the
        // handler reads `$args[n]` for exactly as many as it needs.
        checks_arity = matches!(endpoint.shape, Shape::Value),
    );

    context
        .eval(Source::from_bytes(driver.as_bytes()))
        .map_err(|e| failed(format!("the invocation did not start: {e}")))?;

    // Every `await` in the handler is a job. Without this the handler has
    // begun and nothing has finished, which is precisely the failure mode
    // "the compiler emitted it" hides.
    context
        .run_jobs()
        .map_err(|e| failed(format!("an awaited operation failed: {e}")))?;

    let mut read = |name: &str| -> Result<JsValue, HostError> {
        context
            .eval(Source::from_bytes(name.as_bytes()))
            .map_err(|e| HostError::Failed {
                endpoint: endpoint.name.clone(),
                message: format!("could not read `{name}` back: {e}"),
            })
    };

    if let Some(message) = read("$zdBad")?.as_string() {
        return Err(HostError::BadRequest {
            message: message.to_std_string_escaped(),
        });
    }
    if let Some(message) = read("$zdErr")?.as_string() {
        return Err(failed(message.to_std_string_escaped()));
    }
    if !read("$zdSettled")?.to_boolean() {
        // A handler that never settles is a bug in the emitted code or in
        // an adapter binding, and returning `Loading` for ever is the one
        // outcome a `Remote of T` cannot recover from.
        return Err(failed(
            "the handler never settled: it returned a promise nothing resolves".to_string(),
        ));
    }
    match read("$zdOut")?.as_string() {
        Some(json) => Ok(json.to_std_string_escaped()),
        None => Ok("null".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Evaluate an expression and return it as text, or the engine's
    /// complaint. Used to check escaping against the parser rather than
    /// against a second copy of the escaping rules.
    fn evaluate(expression: &str) -> Result<String, String> {
        let mut context = Context::default();
        match context.eval(Source::from_bytes(expression.as_bytes())) {
            Ok(value) => Ok(value.display().to_string()),
            Err(error) => Err(error.to_string()),
        }
    }

    #[test]
    fn a_body_cannot_escape_the_literal_it_is_spliced_into() {
        // The request body is attacker-controlled. If it were concatenated
        // into source rather than escaped, `'); <anything>; ('` would run.
        // Checked by evaluating it: an assertion about the escaped text
        // would only be restating the escaping function.
        let hostile = "'); globalThis.$owned = 1; ('";
        let literal = js_string(hostile);
        assert_eq!(
            evaluate(&format!(
                "globalThis.$owned = 0; const $b = {literal}; $b.length + ':' + $owned"
            )),
            Ok(format!("\"{}:0\"", hostile.encode_utf16().count())),
            "the body did not survive as one inert string"
        );
    }

    #[test]
    fn a_line_separator_in_a_body_does_not_end_the_literal() {
        // U+2028 terminates a line in JavaScript even inside a string, so
        // an unescaped one closes the literal mid-body.
        let literal = js_string("a\u{2028}b");
        assert_eq!(literal, "'a\\u2028b'");
    }

    #[test]
    fn stripping_export_leaves_the_declaration_untouched() {
        assert_eq!(
            as_script("export async function handler({ a }) {\n  return a;\n}"),
            "async function handler({ a }) {\n  return a;\n}"
        );
        assert_eq!(as_script("  const x = 1;"), "  const x = 1;");
    }

    #[test]
    fn a_value_endpoint_is_called_with_a_named_object() {
        let endpoint = Endpoint {
            name: "greeting".to_string(),
            shape: Shape::Value,
            inputs: vec!["name".to_string(), "count".to_string()],
            source: String::new(),
        };
        assert_eq!(
            call_expression(&endpoint),
            "handler({ 'name': $zdArgs[0], 'count': $zdArgs[1] })"
        );
    }

    #[test]
    fn a_command_endpoint_is_called_with_the_array_itself() {
        let endpoint = Endpoint {
            name: "visits.incr".to_string(),
            shape: Shape::Command,
            inputs: Vec::new(),
            source: String::new(),
        };
        assert_eq!(call_expression(&endpoint), "handler($zdArgs)");
    }
}
