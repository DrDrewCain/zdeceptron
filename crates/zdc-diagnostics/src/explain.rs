//! The long form of every diagnostic, behind `zdc explain <CODE>`.
//!
//! Barik et al. (ICSE 2017, n = 56, eye tracking) measured that reading a
//! compiler error is about as hard as reading source code, that reading
//! difficulty **significantly predicts** how long the task takes, and that
//! participants spent 13–25% of their fixations on error messages. Message
//! length is therefore a cost, not a free way to be helpful.
//!
//! So the inline diagnostic carries only what a reader needs in order to
//! act — the claim and the spans — and everything explaining *why* the rule
//! exists lives here, one command away. That is the progressive-disclosure
//! shape Rust uses for `--explain`, applied to the measurement.
//!
//! Two invariants, both tested:
//!
//! * every code the compiler can produce has an entry here, with the code
//!   list enumerated from `zdc-graph`'s source rather than maintained by
//!   hand (`tests/explanations.rs`);
//! * the inline form stays inside [`INLINE_MESSAGE_BUDGET`]
//!   (`tests/inline_budget.rs`).

/// The most characters a diagnostic's inline message may use.
///
/// Two lines at a comfortable terminal width. The number is a budget
/// rather than a measurement of anything: what the evidence establishes is
/// that length costs reading time, not where the cliff is. What matters is
/// that a ceiling exists, that it is small enough to force the "why" out of
/// the message, and that a test fails when a new diagnostic exceeds it.
///
/// Everything else the reader sees is source spans, which the design
/// requires — Sec. 7.3 asks an information-flow rejection to *show the
/// path* — plus one help line, which is always the pointer to `zdc explain`.
pub const INLINE_MESSAGE_BUDGET: usize = 200;

/// The one-line help every coded diagnostic ends with.
pub fn inline_help(code: &str) -> String {
    format!("run 'zdc explain {code}' for the rule")
}

/// The full statement of one rule.
pub struct Explanation {
    pub code: &'static str,
    /// The rule in a few words, used as the heading.
    pub name: &'static str,
    /// What the compiler concluded about the program.
    pub meaning: &'static str,
    /// Why the language has this rule at all.
    pub why: &'static str,
    /// A worked example: the rejected program, then the repair.
    pub example: &'static str,
}

impl Explanation {
    /// The rule as `zdc explain` prints it.
    pub fn render(&self) -> String {
        use std::fmt::Write as _;

        let mut out = String::new();
        let _ = writeln!(out, "{} \u{2014} {}", self.code, self.name);
        for (heading, body) in [
            ("What it means", self.meaning),
            ("Why the rule exists", self.why),
            ("How to fix it", self.example),
        ] {
            let _ = writeln!(out);
            let _ = writeln!(out, "{heading}");
            let _ = writeln!(out, "{}", indent(body));
        }
        out
    }
}

/// Indent a block by four columns, leaving blank lines empty rather than
/// filling them with trailing spaces.
fn indent(body: &str) -> String {
    body.lines()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("    {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The rule for one code, or `None` if there is no such code.
pub fn explain(code: &str) -> Option<&'static Explanation> {
    EXPLANATIONS.iter().find(|entry| entry.code == code)
}

/// Every code the compiler can produce, in the order `zdc explain` lists
/// them when it is asked for a code that does not exist.
pub fn codes() -> Vec<&'static str> {
    EXPLANATIONS.iter().map(|entry| entry.code).collect()
}

