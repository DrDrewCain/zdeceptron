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
    /// What the caret says about the span this code points at.
    ///
    /// A code reports the same *kind* of thing wherever it is raised, so
    /// what its caret covers is a fact about the rule and is written here
    /// with the rule's other prose. A reporting site that knows better
    /// overrides it; a site that knows nothing more still gets a label
    /// that says something, which is why the field is not optional.
    pub caret: &'static str,
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

/// What the caret says for one code, or `None` if there is no such code.
///
/// This is the default a reporting site gets for free. It replaced the
/// literal string `here`, which every diagnostic in the compiler used and
/// which told the reader where the caret already was.
pub fn caret(code: &str) -> Option<&'static str> {
    explain(code).map(|entry| entry.caret)
}

/// Every code the compiler can produce, in the order `zdc explain` lists
/// them when it is asked for a code that does not exist.
pub fn codes() -> Vec<&'static str> {
    EXPLANATIONS.iter().map(|entry| entry.code).collect()
}

/// The syntax, placement and information-flow rules, in full.
///
/// Hand-written, because Santos & Becker (2024, n = 106) measured that
/// hand-written expert explanations beat both conventional compiler
/// messages and LLM-generated ones on time-to-fix *and* on satisfaction.
pub const EXPLANATIONS: &[Explanation] = &[
    Explanation {
        code: "E0101",
        caret: "a placement belongs before this",
        name: "a `state` declaration did not say where its value lives",
        meaning: "Every `state` declaration names a placement between `is` and the type:
`client`, `static`, `server` or `durable`. This one goes straight from
`is` to the type, so the compiler has been told what the value is and
not where it is.",
        why: "Placement is not a default the compiler can pick. The four are four
different machines: `client` is one variable per open tab, `static` is
computed once by the build and inlined, `server` is a serverless
invocation per request, and `durable` is a store that outlives both.
Choosing wrongly on your behalf would produce a program that runs and
is wrong, and the whole placement pass exists to reason about the
answer, so it has to be written down.

The compiler suggests `client`, because a value with no other
requirement belongs in the browser that shows it. That is a suggestion
and not an inference: the other three are one word away and this page
is where they are described.",
        example: "Rejected — no placement:

    state votes is Map of Id to Int starting empty

Accepted — one word, and the rest of the line unchanged:

    state votes is client  Map of Id to Int starting empty

The choice, in one line each:

    client   browser memory, one copy per open tab
    static   computed once at build time and inlined into the bundle
    server   a serverless invocation, recomputed per request
    durable  persistent storage, shared across visitors and visits",
    },
    Explanation {
        code: "E0102",
        caret: "this is a keyword, so it cannot be a name",
        name: "a keyword was written where a name goes",
        meaning: "A keyword may not be a record field name, a function parameter name, a
state name, or any other name. The word written here is one of the
words the grammar has already spent, and the diagnostic says which
construct spends it.",
        why: "The grammar is keyword-led: a statement, a declaration and a clause are
each recognised by the word that opens them. A name that could also be
one of those words would make the same line readable two ways, and §4.1
buys exactly one reading per construct. Reserving the word everywhere,
rather than only where it would actually be ambiguous, is what keeps
the rule statable in one sentence.

The cost is real and is not hidden here: `from`, `to`, `route` and
`limit` are ordinary names for ordinary data, and a program that models
graph edges will reach for `from` and `to` first. The compiler will not
invent a replacement, because the right name depends on what the field
means and a mechanical one would be worse than the reader's own.",
        example: "Rejected — `from` introduces a pipeline's source, so an edge cannot be
called it:

    record Edge
        from is Whole
        to   is Whole

Accepted — the graph theorist's spelling, or anything else that is not
reserved:

    record Edge
        tail is Whole
        head is Whole",
    },
    Explanation {
        code: "E0103",
        caret: "this is not the form the construct takes",
        name: "the construct has one valid form and this is not it",
        meaning: "The parser was in the middle of a construct whose next part is fixed:
one particular keyword, a quoted literal, a line break, or an indented
block. The message names what belongs there; the caret names what is
written instead.",
        why: "§4.1's bargain is exactly one phrasing per construct, and the price of
that bargain is paid here: there is no second spelling to try, so the
diagnostic can always state the single valid form rather than listing
candidates. The rule is worth the price because the reverse — several
spellings for one meaning — makes every program a dialect and every
error message a guess about which dialect was intended.",
        example: "Rejected — a call with `with` inside an argument list, where a following
`,` could belong to either call:

    Link Photo with album is slug, padding is 8

Accepted — the parentheses say which call the `,` ends:

    Link (Photo with album is slug), padding is 8",
    },
    Explanation {
        code: "E0104",
        caret: "nothing here can begin the construct this position expects",
        name: "the word written begins none of the constructs allowed here",
        meaning: "This position begins a value, a statement, a view node or a
declaration, and each of those is a closed set. What is written begins
none of them, so the message lists the set rather than guessing which
member was meant.",
        why: "This is the other half of E0103. There the next part was one specific
thing; here it is any of several, and the honest diagnostic is the list.
Listing it is affordable precisely because the sets are closed: a
language that let a position begin arbitrarily many constructs could
only say that something was wrong.",
        example: "Rejected — a bare number is not a view node:

    view
        5

Accepted — a node, with the number as an argument to it:

    view
        Text \"5\"",
    },
    Explanation {
        code: "E0105",
        caret: "the nesting reaches its limit here",
        name: "the source nests deeper than the compiler will follow",
        meaning: "Expressions, types and indented blocks are each parsed by recursion,
and each has a depth limit. This file passes one of them.",
        why: "The limits are not a judgement about style, they are a totality
guarantee. Recursive descent turns nesting in the source into frames on
the stack, and exhausting the stack raises `SIGABRT`: no panic, nothing
`catch_unwind` can hold, no diagnostic at all, and a language server
that simply dies mid-keystroke. A limit turns that into a sentence. The
numbers are measured from the frame sizes rather than guessed, and both
are far above anything written by hand — a generated file is what
reaches them.",
        example: "Give the inner parts names and refer to them, which is the repair in
every case:

    state total is client Whole from sumOf with parts
    state parts is client List of Whole starting empty",
    },
    Explanation {
        code: "E0106",
        caret: "this URL is not a canonical absolute path",
        name: "a route's URL is not a canonical absolute literal path",
        meaning: "A route's URL is a literal prefix. It begins with `/`, each segment uses
only letters, digits, `-`, `_` or `.`, neither `.` nor `..` is a
segment, and there are no repeated or trailing slashes. A parameter is
declared after `with` rather than written inside the string.",
        why: "A URL with a parameter spelled inside it is a second grammar inside a
string literal, and a grammar inside a literal is a grammar nothing
checks — §6 refuses embedded markup in a string for the same reason.
Declaring the parameter after `with` puts it where the type checker and
the router can both see it. Requiring the canonical form is what makes
two routes comparable: `/blog` and `/blog/` would otherwise be two
spellings of one address, and deciding which one an incoming request
matched is a decision nobody wants to make twice.",
        example: "Rejected — the parameter is inside the literal:

    route
        BlogPost is \"/blog/[slug]\"

Accepted — the literal is the prefix, and the parameter is declared:

    route
        BlogPost is \"/blog\" with slug is Text in postSlugs",
    },
    Explanation {
        code: "E0301",
        caret: "this read runs at build time",
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
        caret: "this code runs on a schedule, with no browser",
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
        caret: "this code runs with no session",
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
        caret: "this write has nowhere to land",
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
        caret: "a derived signal is not a place to write",
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
        caret: "this code cannot reach a browser's memory",
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
        caret: "this declaration puts the secret where its reader is",
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
        caret: "this is a value, not a place",
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
        code: "E0315",
        caret: "a file's contents have to be `Text`",
        name: "a generated file was written from something that is not text",
        meaning: "`emitting` writes a signal's value into a file in the bundle. A file's
contents are text, and this signal is some other type.",
        why: "The alternative is for the compiler to pick a serialisation — a number's
formatting, a list's separator, a record's shape — and any choice it
made would be invisible in the source and wrong for somebody. §14C.3b
puts that decision in the program, where it can be read.",
        example: "Rejected — emitting a value that is not text:

    state count is static Whole starting 3 emitting \"count.txt\"

Accepted — derive the file's text in another `static` signal, and emit
that one:

    state count is static Whole starting 3
    state page  is static Text  from textOf with count emitting \"count.txt\"",
    },
    Explanation {
        code: "E0316",
        caret: "this path leaves the bundle",
        name: "a generated file was written outside the bundle",
        meaning: "The path after `emitting` names a place that is not inside the bundle:
it is absolute, it climbs out with `..`, it carries a drive or scheme,
or it names a directory rather than a file.",
        why: "A build writes the bundle and nothing else. A compiler that honoured an
absolute path would let a source file decide to write anywhere the
person running the build can write, which is a build-time arbitrary
write and not a feature.",
        example: "Rejected — a path that leaves the bundle:

    state feed is static Text starting \"...\" emitting \"/etc/hosts\"

Accepted — a path relative to the bundle root:

    state feed is static Text starting \"...\" emitting \"feeds/posts.xml\"",
    },
    Explanation {
        code: "E0317",
        caret: "nothing can send this anywhere",
        name: "a handle was written somewhere it would have to travel",
        meaning: "`Handle` is the type of an object the host owns — a three.js `Scene`, a
`WebGLRenderer`, a canvas context. It may be written bare in three
places: a `foreign`'s parameter type, a `foreign`'s result type, and the
type of a `client` signal declared `starting`. This declaration puts one
somewhere else — somewhere a value is sent, wrapped in another type, or
replaced.",
        why: "Two different facts, and the message says which one applies.

The first is the wire. A handle is a reference into one JavaScript heap.
This is not a value whose encoding the compiler declines to write: there
is no encoding, because what would be sent is an identity inside a
running process, and the object it names exists nowhere else. `Remote of
Handle` is the clearest case — it asks for a host object over the
network — but `server`, `durable` and `static` state, a `record` field
and a `release`'s `gives` are all places a value crosses or persists,
and `List of Handle` would need the same marshalling rule a bare one
would.

The second is time. `client` state *can* hold a handle, and what it may
not do is hold a second one: a derived signal recomputes, a `set`
replaces, and the language has no `destroy` to run on the value that
goes. Either would drop a live WebGL context and acquire another, on
every update, until the browser stopped granting them — which is the
leak a `foreign … gives view` module's own `destroy` hook exists to
prevent.

So the rule a `client` signal has to satisfy is `starting` and no write.
Its initialiser runs once, when the document loads, and the handle then
lives as long as the page does. Releasing one sooner is a call the
program makes — `do disposeOf with r is renderer` — and not an
obligation this compiler enforces.",
        example: "Rejected — a handle asked to cross the network:

    state scene is server Remote of Handle starting empty

Rejected — a handle that would be replaced on every recomputation:

    state scene is client Handle from newScene

Accepted — acquired once, and never replaced:

    foreign newScene is client
        from \"three\" as \"Scene\"
        gives new Handle

    state scene is client Handle starting newScene",
    },
    Explanation {
        code: "E0320",
        caret: "following the `from` clauses returns here",
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
        caret: "`durable` stores a value rather than computing one",
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
        caret: "`environment` has no answer in this context",
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
        code: "E0361",
        caret: "the build is over by the time this runs",
        name: "a build capability was asked for outside the build",
        meaning: "`build read`, `build list` and `build markdown` are answered by the
compiler while the compiler is running. This code does not run then.",
        why: "A build capability is not a permission that could be granted more
widely: it is a question only the compiler can answer, and once the
build is over there is nobody left to ask. A browser has no project
directory and a serverless invocation has no compiler, so the read is
refused where it cannot be answered — the same shape of rule as E0360,
from the other end of the pipeline.",
        example: "Accepted — read the file into a `static` signal, which is computed once
at build time and inlined into the bundle, then read that signal from
wherever it is needed:

    state page is static Text from render with path is \"content/hello.md\"

    function render with path
        give build markdown (build read path)",
    },
    Explanation {
        code: "W0330",
        caret: "nothing reads this, so no endpoint exists",
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
        caret: "nothing reads this, so no cell exists",
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
        caret: "this placement cannot hold a secret",
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
        caret: "this declaration does not say `secret`",
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
        caret: "the place written is not secret",
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
        caret: "the browser would see the value here",
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
                Failed with e   show ErrorBar message is e.code
                Ready with text show Text text

Note which field the error arm reads. This endpoint reads `apiKey`, so
`e.message` is refused here by §14G.1.3(d): the host was holding the
key when it failed, and error text carries the request it was making.
`e.code` is the browser's own account of the transport — \"Unreachable\",
\"Timeout\" or \"Rejected\" — so it says how the call failed and never
what the host said about it. On an endpoint that reads nothing secret,
`e.message` renders as before.

If the value really must be shown, `secret` is the wrong declaration
for it — and changing that declaration is a decision worth making
deliberately, which is why the compiler will not make it for you.",
    },
    Explanation {
        code: "E-IFC-06",
        caret: "browser memory is where the reader is",
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
        caret: "the build artefact ships to every visitor",
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
        caret: "a response body goes to the browser",
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
        caret: "a log outlives the request that wrote it",
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
        caret: "a subscribed browser would learn about this",
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
    Explanation {
        code: "E-IFC-11",
        caret: "the browser resolves this and issues a request",
        name: "a secret would choose where the browser sends a request",
        meaning: "This value ends up in an attribute the browser dereferences \u{2014} `src`,
`source`, `href`, `srcset`, `poster`, `action` and the rest. The browser
resolves it and issues a request, and the value chooses the host that
request goes to.",
        why: "The view sink catches what a reader *sees*. This catches what the
browser *sends*, and they are different escapes. `Image source is apiKey`
renders no visible text and appears in no response body, and the browser
still fetches `https://attacker.example/<key>` before anything is
painted \u{2014} an image with `display: none` leaks exactly as well as a visible
one. The rule ranges over the attribute *name* on every element, not
over the elements meant to have a URL, because an unrecognised named
argument reaches the DOM as the attribute of that name: `Text src is
apiKey` would fall straight through a rule keyed on the element.",
        example: "Rejected \u{2014} the key chooses the host, so the host learns the key:

    Image source is apiKey, alt is \"a\"

Accepted \u{2014} make the request on the server, where the secret already lives,
and give the browser back something public:

    state chart is server Text from renderWith name, apiKey",
    },
    Explanation {
        code: "E-IFC-13",
        caret: "this argument crosses into the browser's bundle",
        name: "a secret is passed to a client foreign",
        meaning: "This value is secret, and it is an argument to a `foreign \u{2026} is client`.
A client foreign is JavaScript from a package or a file of your own,
linked into the browser bundle by that declaration \u{2014} so passing it a
value is handing that value to code running where the reader is.",
        why: "The compiler cannot see inside a foreign function. §14E.3 makes the
FFI a hole in the *type* system deliberately, and deliberately not a
hole in the information-flow system, and this is the rule that keeps the
second half true. It is not caught by the view sink or by client state,
because nothing is rendered and nothing is stored: the value leaves the
program through the module's own import, which no other rule looks at.

Stated without euphemism, so the grant is legible: a client foreign can
`innerHTML` attacker markup, set `href` or `src` to an exfiltrating URL,
walk `parentNode` out of its own subtree and rewrite the page, read
`document.cookie` and `localStorage`, and open outbound requests. The
compiler prevents exactly two things \u{2014} a `secret` crossing in, which is
this rule, and any value crossing back out. Everything else is granted
by the declaration, and the declaration is the audit surface.",
        example:
            "Rejected \u{2014} the digest runs in the browser, so the browser is given the key:

    foreign hashOf is client
        from  \"./hash.js\" as \"digest\"
        takes input is Text
        gives Text

    state shown is server Text from hashOf with input is apiKey

Accepted \u{2014} do the work where the secret already lives. If the module needs
no DOM, `is server` puts it in the bundle that may read credentials:

    foreign hashOf is server
        from  \"./hash.js\" as \"digest\"
        takes input is Text
        gives Text",
    },
    Explanation {
        code: "E-URL-01",
        caret: "the browser would run this rather than fetch it",
        name: "a URL whose scheme executes rather than fetches",
        meaning: "This URL is written out in the source, and its scheme is not one the
browser fetches. `javascript:` runs the rest of the value as a script;
`data:` is a document the author of the URL controls completely.",
        why: "\u{00A7}16.3.5's escaping argument is about the *markup* grammar: it
establishes that a value cannot close a tag or open one. A URL is handed
to the URL parser instead, and `javascript:alert(1)` contains nothing an
HTML escaper would touch \u{2014} so escaping it changes nothing at all.
`setAttribute('href', v)` stores the value verbatim and the browser runs
it on click. The rule is an allowlist rather than a list of the
dangerous schemes, because which schemes a browser executes is the
browser's decision and it grows; a denylist is out of date the day it is
written. It is a rejection rather than a sanitisation: silently
rewriting the URL turns a program its author got wrong into a link that
goes nowhere, which is harder to find than a compile error.",
        example: "Rejected \u{2014} nothing fetches this, and clicking it runs it:

    Link \"javascript:alert(1)\"
        Text \"go\"

Accepted \u{2014} a relative URL, or one in `http`, `https`, `mailto` or `tel`:

    Link \"/notes/signals\"
        Text \"go\"",
    },
    Explanation {
        code: "E-INT-01",
        caret: "a browser owns this cell, so the program cannot vouch for it",
        name: "`trusted` on a placement that cannot carry it",
        meaning: "`trusted` is a claim about *who chose this value* (spec \u{00A7}18.1.1). It is
a claim only the program can make good on, so it may sit only where a
browser is not the writer. `client` state is a browser's own memory, so
it is the one placement the word can never be true of.",
        why: "Secrecy and integrity are two independent lattices, and the mistake
they invite is opposite. A secret in the wrong place leaks. A `trusted`
in the wrong place is worse than useless: every obligation downstream
is discharged against a promise nobody kept, so the pass reports
nothing and the program looks checked. Refusing the declaration is what
keeps `trusted` from being a word that turns the analysis off.",
        example: "Rejected \u{2014} the browser owns this cell, so the program cannot vouch for
what is in it:

    trusted state role is client Text starting \"guest\"

Accepted \u{2014} declare it where a browser is not the writer, and let the
integrity pass check every write into it:

    trusted state role is durable Text starting \"guest\"",
    },
    Explanation {
        code: "E-INT-02",
        caret: "a browser chose this index",
        name: "an untrusted value chose which entry was written",
        meaning: "This write names an entry of a `trusted` place, and the index came from
somewhere a browser had a hand in \u{2014} a route parameter, an event payload,
or a value lifted from the client. Obligation A1 of \u{00A7}18.1 semantics 8.",
        why: "This is what an insecure direct object reference *is*. The value being
written may be perfectly ordinary; the mistake is that a visitor chose
the row. Checking the value and not the index is the commonest way to
write the bug, which is why the index carries an obligation of its own
rather than sharing the value's.",
        example: "Rejected \u{2014} the visitor typed the key:

    trusted state moderators is durable Map of Text to Truth starting empty
    ...
        on keydown with press
            set moderators at press.key to yes

Accepted \u{2014} decide the entry from state the program owns, and let the
untrusted value only select among choices the program already made.",
    },
    Explanation {
        code: "E-INT-03",
        caret: "a browser had a hand in this write",
        name: "an untrusted value was written to a `trusted` place",
        meaning: "This write puts a value into a `trusted` place, and the value came from
somewhere a browser had a hand in choosing. Obligation A3 of \u{00A7}18.1
semantics 8. The diagnostic names where the value picked up its label,
which may be several calls away from the write.",
        why: "`trusted` is a claim the rest of the program is allowed to rely on. A
value a visitor chose is exactly what the claim excludes, so admitting
one silently would make every later reader of that place wrong \u{2014} and
the reader is usually a decision about what somebody is allowed to do.",
        example: "Rejected \u{2014} the payload of an event is whatever the browser sent:

    trusted state note is durable Text starting \"\"
    ...
        on keydown with press
            set note to press.key

Accepted \u{2014} write a value the program chose, or check the untrusted one
against something the program owns before it reaches the place.",
    },
    Explanation {
        code: "E-INT-04",
        caret: "a browser decided whether this write runs",
        name: "a write happened under an untrusted decision",
        meaning: "The write itself is fine and its value is fine, but *whether it happens*
was decided by an untrusted value \u{2014} a `when` or an `if` whose condition a
browser had a hand in. This is the implicit-flow arm of \u{00A7}18.1, the same
`pc` threading \u{00A7}17.3.4 does for secrecy.",
        why: "A rule that watched only the value written would be trivially defeated:
`if theyAskedNicely then set admin to yes` writes a constant. What a
visitor controls is the branch, and control over the branch is control
over the outcome, so the branch condition is part of the obligation.",
        example: "Rejected \u{2014} a visitor decided whether the write ran:

    trusted state moderators is durable Map of Text to Truth starting empty
    ...
        on click with press
            if press.shift
                set moderators at \"root\" to yes

Accepted \u{2014} branch on state the program owns, or make the decision
somewhere the program can vouch for it.",
    },
    Explanation {
        code: "E-INT-05",
        caret: "a browser chose this argument",
        name: "an untrusted argument to a `trusted` foreign parameter",
        meaning: "A `foreign` declaration wrote `trusted` on one of its parameters, and this
call site passes a value the compiler cannot trace back to a grant.
Obligation A2 of \u{00A7}18.1 semantics 8. The declaration is what raises the
obligation \u{2014} a parameter without the word carries none.",
        why: "The word on the parameter is a library author saying *this argument
decides something, and I am not in a position to check it*. A storage
key, a path, an identifier: the call is ordinary and the argument is
what makes it dangerous. Reporting at the call site rather than inside
the library is the only place the program's own values are visible.",
        example: "Rejected \u{2014} the visitor types the object key:

    foreign putObject is server
        from  \"./s3\" as \"put\"
        takes key is trusted Text, body is Text
        gives Text

    state receipt is server Text from putObject with key is typed, body is \"hi\"

Accepted \u{2014} derive the key from state the program owns, or drop `trusted`
from the parameter and record in the declaration why nothing checks it.",
    },
    Explanation {
        code: "E-REL-04",
        caret: "a release's inputs are its parameters and nothing else",
        name: "a release body read a signal",
        meaning: "A `release` body, and everything it calls, may read no signal at all
(rule REL-CLOSED, spec \u{00A7}19.2 rule 8). This body reaches one, directly or
through a function it calls. The diagnostic names the read as well as
the declaration, because the read is usually several calls away.",
        why: "The parameter list is meant to be the release's entire input. If a body
may also read state, then what a release declassifies is not what its
call sites pass it, and the audit a reviewer performs at the declaration
no longer describes what runs. Closing the body is what makes the
parameter list worth reading.",
        example: "Rejected \u{2014} the body reaches `cards`, which no call site passed:

    state cards is server Text starting \"\"

    release digitOracle with guess
        gives Whole
        give cards

Accepted \u{2014} take the value as a parameter, so that every call site names
what it hands over.",
    },
    Explanation {
        code: "E-REL-08",
        caret: "no grant accounts for this argument",
        name: "an unendorsed release argument the compiler cannot trace to a grant",
        meaning: "This argument is Untrusted \u{2014} no grant in \u{00A7}21.7.3's closed set covers it \u{2014}
and the parameter it lands on is not named in the declaration's `trusted`
clause (rule REL-ARG, spec \u{00A7}19.10.1). The diagnostic prints the argument
and the declaration, so both ends of the flow are findable.",
        why: "A release turns Secret into Public, so who steered the call matters as
much as what came out. Naming the parameter in a `trusted` clause is a
human signing for it, and the point of the rule is that the signature is
written down at a declaration rather than assumed. It reports what it
can trace and nothing further: a call with no E-REL-08 is not thereby a
call nobody steered.",
        example: "Rejected \u{2014} a browser chose the value this release is asked about:

    release digitOracle with all, holder
        gives Whole
        trusted all
        give 0

    state hits is server Whole from digitOracle with all is cards, holder is typed

Accepted \u{2014} write `trusted holder` if a reviewer has read the call sites
and will sign for them, or pass a value that derives from a grant.",
    },
    Explanation {
        code: "E-REL-10",
        caret: "this foreign declares neither `pure` nor `trusted`",
        name: "a release body reached a foreign declaring neither `pure` nor `trusted`",
        meaning: "A `release` body may reach a `foreign` only if its `gives` line carries
`pure` or `trusted` (rule REL-PURE, spec \u{00A7}21.7.3 as amended by \u{00A7}21.9).
This body reaches one that carries neither. `is client`, `is server` and
`is anywhere` answer a different question \u{2014} which bundles the library may
be linked into \u{2014} and say nothing about the result.",
        why: "The rule used to read `is anywhere` as though it meant *the result is a
function of the arguments*. It does not: a query-string reader is
honestly `is anywhere`, and its result is whatever a visitor typed into
the URL. Separating the two questions is what this code exists for. The
marker itself is asserted about JavaScript nobody reads \u{2014} it moves an
obligation onto a human at a conspicuous declaration, and does not
establish anything about what the JavaScript does.",
        example: "Rejected \u{2014} `query` reads the URL, and `is server` does not say otherwise:

    foreign queryParam is server
        from  \"zd:http\" as \"query\"
        takes key is Text
        gives Text

    release digitOracle with guess
        gives Whole
        give queryParam with key is guess

Accepted \u{2014} write `gives pure Text` if the result really is a function of
the arguments, or lift the value into the release's parameter list where
an endorsement has to name it.",
    },
    Explanation {
        code: "W-REL-01",
        caret: "no clause caps how often one session evaluates this",
        name: "a release with no `limit`",
        meaning: "The `gives` type is how much one evaluation may disclose. Without a
`limit` clause there is no ceiling on how many times one session may
evaluate the declaration, so the per-evaluation figure is the only
number written down anywhere (spec \u{00A7}19.4).",
        why: "A reviewer reading `gives Whole` will read it as the size of the
disclosure, and for a single call it is. The warning exists so that the
missing second number is visible at the declaration rather than being
discovered by arithmetic later.

Writing a `limit` does not bound cumulative disclosure, and the warning
is careful not to say it does: the budget is per declaration and per
anonymous session, a second declaration carries its own, clearing a
cookie mints a fresh one, and budgets are not enforced at all until
durable storage exists.",
        example: "Warned \u{2014} nothing caps how often a session asks:

    release judge with guess
        gives Text
        give guess

Quieter \u{2014} the count is now written down, with the caveats above:

    release judge with guess
        gives Text
        limit 10 per visitor
        give guess",
    },
];
