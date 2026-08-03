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
//! crate documents: §7.4's operations are the interface every backing
//! store must implement, and §18.2's five verbs are the wire contract. A
//! `decr` in Rust would be a sixth thing DynamoDB, Deno KV and a Durable
//! Object each have to grow.
//!
//! # `$store` records; the invocation commits
//!
//! The four write bindings below **do not write**. They append to a
//! [`Transaction`] the invocation owns, and [`run_all`] hands the whole
//! thing to [`DurableStore::apply`] once, after every handler has settled.
//! That is what makes a handler all-or-nothing: a throw part way through
//! never reaches the commit, so there is nothing to roll back — the writes
//! were never applied in the first place.
//!
//! Two consequences worth stating rather than discovering.
//!
//! **A failure is reported at the same place it was before.** `incr` on a
//! key holding text still throws from the binding, with the same message,
//! because the projection it computes for its return value consults the
//! store. What changed is that the two writes before it are now also
//! discarded.
//!
//! **`append` and `remove` stopped being a race.** They were
//! read-modify-write in the prelude, and the comment there admitted two
//! concurrent appends could lose one. The read is now *recorded* — it
//! becomes a `check` in the transaction — so a concurrent append is
//! refused with [`StoreError::Conflict`] and re-run, and both land. The
//! re-run is safe because §17.2.7 evaluated the right-hand side in the
//! caller's region: the server half of a command is a pure function of its
//! arguments, so running it twice cannot mean two different things.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use boa_engine::{Context, JsNativeError, JsString, JsValue, NativeFunction, Source};
use zdc_store::{DurableStore, Json, Number, Read, StoreError, Transaction, Write};

use crate::endpoint::{Endpoint, Shape};
use crate::env::Environment;
use crate::HostError;

/// How many times an invocation refused for a conflict is re-run.
///
/// Bounded rather than unbounded: an unbounded retry under sustained
/// contention is a request that never answers, which is worse than one
/// that fails and says why. Eight is generous for the only operations that
/// can conflict at all — `append` and `remove`, the two that read before
/// they write.
const ATTEMPTS: usize = 8;

/// The writes an invocation has asked for, and the reads that justify
/// them.
#[derive(Default)]
struct Pending {
    reads: Vec<Read>,
    writes: Vec<Write>,
    /// What each key holds as far as this invocation is concerned.
    ///
    /// Read-your-own-writes. Without it `append` twice to one key in one
    /// handler would read the pre-transaction list both times and the
    /// second would overwrite the first — a half-apply inside a single
    /// transaction, which is the bug wearing a smaller hat.
    view: HashMap<String, Option<Json>>,
}

impl Pending {
    /// What `key` holds, recording the read the first time it is asked
    /// for.
    ///
    /// A key already in `view` is answered from there and records nothing:
    /// either it was read before — in which case the read is already in
    /// the check set — or this invocation wrote it, in which case the
    /// value is its own and there is nothing to be stale about.
    fn read(
        &mut self,
        store: &Arc<dyn DurableStore>,
        key: &str,
    ) -> Result<Option<Json>, StoreError> {
        if let Some(known) = self.view.get(key) {
            return Ok(known.clone());
        }
        let current = store.get(key)?;
        self.reads.push(Read {
            key: key.to_string(),
            seen: current.clone(),
        });
        self.view.insert(key.to_string(), current.clone());
        Ok(current)
    }

    /// What `key` holds, without recording a read.
    ///
    /// For `incr`'s projected answer only. A blind delta must not join the
    /// check set — two visitors incrementing one key have to *both* be
    /// counted (§18.3), and a recorded read would make one of them
    /// conflict with the other.
    fn peek(&self, store: &Arc<dyn DurableStore>, key: &str) -> Result<Option<Json>, StoreError> {
        match self.view.get(key) {
            Some(known) => Ok(known.clone()),
            None => store.get(key),
        }
    }

    fn into_transaction(self) -> Transaction {
        Transaction {
            reads: self.reads,
            writes: self.writes,
        }
    }
}