/// The placement and information-flow rules, in full.
///
/// Hand-written, because Santos & Becker (2024, n = 106) measured that
/// hand-written expert explanations beat both conventional compiler
/// messages and LLM-generated ones on time-to-fix *and* on satisfaction.
pub const EXPLANATIONS: &[Explanation] = &[
    Explanation {
        code: "E0301",
        name: "build-time state read something that does not exist at build time",
        meaning: "A `durable` signal's initial value is written into `manifest.json` when
the program is compiled, so its initialiser runs in the build — not in
a browser and not in a request. This one reads a signal that only
exists later.",
        why: "The build has no browser to ask and no store to read. If the
initialiser could read them, the value baked into the manifest would be
whatever they happened to be on the machine that ran the build, which
is nobody's machine in particular.",
        example: "Rejected — the initial value depends on browser state:

    state seed  is client  Whole starting 7
    state quota is durable Whole starting seed

Accepted — a literal initial value, and the derivation in a `server`
signal, which runs per request and may read anything:

    state seed  is client  Whole starting 7
    state quota is durable Whole starting 0
    state shown is server  Whole from clamp with seed, quota",
    },
    Explanation {
        code: "E0302",
        name: "a scheduled handler read browser state",
        meaning: "This code runs on a schedule rather than in response to a visitor, so
no browser is attached to it, and the signal it read lives in browser
memory.",
        why: "A `client` signal is one variable per open tab. Code with no tab
attached could only read some arbitrary tab's copy, or none at all, and
either answer would be wrong in a way that stays invisible until it is
in production.",
        example: "Accepted — read the browser value in a `server` signal that the view
asks for, where the client supplies it as an argument to the generated
call:

    state query   is client Text         starting \"\"
    state matches is server List of Item from search with query",
    },
    Explanation {
        code: "E0303",
        name: "a trigger read state that exists only inside a session",
        meaning: "`durable per visitor` state is partitioned: each visitor has a private
slice of it. Code running from a trigger has no session, so there is no
visitor whose slice it could mean.",
        why: "Per-visitor storage is only meaningful relative to a visitor. A trigger
that read it would have to pick one, and any rule for picking is a
security decision the language will not make on your behalf.",
        example: "Accepted — read globally scoped `durable` state from a trigger, and
leave the per-visitor slice to code that a visitor's request reached:

    state totals is durable Map of Id to Whole starting empty",
    },
    Explanation {
        code: "E0310",
        name: "something wrote to state that is computed once at build time",
        meaning: "`static` state is evaluated when the program is compiled and baked into
the artefact. There is no cell at run time for a write to land in.",
        why: "The alternative is a write that appears to succeed and is discarded, or
one that mutates a value every visitor shares with no store behind it.
Both fail silently, and this language's standard is that a construct
which fails silently is a defect rather than an inconvenience.",
        example: "Rejected — writing to build-time state:

    state total is static Whole starting 1
    ...
    add 1 to total

Accepted — declare it somewhere that exists at run time: `client` for
one browser, `durable` for storage shared across visitors.",
    },
    Explanation {
        code: "E0311",
        name: "the browser tried to write server state",
        meaning: "A `server` signal is derived: it is recomputed from its inputs whenever
they change. It is not a variable, so there is nothing to assign to.",
        why: "If the browser could assign to a derived signal, the next recomputation
would silently overwrite what it wrote, and which of the two won would
depend on timing. Writing the input instead makes the update
deterministic and keeps one definition of where the value comes from.",
        example: "Rejected — assigning to the derived value:

    state hits is server Whole from countOf with log
    ...
    add 1 to hits

Accepted — write the state it is derived from, and let the compiler
re-run the derivation:

    state log  is durable List of Text starting empty
    state hits is server  Whole        from countOf with log
    ...
    append entry to log",
    },
    Explanation {
        code: "E0312",
        name: "server code tried to write browser state",
        meaning: "This statement runs in a serverless invocation, and the signal it
writes lives in the memory of one browser tab.",
        why: "The server holds no handle on a tab's variables. A write that looked
like it worked and reached nothing would be the worst version of this,
so the compiler refuses it instead, and the network stays visible in
the source.",
        example: "Rejected — the server assigning to a `client` signal:

    function bump with n
        set seen to n
        give n

Accepted — give the value back, and let the browser store it:

    function bump with n
        give n + 1

    Button \"more\"
        on click
            set seen to bump with seen",
    },
    Explanation {
        code: "E0313",
        name: "a secret was declared somewhere its reader can see it",
        meaning: "`secret` may be declared on `server` and `durable` state. This
declaration puts it in browser memory or in the build artefact, both of
which the reader of the page already holds.",
        why: "`secret` is a claim about who can observe a value, and the whole
information-flow pass rests on that claim being true at the
declaration. A secret in a place the browser holds is not secret from
anything, and honouring the keyword there would make every downstream
rejection meaningless.",
        example: "Rejected — a secret in browser memory:

    secret state token is client Text starting \"\"

Accepted — a secret where the browser is not:

    secret state token is server Text from environment \"API_TOKEN\"",
    },
    Explanation {
        code: "E0314",
        name: "something wrote into a value rather than a place",
        meaning: "`set`, `add`, `subtract`, `append` and `remove` write into `state`. The
name on the left of this one is a function parameter, which holds a
copy of what was passed.",
        why: "Writing into a copy cannot be observed by anyone: the caller's value is
unchanged, the write is discarded when the function returns, and
nothing anywhere reports it. Until this rule existed the program
compiled and the statement was silently dropped.",
        example: "Rejected — writing through a parameter:

    function bump with box
        add 1 to box
        give 0

Accepted — return the new value, and write the state at the call site:

    function bump with box
        give box + 1

    set total to bump with total",
    },
    Explanation {
        code: "E0320",
        name: "signals are defined in terms of each other",
        meaning: "Following the `from` clauses leads back to where it started, so none of
the signals in the cycle has a value to compute from. The diagnostic
prints the cycle, one span per edge.",
        why: "A derived signal is a function of its inputs. A cycle asks for a value
that is a function of itself, which has no answer unless one member of
the cycle is a starting point instead.",
        example: "Rejected — each is derived from the other:

    state a is client Whole from idOf with b
    state b is client Whole from idOf with a

Accepted — break the cycle by giving one of them a starting value:

    state a is client Whole starting 0
    state b is client Whole from idOf with a",
    },
    Explanation {
        code: "E0321",
        name: "a durable signal was derived rather than stored",
        meaning: "`durable` is storage. It has a value because something wrote one, not
because something computed one, so it takes `starting` and never
`from`.",
        why: "A derived durable signal has two answers to \"what is in the store\": the
bytes on disk, and the derivation. Keeping them in step means
recomputing and rewriting the store on every input change, which is a
cache with no invalidation story. Deriving in a `server` signal that
reads the store gives the same value with one source of truth.",
        example: "Rejected — durable and derived:

    state base  is durable Whole starting 1
    state twice is durable Whole from double with base

Accepted — store one, derive the other on the server:

    state base  is durable Whole starting 1
    state twice is server  Whole from double with base",
    },
    Explanation {
        code: "E0360",
        name: "`environment` was read outside server context",
        meaning: "`environment \"NAME\"` reads a value out of the process environment of a
serverless invocation. This code does not run in one.",
        why: "The environment is where credentials live, and it exists on the server
only. Reading it from the browser would mean shipping it to the
browser; reading it at build time would mean baking it into the
artefact. The language can do neither, so the read is refused where it
cannot be answered.",
        example: "Accepted — read it into a `server` signal, and read that signal
wherever it is needed. `environment` is always secret, so the flow
rules apply to it from the declaration onwards:

    secret state apiKey is server Text from environment \"API_KEY\"",
    },
    Explanation {
        code: "W0330",
        name: "nothing reads this signal, so no endpoint was generated",
        meaning: "A `server` or `durable` signal that nothing reads produces no generated
endpoint and no storage, so it costs nothing at run time. It is
reported because that absence is otherwise invisible.",
        why: "The split emits code on demand. A signal nothing reaches is not an
error — it may be about to be used — but a developer who expected an
endpoint to exist should learn that it does not from the compiler
rather than from a 404.",
        example: "Either read it somewhere, or delete the declaration. Reading it from
the view is enough to make the endpoint appear:

    when total
        Loading          show Spinner
        Failed with e    show ErrorBar message is e.message
        Ready with value show Text value",
    },
    Explanation {
        code: "W0331",
        name: "nothing reads this signal, so no cell was emitted",
        meaning: "A `client` signal that nothing reads gets no cell in the bundle and no
setter. Writes to it, if there are any, have nowhere to land.",
        why: "The same demand-driven rule as W0330, on the browser side. It matters
more here, because a `client` signal is usually written by a handler: a
`set` against a signal nothing reads is almost always a misspelling of
the name.",
        example: "Either read it, or delete it. If a handler writes it, check the name in
the handler against the name in the declaration — a `set` to an unread
signal is the shape a misspelling takes.",
    },
    Explanation {
        code: "E-IFC-01",
        name: "a secret was declared on a placement that cannot hold one",
        meaning: "The information-flow pass reached a signal declared `secret` whose
placement puts it where the reader is. This is the same fact E0313
reports, checked a second time by a different pass.",
        why: "Two passes read the `secret` keyword: the split, which decides
placement, and the flow pass, which decides what may be observed. If
they ever disagreed about what `secret` means, the flow pass would be
reasoning about a lattice the split does not implement. This redundant
check exists so that such a disagreement is loud. It is raised only
when E0313 did not already fire, so one mistake is never printed twice.",
        example: "See `zdc explain E0313`: the repair is the same one — move the
declaration to `server` or `durable`.",
    },
    Explanation {
        code: "E-IFC-02",
        name: "a secret was derived into a signal that is not declared secret",
        meaning: "Following the derivation, a value that is secret reaches this signal,
and the signal's declaration does not say `secret`. The numbered labels
on the diagnostic are that path, in reading order.",
        why: "Secrecy is declared, not inferred. If the compiler quietly promoted
this signal to secret, the promotion would propagate to everything
reading it, and a developer would meet the consequences at some distant
use site rather than here. Declaring it is also the honest answer: a
value computed from a secret usually is one.",
        example: "Rejected — the derivation is secret, the declaration is not:

    secret state apiKey is server Text from environment \"API_KEY\"
    state request is server Text from sign with apiKey

Two repairs. Declare the result secret, and accept that it can no
longer be rendered:

    secret state request is server Text from sign with apiKey

Or stop the secret reaching it, by computing the public part
separately from the part that needs the key.",
    },
    Explanation {
        code: "E-IFC-03",
        name: "a secret was written into a place that is not secret",
        meaning: "A write must satisfy `label(value) or pc <= label(place)`: what is
written, joined with the secrecy of the branch the write sits under,
must be no more secret than the place written to. This write is not.",
        why: "`pc` is in that rule because control flow leaks too. Writing a public
constant inside `if apiKey is \"\"` still tells a reader which branch
ran, and over enough writes that is the key. Tracking data dependencies
alone misses it, which is why the rule joins the branch context in.",
        example: "Rejected — the audit log is public, the value is not:

    state auditLog is durable List of Text starting empty
    secret state apiKey is server Text from environment \"API_KEY\"
    ...
    append apiKey to auditLog

Accepted — declare the place secret, or write something that is
neither derived from the secret nor under a branch on it:

    append \"key rotated\" to auditLog",
    },
    Explanation {
        code: "E-IFC-05",
        name: "a secret would be rendered",
        meaning: "A value that is secret reaches the view. The view is the page, so
anything in it is in the browser, in the DOM, and in view-source. The
numbered labels on the diagnostic are the path the value takes, from
the declaration to the point where the browser would see it.",
        why: "This is the rule the rest of the design exists to support. `secret` is
worth declaring only if the compiler refuses every path to a reader,
and the view is the most direct one. The branch context counts as such
a path: rendering nothing inside `if apiKey is \"\"` still renders one
bit of the key, which is why a `when` on a secret is refused even when
both arms draw the same thing.",
        example: "Rejected — the secret in an element:

    secret state apiKey is server Text from environment \"API_KEY\"

    view
        Column
            Text apiKey

Accepted — render something computed on the server that is not itself
secret. The key is used, and never leaves:

    secret state apiKey is server Text from environment \"API_KEY\"
    state greeting is server Text from politeGreeting with name, apiKey

    view
        Column
            when greeting
                Loading         show Spinner
                Failed with e   show ErrorBar message is e.message
                Ready with text show Text text

If the value really must be shown, `secret` is the wrong declaration
for it — and changing that declaration is a decision worth making
deliberately, which is why the compiler will not make it for you.",
    },
    Explanation {
        code: "E-IFC-06",
        name: "a secret would be stored in browser memory",
        meaning: "A value that is secret reaches a `client` signal. `client` state lives
in the tab, so writing a secret there ships it to the reader whether or
not anything ever renders it.",
        why: "The same rule as E-IFC-05, one step earlier. Client state is observable
in the debugger, in a heap snapshot, and by any script on the page. A
secret that is merely stored in the browser has already left the
server.",
        example: "Rejected — a click handler writing a secret into browser state:

    state cached is client Text starting \"\"
    ...
    set cached to apiKey

Accepted — keep the secret on the server and send only what the page
needs, which is usually the result rather than the input.",
    },
    Explanation {
        code: "E-IFC-07",
        name: "a secret would be baked into the build artefact",
        meaning: "A value that is secret reaches something evaluated at build time and
written into the shipped files: `manifest.json`, or a `static` value
inlined into the bundle.",
        why: "The build artefact is downloaded by every visitor. A secret in it is
not merely exposed, it is exposed permanently — it is in every cache
and every copy of the deploy, and rotating the key is the only remedy.",
        example: "Accepted — read the secret in a `server` signal, which is evaluated per
request rather than per build:

    secret state apiKey is server Text from environment \"API_KEY\"",
    },
    Explanation {
        code: "E-IFC-08",
        name: "a secret would be sent in a response body",
        meaning: "A value that is secret reaches the body of a response that a generated
endpoint returns to the browser.",
        why: "A generated endpoint exists because the view asked for a server value.
What comes back goes to the browser by definition, so the response body
is a sink exactly as the view is — and it is the one a reader is least
likely to think of, because no line of source says `send`.",
        example: "Accepted — return the part of the answer that is not secret. If the
whole answer is secret, declare the signal `secret` and compute what
the page needs from it on the server.",
    },
    Explanation {
        code: "E-IFC-09",
        name: "a secret would be written to a platform log",
        meaning: "A value that is secret reaches something the hosting platform records:
a log line, an error report, or a trace.",
        why: "Logs are the least guarded copy of anything. They are retained longer
than the request, replicated to systems with different access rules,
and read by people who were never given the key. A secret in a log has
usually leaked to more readers than a secret on a page.",
        example: "Accepted — log something derived from the secret that is not itself
secret: whether it was present, how long it was, which branch ran.
Never the value.",
    },
    Explanation {
        code: "E-IFC-10",
        name: "a secret would be observable through live sync",
        meaning: "`durable` state is streamed to subscribed browsers so that two open
windows agree. This signal is secret, and either its value is streamed
or the browser is told when it changes.",
        why: "Being told *that* a value changed is an observation of it. A page that
learns the moment a secret store is written learns the write pattern,
and for a great many secrets the write pattern is the interesting part.
Conflating \"the browser is sent the value\" with \"the browser is told it
changed\" is what made an earlier version of this rule either
permanently stale or a live leak; they are now separate edges, and both
are checked.",
        example: "Accepted — refresh on a cadence rather than on change, so that nothing
about the write is streamed; or derive a public summary and let the
browser subscribe to that instead:

    secret state ledger is durable List of Text starting empty
    state total is server Whole from countOf with ledger",
    },
];