/// What one invocation's native functions can reach.
struct Bound {
    store: Arc<dyn DurableStore>,
    env: Environment,
    pending: Mutex<Pending>,
    /// Where a binding leaves text it wants the *server* to read.
    ///
    /// A native function's only channel to the caller is the message it
    /// throws, and that message becomes `error.message` in a browser. A
    /// missing `environment` key has to be reported in two different
    /// words to two different readers, so it is reported twice: a throw
    /// that names no configuration, and this.
    detail: Mutex<Option<String>>,
}

impl Bound {
    fn pending(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.pending
            .lock()
            .expect("the pending transaction is poisoned")
    }
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
    /// Issue a ticket, and hand back the entry it names so the invocation
    /// can take the recorded transaction out at the end without going back
    /// through the registry.
    fn issue(bound: Bound) -> (Ticket, Arc<Bound>) {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let bound = Arc::new(bound);
        registry()
            .lock()
            .expect("the host registry is poisoned")
            .insert(id, Arc::clone(&bound));
        (Ticket(id), bound)
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

/// The server-only text a binding left behind for this invocation, if any.
///
/// `None` once the ticket is gone, which is the same answer as "nothing
/// was left": either way there is nothing for the server to print.
fn detail_of(id: u64) -> Option<String> {
    let bound = bound(id).ok()?;
    let detail = bound.detail.lock().expect("the detail slot is poisoned");
    detail.clone()
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
                None => {
                    // §16.3.12 assertion C. The key *name* is not the
                    // secret's value, and it is still server
                    // configuration: it tells an anonymous caller which
                    // credential this deployment expects and therefore
                    // which service it talks to. It goes to the server's
                    // own console and no further.
                    *bound.detail.lock().expect("the detail slot is poisoned") = Some(format!(
                        "`{key}` is not set in this environment; the program declares it with \
                         `from environment {key:?}`"
                    ));
                    Err(JsNativeError::error()
                        .with_message(
                            "this endpoint reads a value from the environment that is not set on \
                             this host",
                        )
                        .into())
                }
            }
        }),
    )?;

    context.register_global_callable(
        JsString::from("$zdGet"),
        1,
        NativeFunction::from_copy_closure(move |_this, args, context| {
            let bound = bound(ticket)?;
            let key = as_key(&argument(args, 0), context)?;
            // Through `Pending`, so the read joins the transaction's check
            // set and this invocation sees its own earlier writes.
            let value = bound
                .pending()
                .read(&bound.store, &key)
                .map_err(store_failure)?;
            match value {
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
            let value = Json::from_text(json);
            let mut pending = bound.pending();
            pending.view.insert(key.clone(), Some(value.clone()));
            pending.writes.push(Write::Set { key, value });
            Ok(JsValue::undefined())
        }),
    )?;

    context.register_global_callable(
        JsString::from("$zdIncr"),
        2,
        NativeFunction::from_copy_closure(move |_this, args, context| {
            let bound = bound(ticket)?;
            let key = as_key(&argument(args, 0), context)?;
            let delta = Number::new(argument(args, 1).to_number(context)?);
            let mut pending = bound.pending();

            // A projection, not the answer. It is what the handler's own
            // `return await $store.incr(...)` yields, and under contention
            // it can differ from what commits — so `run_all` replaces the
            // response with the committed value. It is computed here
            // anyway because it is where "you cannot increment text" is
            // caught, and catching it before anything is applied is the
            // whole point.
            let current = pending.peek(&bound.store, &key).map_err(store_failure)?;
            let base = match &current {
                None => Number::ZERO,
                Some(json) => Number::parse(json.as_str()).ok_or_else(|| {
                    store_failure(StoreError::NotANumber {
                        key: key.clone(),
                        found: json.as_str().to_string(),
                    })
                })?,
            };
            let projected = base
                .plus(delta)
                .ok_or_else(|| store_failure(StoreError::OutOfRange { key: key.clone() }))?;

            pending.writes.push(Write::Incr { key, delta });
            Ok(JsValue::from(projected.as_f64()))
        }),
    )?;

    context.register_global_callable(
        JsString::from("$zdDelete"),
        1,
        NativeFunction::from_copy_closure(move |_this, args, context| {
            let bound = bound(ticket)?;
            let key = as_key(&argument(args, 0), context)?;
            let mut pending = bound.pending();
            pending.view.insert(key.clone(), None);
            pending.writes.push(Write::Delete { key });
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
  // `$wireParse` and `$wireStringify`, never `JSON.parse` and
  // `JSON.stringify`. A `Map of K to V` compiles to a JavaScript `Map`,
  // and `JSON.stringify(new Map(...))` is `{}` — silently. Every durable
  // map wrote an empty object and read nothing back until this changed.
  get(key) {
    const text = $zdGet(key);
    return text === null ? undefined : $wireParse(text);
  },
  set(key, value) {
    $zdSet(key, $wireStringify(value));
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
  // Read-modify-write, and no longer a race. The `get` records what it
  // saw into the invocation's transaction, so the write it computes is
  // conditional on the list not having changed underneath it; a concurrent
  // append is refused with a conflict and the whole invocation is re-run.
  // `incr` is different and deliberately so — it is a blind delta with no
  // recorded read, so two increments never conflict and both count.
  append(key, item) {
    const current = $store.get(key);
    const list = current === undefined ? [] : current;
    if (!Array.isArray(list)) {
      throw new Error('`' + key + '` does not hold a list, so `append` cannot add to it');
    }
    const next = list.concat([item]);
    $zdSet(key, $wireStringify(next));
    return next;
  },
  remove(key, item) {
    const current = $store.get(key);
    const list = current === undefined ? [] : current;
    if (!Array.isArray(list)) {
      throw new Error('`' + key + '` does not hold a list, so `remove` cannot take from it');
    }
    const next = list.filter((entry) => entry !== item);
    $zdSet(key, $wireStringify(next));
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

/// `wire.js`, as a script, with the two names the prelude calls.
///
/// The *same file* the browser imports. An adapter that re-implemented the
/// encoding would be a second definition of the wire format, and the way
/// the two would be found to disagree is a `Map` arriving as `{}` — which
/// is the bug this whole indirection exists to have fixed once.
fn wire() -> String {
    format!(
        "{}\nconst $wireStringify = stringify, $wireParse = parse;\n",
        as_script(zdc_runtime::WIRE_JS)
    )
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
    run_all(&[(endpoint, arguments_json)], store, env)
}

/// Why an attempt did not produce an answer.
///
/// Split because the two need opposite treatment: a conflict means "that
/// was a valid run against a store that moved, do it again", and anything
/// else means "stop".
enum Attempt {
    Conflict { key: String },
    Fatal(HostError),
}

/// Run a whole handler's writes as one transaction.
///
/// `calls` is the ordered list of commands one event handler asked for.
/// They run in one JavaScript context, against one recording `$store`, and
/// commit in one [`DurableStore::apply`] — which is the transaction. A
/// throw from any of them means the commit never happens, so the earlier
/// ones are not "rolled back": they were never applied.
///
/// A single call is not a special case. `$call("visits.incr", 1)` is a
/// one-element list, takes the same path, and commits the same way.
pub fn run_all(
    calls: &[(&Endpoint, &str)],
    store: &Arc<dyn DurableStore>,
    env: &Environment,
) -> Result<String, HostError> {
    let Some((first, _)) = calls.first() else {
        return Ok("null".to_string());
    };
    let mut attempt = 1;
    loop {
        match attempt_once(calls, store, env) {
            Ok(answer) => return Ok(answer),
            Err(Attempt::Fatal(error)) => return Err(error),
            Err(Attempt::Conflict { .. }) if attempt < ATTEMPTS => {
                attempt += 1;
            }
            Err(Attempt::Conflict { key }) => {
                // No `detail`: every attempt's ticket is gone by now, and
                // exhausting the retries is the host's own report about
                // contention rather than a binding's about configuration.
                return Err(HostError::Failed {
                    endpoint: first.name.clone(),
                    message: format!(
                        "`{key}` was changed by another handler on every one of {ATTEMPTS} \
                         attempts, so this write was refused rather than applied over somebody \
                         else's"
                    ),
                    detail: None,
                });
            }
        }
    }
}

/// One run of every call, and one commit.
///
/// A fresh context per attempt on purpose: a retry must re-run the
/// handlers against the store as it now is, and reusing a context would
/// carry the first attempt's `$zdOut` and any state a handler left behind.
fn attempt_once(
    calls: &[(&Endpoint, &str)],
    store: &Arc<dyn DurableStore>,
    env: &Environment,
) -> Result<String, Attempt> {
    let (ticket, bound) = Ticket::issue(Bound {
        store: Arc::clone(store),
        env: env.clone(),
        pending: Mutex::new(Pending::default()),
        detail: Mutex::new(None),
    });

    let mut context = Context::default();
    let id = ticket.0;
    let name = calls
        .first()
        .map(|(endpoint, _)| endpoint.name.clone())
        .unwrap_or_default();
    // `detail_of` is read at the moment the error is built rather than
    // once up front: a binding may not have run yet when `fatal` is
    // defined.
    let fatal = |endpoint: &str, message: String| {
        Attempt::Fatal(HostError::Failed {
            endpoint: endpoint.to_string(),
            message,
            detail: detail_of(id),
        })
    };

    install(&mut context, ticket.0).map_err(|e| fatal(&name, e.to_string()))?;
    context
        .eval(Source::from_bytes(wire().as_bytes()))
        .map_err(|e| fatal(&name, format!("the wire format did not evaluate: {e}")))?;
    context
        .eval(Source::from_bytes(PRELUDE.as_bytes()))
        .map_err(|e| fatal(&name, format!("the adapter prelude did not evaluate: {e}")))?;

    let mut answer = String::from("null");
    for (endpoint, arguments_json) in calls {
        answer = run_one(&mut context, endpoint, arguments_json, id)?;
    }

    // The recording is over. Take it out before the commit so a store that
    // calls back into nothing cannot see a half-taken transaction.
    let transaction = std::mem::take(&mut *bound.pending()).into_transaction();
    drop(ticket);

    if transaction.is_empty() {
        return Ok(answer);
    }

    let applied = store.apply(&transaction).map_err(|error| match error {
        StoreError::Conflict { key } => Attempt::Conflict { key },
        // No `detail`: the ticket is already dropped, and a commit failure
        // is the store's own words about a key the program named, not a
        // binding's report about configuration it could not read.
        other => Attempt::Fatal(HostError::Failed {
            endpoint: name.clone(),
            message: other.to_string(),
            detail: None,
        }),
    })?;

    // A command answers with what *committed*, not with what its handler
    // projected: `incr` computes its return value from the store as it was
    // when the binding ran, and under contention that is not the number
    // the browser must be shown. A batch answers with nothing — no
    // statement in the language consumes a write's result, and returning
    // the values would hand the client state it did not ask for.
    Ok(match calls {
        [(endpoint, _)] if matches!(endpoint.shape, Shape::Command) => applied
            .values
            .last()
            .cloned()
            .flatten()
            .map_or_else(|| "null".to_string(), Json::into_string),
        [_] => answer,
        _ => "null".to_string(),
    })
}

/// Evaluate one endpoint's source and drive its handler.
///
/// Each call redefines `handler`, which is why they run one at a time:
/// evaluating two bundles first would leave only the second's handler
/// bound to the name both drivers call.
fn run_one(
    context: &mut Context,
    endpoint: &Endpoint,
    arguments_json: &str,
    id: u64,
) -> Result<String, Attempt> {
    // `detail_of` is read when the error is built, not once up front: the
    // binding that leaves the detail behind runs *during* the handler, so
    // reading it at closure-definition time would always find nothing.
    //
    // This is the path that matters for §16.3.12 assertion C. A missing
    // `environment` key becomes a throw inside the handler, which the
    // driver puts in `$zdErr`, which arrives here — so if this closure
    // omitted `detail`, the server-side half of that report would be
    // written by the binding and read by nobody.
    let failed = |message: String| {
        Attempt::Fatal(HostError::Failed {
            endpoint: endpoint.name.clone(),
            message,
            detail: detail_of(id),
        })
    };
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
      $zdOut = $wireStringify(value);
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

    let mut read = |name: &str| -> Result<JsValue, Attempt> {
        context
            .eval(Source::from_bytes(name.as_bytes()))
            .map_err(|e| {
                Attempt::Fatal(HostError::Failed {
                    endpoint: endpoint.name.clone(),
                    message: format!("could not read `{name}` back: {e}"),
                    detail: detail_of(id),
                })
            })
    };

    if let Some(message) = read("$zdBad")?.as_string() {
        return Err(Attempt::Fatal(HostError::BadRequest {
            message: message.to_std_string_escaped(),
        }));
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
