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
        code: "E0107",
        caret: "there is no visitor the compiler could key this by",
        name: "a declaration named a principal the language cannot establish",
        meaning: "`durable per visitor` asks for storage partitioned per principal. The
language has no principal. `durable` state is one value shared by every
visitor, and there is no second, per-visitor kind of it.",
        why: "Per-visitor storage needs an identity, and an identity has to arrive
from somewhere. The only channel is the request, and a request here is
an endpoint name and a JSON array of arguments — no headers, no
cookies, no session. Establishing one means minting and checking a
credential, which is authentication, which is a v1 non-goal in the same
breath as per-user durable scoping. They are listed together because
they are one problem.

Building it anyway would not give you a visitor. An anonymous session
cookie names a browser profile: two people sharing a machine get one
partition, one person on a phone and a laptop gets two, and anyone
holding the cookie is the principal. The separation would rest on three
things this compiler cannot check — that the deployment marks the
cookie `HttpOnly`, `Secure` and `SameSite`; that its value is
unguessable; and that the store backend honours the partition prefix
instead of ignoring it. A placement spelled `per visitor` that delivers
that would read as isolation to everyone who used it, which is worse
than not having it, so the compiler refuses the words rather than
implement something it cannot stand behind.

What this leaves true is worth saying plainly: scoping a durable value
to one visitor is your program's job, and nothing checks that you did
it. A durable row is visible to every request that computes its key.",
        example: "Rejected — a placement the language does not have:

    state hits is durable per visitor Whole starting 0

Accepted — one durable value, scoped by the program, and understood to
be scoped by nobody else:

    state hits is durable Map of Text to Whole starting empty",
    },
    Explanation {
        code: "E0201",
        caret: "this value is not the type this position takes",
        name: "a value is not the type the position it sits in requires",
        meaning: "Inference solved the program's equations and two of them disagree here.
The type this expression produces is not the type the place it was
written into accepts, and the message names both — what the value
starts as, and what is expected — because either one of them can be
the mistake.",
        why: "§5.4's type system converts nothing on its own. That is the decision
this diagnostic enforces and it is worth its friction: an implicit
conversion is a rule you have to know in order to read a line, and it
is nowhere in the line. `Whole` and `Decimal` are the pair that makes
the point. A language that narrowed one to the other quietly would
answer `3 / 2` with `1` in some positions and `1.5` in others, and
neither number would be written anywhere in the program.

The expectation usually comes from somewhere other than the caret: a
`state` declaration's written type, a parameter's use further down its
own function, an element's argument. The message names it rather than
pointing at it, because the type it names is often not written in one
place at all — it is what the rest of the program forced.",
        example: "Rejected — `/` gives a `Decimal` whatever it divides, so the written
type and the initialiser disagree:

    state half is client Whole from 3 / 2

Accepted — divide as whole numbers. `quotient` is `None` on a zero
divisor, because a `Whole` is finite, so the `Option` is spent here:

    state half is client Whole from valueOr with maybe is (quotient with value is 3, divisor is 2), fallback is 0",
    },
    Explanation {
        code: "E0202",
        caret: "this operand is outside the set the operator takes",
        name: "an operator or built-in was given a type it does not accept",
        meaning: "Each operator and each built-in accepts a closed set of operand types:
`+` takes `Whole`, `Decimal` or `Text`; `<` takes a number; `at`
indexes a `List`, a `Map` or a `Text`; `length of` and `contains` take
a `Text`, a `List` or a `Map`. This one was given a type outside its
set, and the message names the set rather than only the value.",
        why: "There are no typeclasses and no operator overloading (§5.4), so what
`+` means is fixed by the language rather than by what is to its left.
The cost is real — there is no way to teach `+` about a `record` you
declared, and a program that wants one writes a `function` with a name
that says what it does. The benefit is that `a + b` means the same
thing in every file, including files you did not write, and that
reading an expression never requires first working out which
implementation of an operator is in scope.

The set is named in the message because the set is the part that is
not in the source. Which types `contains` accepts is a fact about the
language; which type this value has is a fact about the program, and
the caret already points at that one.",
        example: "Rejected — `contains` looks inside a collection or a text, and a
`Truth` is neither:

    state ok    is client Truth starting yes
    state found is client Truth from ok contains \"x\"

Accepted — ask the question of something that has things inside it:

    state words is client List of Text starting [\"x\"]
    state found is client Truth from words contains \"x\"",
    },
    Explanation {
        code: "E0203",
        caret: "this type would have to contain itself",
        name: "a value would have to be a value that contains itself",
        meaning: "Solving this expression's equations produced a type that appears
inside its own definition — a list whose element type is that same
list, and so on without end. This is the occurs check, and it is the
one failure inference reports about its own arithmetic rather than
about a written type.",
        why: "A type is a finite description of a value's shape, and a value of an
infinite type is a value nothing can build or print. Unification would
loop forever constructing one, so the check is what makes inference
terminate — the classic reason, and it holds here for the ordinary
reason it holds anywhere.

In practice this almost always means a name is being used at two
depths at once: as a collection in one place and as one of its own
elements in another. The repair is nearly always to introduce the
second name rather than to change a type.",
        example: "Rejected — `xs` would have to be both the list and one of its
elements:

    function grow with xs
        give append xs to xs

Accepted — the element is its own value, and the list holds it:

    function grow with xs, item
        give append item to xs",
    },
    Explanation {
        code: "E0204",
        caret: "nothing in the program settles what this is",
        name: "nothing in the program says what type this is",
        meaning: "The checker finished and this expression's type was still open. `at`,
`contains`, `length of`, `map each … in`, `when` and `empty` each need
to know *which* shape they are working on — a `Text` is indexed
differently from a `Map` — and nothing anywhere in the program pinned
this one down.",
        why: "The alternative is a default, and every default here is a silent
choice about what the code does. Guessing `List` for an `empty` that
was meant to be a `Map` produces a program that compiles, runs, and is
wrong in a way no diagnostic will ever mention again.

Inference is deliberately whole-program rather than per-line (§5.4),
so this is not a demand for annotations everywhere — most values need
none, because some use somewhere settles them. It is the report that
*no* use settled this one, which is why the repair is to write the
type on the declaration the value comes from rather than at the caret.",
        example: "Rejected — nothing anywhere says what `xs` is, so `length of` does not
know what it is counting:

    function sizeOf with xs
        give length of xs

Accepted — a call settles it, and the parameter needs no annotation of
its own:

    state words is client List of Text starting [\"a\", \"b\"]
    state count is client Whole        from sizeOf with xs is words

    function sizeOf with xs
        give length of xs",
    },
    Explanation {
        code: "E0210",
        caret: "this value has no variants to take apart",
        name: "`when` was given something that is not a choice",
        meaning: "`when` takes apart a value that is one of several things: an `Option`,
a `Remote`, a `Code`, or a `choice` this program declares. The value
under the caret is none of those, so there are no variants to write
arms for and the message names the ones there are.",
        why: "`when` is the language's only elimination form, and it eliminates
*sums* specifically. A `Whole` is not a sum: it has no variants, so no
set of arms could be exhaustive over it and the construct has nothing
to check. Branching on a condition is `if`,
which is a different construct because it answers a different
question — `if` asks whether something is so, `when` asks which of
several things this is.

Confusing the two usually means the value is not the one intended: a
`server` signal read from the view is a `Remote of T` and takes a
`when`, and the same signal read from a server derivation is a plain
`T` and does not (§14G.1.4).",
        example: "Rejected — a number is not one of several things:

    state count is client Whole starting 0

    view
        Column
            when count
                Loading show Spinner

Accepted — ask the yes-or-no question with the construct for it:

    view
        Column
            if count > 0
                Text \"some\"",
    },
    Explanation {
        code: "E0211",
        caret: "an arm is missing from this `when`",
        name: "a `when` does not write an arm for every variant",
        meaning: "Every variant of the choice gets an arm, and this `when` leaves at
least one out. The message names the ones that are missing.",
        why: "§14G.1.6 asks for all of them **in every context**, including arms the
compiler can prove will never run. That last part is the deliberate
one, and it is what makes a loading state impossible to forget: a
`Remote` has three arms because a call over a network has three
outcomes, and a program that writes only `Ready` is a program that
renders nothing at all while the request is in flight and nothing at
all when it fails. Those are the two states a user actually
experiences on a bad connection, and they are exactly the two a
language with an optional else-branch lets you skip.

The unreachable-arm rule follows from the same argument in the other
direction. If arms could be omitted when the compiler can prove them
dead, then whether a program compiles would depend on how clever the
prover was that week, and adding a variant to a `choice` would be a
change whose consequences appear somewhere else entirely.",
        example: "Rejected — nothing is drawn while the request is in flight:

    view
        Column
            when quote
                Ready with text show Text text

Accepted — all three outcomes, each with something to draw:

    view
        Column
            when quote
                Loading         show Spinner
                Failed with e   show ErrorBar message is e.message
                Ready with text show Text text",
    },
    Explanation {
        code: "E0212",
        caret: "this arm does not name one variant with its fields",
        name: "an arm does not match the choice it takes apart",
        meaning: "An arm names one variant of the choice, once, and binds exactly the
fields that variant carries, in the order they were declared. This one
does not: it names a variant the choice does not have, it names one a
previous arm already took, or it binds a number of names the variant
has no fields for.",
        why: "The three are one rule — an arm is a *pattern*, and a pattern that
does not correspond to the shape it takes apart cannot be run. Naming
a variant twice is worth reporting rather than accepting because the
second arm can never run: the code is there, it looks like it does
something, and it does nothing, which is the failure mode this
language treats as a defect wherever it appears.

The binders are positional and unnamed on purpose. A variant's fields
are declared in an order, the arm restates that order, and there is
nothing to keep in sync — where a name-per-field syntax would let an
arm bind `title` to what the declaration calls `body` and typecheck.",
        example: "Rejected — `Some` carries one field, and this binds two:

    when found
        Some with value, extra show Text value
        None                   show Text \"none\"

Accepted — one name per field the variant declares:

    when found
        Some with value show Text value
        None            show Text \"none\"",
    },
    Explanation {
        code: "E0220",
        caret: "this argument list does not fill the declaration",
        name: "a call does not fill the parameters the declaration names",
        meaning: "A call fills every parameter the declaration lists, exactly once,
positionally or by name. This one passes an argument for a parameter
that does not exist, passes more arguments than there are parameters,
or leaves one unfilled. The message names the parameters the
declaration has.",
        why: "There are no default arguments and no optional parameters, and both
absences are the same decision. A default is a value written in the
declaration and read at a call site that does not mention it, so the
call no longer says what it does — and the reader who has to know the
default is the reader least likely to have the declaration open.
Filling every parameter costs a few characters at the call and buys a
call site that can be read on its own.

`with` names its arguments for the neighbouring reason (§4.4). A call
with three positional arguments is a call whose meaning depends on an
order written somewhere else, and swapping two of the same type is a
mistake no compiler can catch. Naming them turns that into this
diagnostic.",
        example: "Rejected — the declaration has no `divisor`, and `n` is unfilled:

    function halve with n
        give n / 2

    state half is client Decimal from halve with divisor is 2

Accepted — the parameters the declaration names, each filled once:

    state half is client Decimal from halve with n is 4",
    },
    Explanation {
        code: "E0221",
        caret: "this does not name every field exactly once",
        name: "a record or variant was not built by naming every field once",
        meaning: "A `record` and a variant that carries fields are built the same way:
`Name with field is …, other is …`, naming every field the declaration
lists, each of them once. This one leaves a field out, names one the
declaration does not have, gives one twice, or writes the shape's name
with no fields at all.",
        why: "Every field is given a value because **there is no value in this
language that stands for nothing**. There is no `null` and no
`undefined`, so a half-built record is not a thing that could exist
and be checked later — the language would have to invent a filler, and
the filler would then be a value the program never wrote and every
reader would have to know about.

A field that may genuinely be absent is spelled `Option of T`, which
is a different type and takes a `when`. That is more to write, once,
at the declaration; the alternative is more to check, at every read.",
        example: "Rejected — `Post` declares three fields and this names two:

    record Post
        slug  is Text
        title is Text
        draft is Truth

    state first is client Post starting Post with slug is \"a\", title is \"A\"

Accepted — every field named, including the one whose value happens to
be the boring one:

    state first is client Post starting Post with slug is \"a\", title is \"A\", draft is no",
    },
    Explanation {
        code: "E0222",
        caret: "this names a declaration rather than a value",
        name: "a declaration was written where a value goes",
        meaning: "The name under the caret resolves to a `function`, a `component`, a
`record`, a `choice` or something else the program declared — not to a
value. The message says which kind it is and how that kind is spelled
where it *is* usable.",
        why: "There are no first-class functions (§5.4), so a function's name is not
a value that can be passed, stored or returned: it can only be called.
That is a real limitation and it is written down rather than worked
around, because the alternative — closures as values — brings a type
system with higher-rank types and an escape analysis for the
placements, and neither is in this language.

The other kinds are refusals of a subtler mistake. A `record`'s name
is a *shape*, and a shape is not one of its own instances; a
`choice`'s name is a set of variants, and a set is not a member. Both
mistakes read as though the program had a value, which is why the
message names the spelling that would produce one.",
        example: "Rejected — the name of a record is not a record:

    record Post
        slug is Text

    state first is client Post starting Post

Accepted — build one by naming its fields:

    state first is client Post starting Post with slug is \"a\"",
    },
    Explanation {
        code: "E0223",
        caret: "this value has no field of that name",
        name: "a field was read from a value that does not carry it",
        meaning: "`.` reads a field of a `record`, of a variant's payload, of an event's
payload, or of an `Error`. The value under the caret carries no field
of this name, and the message lists the fields it does carry.",
        why: "The set of fields is closed at the declaration, and there is no
dynamic lookup: a name that is not a field is a mistake now rather
than `undefined` later. The types that are not records are the
interesting half of this rule. An event payload carries what the
browser reports for *that* event and nothing else, which is why
`press.key` reads on `keydown` and not on `click`; and an `Error`'s
fields are a closed pair for §14G.1.3(d)'s sake, so that `e.code` — the
browser's own account of the transport — is always available and
`e.message` is not always.

A `choice` is the one case where the message points somewhere else
entirely: its variants are not fields, and the way in is `when`.",
        example: "Rejected — `Todo` has no `name`:

    record Todo
        title is Text
        done  is Truth

    state first is client Todo starting Todo with title is \"a\", done is no

    view
        Column
            Text first.name

Accepted — a field the declaration lists:

    view
        Column
            Text first.title",
    },
    Explanation {
        code: "E0230",
        caret: "this clause has no collection left to walk",
        name: "a pipeline clause with nothing to walk",
        meaning: "A pipeline starts with `from`, which names the collection, and each
later clause walks what the one before it produced. This clause has no
collection in front of it: either the pipeline never started with
`from`, or a `fold each` already ended it.",
        why: "`fold each` is the clause that turns a sequence into one value, so
what follows it is not a sequence and there is nothing left to walk.
The rule is stated as a property of the pipeline rather than left to
whatever the runtime would do, because the alternative — a clause that
quietly walks a one-element sequence — would make `fold each` mean two
different things depending on what came after it.

The `from`-first rule is what makes a pipeline readable top to bottom:
the collection is named once, at the top, and every clause below is
about that collection. A pipeline whose source could appear anywhere
would have to be read backwards to find out what it was about.",
        example: "Rejected — the fold has already produced one number:

    function totalOf with xs
        from xs
        fold each x into sum starting 0 to sum + x
        keep each x where x > 0

Accepted — filter first, then fold:

    function totalOf with xs
        from xs
        keep each x where x > 0
        fold each x into sum starting 0 to sum + x",
    },
    Explanation {
        code: "E0240",
        caret: "this is not somewhere a value can be put",
        name: "something was written to that is not a place",
        meaning: "`set`, `add`, `subtract`, `append` and `remove` write into a place: a
`state` signal that stores its value. The target here is not one — it
is a derived signal, a clock, a variant, or the name of something that
is not state at all — and the message says which.",
        why: "A derived signal has one definition of where its value comes from, and
that definition is its `from` clause. A write would give it a second,
and the two would disagree the moment either input changed; which one
won would depend on the order the graph happened to recompute in.
Writing the input instead is not a workaround, it is the same
operation expressed where the compiler can see it.

A clock signal is refused for the sharper version of the same reason:
its writer is the browser's scheduler, and a program that could also
write it would be racing something with no rate it can predict.",
        example: "Rejected — `doubled` is recomputed, so there is nothing to assign to:

    state count   is client Whole starting 0
    state doubled is client Whole from count * 2
    ...
        on click
            set doubled to 10

Accepted — write the input, and let the derivation follow:

    on click
        set count to 5",
    },
    Explanation {
        code: "E0241",
        caret: "this binding needs state it can write back to",
        name: "an element that binds two ways was not given writable state",
        meaning: "`Input`, `Checkbox`, `Slider` and the rest of the two-way elements
write back into what they are given on every keystroke or click, so
what they are given has to be a `state` signal that stores its value.
This one was handed a computed value, a derived signal, or state on a
placement the browser cannot write.",
        why: "Two-way binding is the one place in the language where a *view node*
is a writer, and the whole of its honesty rests on the target being
somewhere a write can land. A computed value would take the keystroke
and drop it, which is the failure this compiler treats as a defect
wherever it appears: the field would look like it worked, and the
character would be gone by the next repaint.

The placement half is a different point. A `server` or `durable`
signal is not in the browser, so binding a field straight to one would
hide a network round trip inside a keystroke — one request per
character, with no place in the source that says so. Binding a
`client` signal and writing the remote one from a handler puts the
round trip where it can be read (§14B.5).",
        example: "Rejected — a derived signal cannot take the keystroke:

    state name  is client Text starting \"\"
    state shout is client Text from uppercase of name

    view
        Input shout

Accepted — bind the signal that stores, and derive from it:

    view
        Column
            Input name
            Text shout",
    },
    Explanation {
        code: "E0250",
        caret: "this state cannot be reached from here",
        name: "state was read from a context that cannot reach it",
        meaning: "Where a signal lives decides who can read it, and this read is from
somewhere that cannot. The message names the placement and the reason
— the build host has no browser, a trigger has no session, and so on
— rather than only saying no.",
        why: "This is §14G.1.4's table, enforced. Placement is not a hint about
performance: the four placements are four machines, and a read that
crosses between them either becomes a network call with a type that
says so (`Remote of T`) or is impossible. This code is the second
case, and it is reported by the type checker because the *type* of a
read is what changes across the boundary.

Reporting it here rather than at code generation is what makes the
answer a compile error instead of a value that is `undefined` in a
browser somebody else is using.",
        example: "Accepted — cross the boundary explicitly, in a `server` signal the
view asks for, and spend the `Remote` with a `when`:

    state query   is client Text         starting \"\"
    state matches is server List of Item from search with query",
    },
    Explanation {
        code: "E0260",
        caret: "this is not what the element takes",
        name: "an element was given arguments it does not take",
        meaning: "Every element declares what it takes: how many leading values, which
named arguments, and — for the ones that bind two ways — what type the
state it binds must have. This one was given something else. The
message names what the element takes; the caret is on what was
written.",
        why: "The element vocabulary is a closed set with a fixed shape per element
(§14D), and the shape is what lets the view be read without a
component library open beside it. `Text` leads with the value it
shows, `Link` leads with where it goes and nests what it shows, and
`NumberInput` binds an `Option of Whole` because an empty field holds
no number at all and the state it writes has to have somewhere to put
that.

That last one is worth stating, because it is the rule that looks like
pedantry and is not. A number field that bound a plain `Whole` would
have to invent a value for the moment the reader clears it — zero,
usually — and a form that silently reads zero when somebody meant to
type nothing is a bug that reaches production in every framework that
allows it.",
        example: "Rejected — an empty field holds no number, so a plain `Whole` has
nowhere to put that:

    state age is client Whole starting 0

    view
        NumberInput age

Accepted — the absence is in the type, and the view spends it:

    state age is client Option of Whole starting None

    view
        NumberInput age",
    },
    Explanation {
        code: "E0270",
        caret: "the boundary cannot carry this type",
        name: "a `foreign` declaration promises something the boundary cannot carry",
        meaning: "A `foreign` declaration is a promise about JavaScript the compiler
cannot read: what the imported symbol takes, and what it gives. This
declaration writes a type the boundary has no representation for —
either a `gives new` whose type is not `Handle`, or a parameter of a
`gives view` foreign whose type has no plain JavaScript form.",
        why: "§14E.3 makes the FFI a hole in the type system deliberately: what
comes back is whatever the JavaScript returns, and the declaration is
the audit surface. A hole is only usable if its edges are exact, which
is what this rule keeps true. `new` builds a host object, and the
language's name for a host object is `Handle` — writing any other type
there would be a promise the compiler cannot check *and* cannot even
state, because there is nothing about a constructed JavaScript object
that corresponds to a `record` this program declared.

The `gives view` case is the same argument about arguments. A view
foreign is handed plain JavaScript values, so its parameters are
`Text`, `Whole`, `Decimal`, `Truth`, or a `List` of one of those.
Anything else would need a marshalling rule, and a marshalling rule
the compiler invented would be invisible in the declaration that is
supposed to be the whole of the contract.",
        example: "Rejected — `new` gives a host object, and `Scene` is not a type this
language has:

    foreign newScene is client
        from  \"three\" as \"Scene\"
        gives new Scene

Accepted — the language's name for a host object:

    foreign newScene is client
        from  \"three\" as \"Scene\"
        gives new Handle",
    },
    Explanation {
        code: "E0271",
        caret: "this owns a node, so it is not a value",
        name: "a view `foreign` was used as a value, or given children",
        meaning: "A `foreign … gives view` mounts a DOM node and owns it. It is written
as a view element and hands back no value, so it cannot be called for
a result, and nothing may be nested underneath it.",
        why: "Ownership is the whole of what `gives view` declares. The module is
handed a node, it does what it likes inside — a canvas, a chart, a
map — and the compiler stops reasoning about that subtree entirely.
Nesting a ZDeceptron node under it would put two owners on one region
of the DOM, and the loser would be whichever one wrote last, which is
a race with no rate anybody can predict.

Calling it for a value is the same confusion the other way round. The
declaration says the result is a *view*; there is no value to give
back, so an expression that used one would be reading whatever the
module happened to return, which is exactly what §14E.3's hole is
bounded to prevent.",
        example: "Rejected — the module owns the node, so nothing goes under it:

    foreign Gauge is client
        from  \"./gauge.js\" as \"mount\"
        takes value is Decimal
        gives view

    view
        Column
            Gauge value is 0.5
                Text \"inside\"

Accepted — the element on its own, and anything else beside it:

    view
        Column
            Gauge value is 0.5
            Text \"beside\"",
    },
    Explanation {
        code: "E0272",
        caret: "this call gives a value, and `do` discards nothing",
        name: "`do` was given a call that gives a value",
        meaning: "`do` runs a call for its effect. It is written for a `foreign … gives
nothing` — an imperative JavaScript call with no result — and this one
gives a value.",
        why: "A statement that quietly threw a result away would be the one place in
this language where a computed value can vanish with nothing said
about it. Every other construct puts its result somewhere a reader can
see: a `give`, a `state` declaration, an argument. `do` exists
precisely because a `gives nothing` foreign has no result to put
anywhere, and widening it to \"any call, result discarded\" would turn
it into the general-purpose sink that makes a dropped return value
invisible.

If the result is genuinely not wanted, that is a fact about the
`foreign` declaration — write `gives nothing` there, where a reviewer
reading the boundary can see it.",
        example: "Rejected — the declaration gives a value, so `do` is not how it is
spent:

    foreign store is client
        from  \"./db.js\" as \"put\"
        takes key is Text
        gives Text

    do store with key is \"a\"

Accepted — declare what is true of the JavaScript, and `do` fits:

    foreign store is client
        from  \"./db.js\" as \"put\"
        takes key is Text
        gives nothing",
    },
    Explanation {
        code: "E0280",
        caret: "some path through this function reaches no `give`",
        name: "a function does not give a value on every path",
        meaning: "Every path through a function must reach a `give`. This one has a path
that runs off the end — usually an `if` with no `otherwise` and no
final `give` after it.",
        why: "There is no value in this language that stands for nothing, so there
is nothing for a function to return when it falls off the end. Other
languages fill the gap with `undefined`, `nil` or `None`, and the cost
is paid by every caller: the result of a call is a value *or* the
absence of one, forever, and the check for that absence is the
programmer's to remember.

A function that really may not have an answer says so in its type by
giving an `Option of T`, which the caller then has to take apart with
`when`. That is the same information, written where the caller reads
it rather than discovered where the caller does not.",
        example: "Rejected — nothing is given when the number is zero or less:

    function adviceFor with count
        if count > 0
            give \"something waiting\"

Accepted — a final `give` the fall-through reaches:

    function adviceFor with count
        if count > 0
            give \"something waiting\"
        give \"nothing waiting\"",
    },
    Explanation {
        code: "E0290",
        caret: "the browser does not report this",
        name: "an `on` handler names an event, key or payload that does not exist",
        meaning: "`on` names one of a closed set of events, and each event has a payload
the compiler knows the fields of. This handler names an event that is
not in the set, a key the browser never reports, or binds a payload
from an event that carries none.",
        why: "The set is closed so that every payload has a field list and a
provenance. A field list is what makes `press.key` a compile-time read
rather than a lookup that may be `undefined`, and a provenance is what
lets the integrity pass know that everything in an event payload was
chosen by a browser (§18.1). Adding an event is therefore a row in the
compiler's table — a name, its fields, and their labels — and not a
spelling that passes through.

Key names are checked against what a browser actually reports, for a
narrower reason: a handler on a key that does not exist is not a
compile error in any other framework, it is a feature that silently
never fires, and the programmer finds out from a user.",
        example: "Rejected — the browser has no `hover` event, so this handler could
never run:

    Button \"go\"
        on hover
            set lit to yes

Accepted — one of the events the message lists, and the payload bound
where the event has one:

    Button \"go\"
        on click
            set lit to yes

    Input typed
        on keydown with press
            set last to press.key",
    },
    Explanation {
        code: "E0108",
        caret: "this names a trigger the compiler cannot yet place",
        name: "the declaration names a construct that is designed and not built",
        meaning: "`inbound \"stripe/payment\"` declares a signal that an HTTP request from
outside the deployment writes — a webhook. It is §14G.4's other trigger,
it is reserved in the grammar, and it is not built. The word stays an
ordinary name everywhere else, so nothing else about your program
changes.",
        why: "What is missing is not the plumbing; the scheduled half of §14G.4 shares
almost all of it. It is that an `inbound` root is an **unauthenticated
public HTTP endpoint**, and three separate rules were settled for it and
are not enforced here yet.

A `release` may not be called from one at all (REL-PLACE′): otherwise an
anonymous caller reaches a declassification site with one request, which
§21.7.8(c) decided against and this compiler does not check, because
until the scheduled trigger landed no program could construct that root
to check it on. The payload is Untrusted and the `pc` at the root must
be seeded Untrusted with it. And delivery is at-least-once with no
uniqueness constraint anywhere in the language, so a redelivered webhook
double-appends — §14G.4 records that as a gap for relational persistence
to close, not for this construct to paper over.

Reserving the word is what keeps the design from being spent. A webhook
that compiled and was unauthenticated would be a worse answer than one
that does not compile.",
        example: "Rejected — the trigger exists in the design and not in the compiler:

    state paid is server Text inbound \"stripe/payment\"

Accepted — the trigger that is built, on the same word and in the same
slot, running on the deployment's schedule rather than on a request:

    state hourly is server Whole every \"1h\"
        add 1 to visits",
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
        code: "E0322",
        caret: "only the browser has a clock to run this on",
        name: "a clock signal was placed somewhere with no clock",
        meaning: "`every \"250ms\"`, `every frame` and `after \"2s\"` declare state the
browser's scheduler writes. They are `client` state and nothing else.

`every` on a `server` declaration is a different construct and is not
refused here: it is a job the deployment runs on a schedule, written
`every \"1h\"` with the work indented under it.",
        why: "Each of the other placements fails for its own reason. `static` is
computed once at build time and inlined, so there is no later for a tick
to happen at: every visitor would be served the one number the build
stopped on. `durable` is storage — a value is in the store because
something wrote one, not because something is still running — and
`remembered` is storage too, on the browser's side, so a restored
reading would be an elapsed time measured from a visit that has ended.

The `server` arm of this rule used to say that a scheduled job was a
construct the language had sketched and not built. That has stopped
being true, and what is refused on a `server` declaration now is
`after` alone: a delay needs a moment to count from, and a serverless
invocation has none — it starts when a request arrives, which is not a
time the program chose. A repeating job has one, because the schedule
supplies it.

The two readings of `every` never share a spelling. A browser interval
is written in `ms`, `s` or `m` and stops at `\"60m\"`; a cadence is
written in `m`, `h` or `d` and is one of nineteen that divide their
unit, so that one cron rule names every beat on every target.",
        example: "Rejected — a build has no later for a tick to happen at:

    state elapsed is static Decimal every \"250ms\"

Accepted — the browser's clock, in the browser:

    state elapsed is client Decimal every \"250ms\"

Accepted — the same word on the deployment, where it is a job:

    state hourly is server Whole every \"1h\"
        add 1 to visits",
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
        code: "E0362",
        caret: "there is no browser here to ask",
        name: "`media` was read outside the browser",
        meaning: "`media \"(prefers-color-scheme: dark)\"` asks the browser whether it
matches a CSS media query. This code does not run in a browser.",
        why: "A media query is answered by the display a visitor is looking at, and
by nothing else. A build host has no display and a serverless
invocation has no visitor, so there is nobody to ask — the same shape
of rule as E0360 and E0361, pointed at the third machine. This is a
refusal where the question cannot be answered rather than a permission
that could be granted more widely.",
        example: "Accepted — read it into a `client` signal, which the browser evaluates
and keeps up to date as the visitor's preference changes, and send that
to the server if the server needs to know:

    state dark is client Truth from media \"(prefers-color-scheme: dark)\"",
    },
    Explanation {
        code: "E0364",
        caret: "there is no document here to listen to",
        name: "a document key handler outside the browser",
        meaning: "`on key \"Escape\"` registers a listener on the browser's document and
keeps it until the view that wrote it is discarded. This code runs
somewhere that has no document to register it on.",
        why: "A build host has no browser at all. A server has no browser of its
own: it renders for one, so a listener registered there would either
not exist or belong to whichever visitor's request happened to be in
flight — which is worse than not existing, because it looks like it
works. The rule is stated as a property of the region rather than as
a list of the regions to refuse, so a region added later has to
answer the question instead of inheriting permission.

The narrower question this handler exists to answer is what it may
*observe*. A document listener sees keystrokes aimed at every element
on the page, including a field this program never declared. So the
construct names its key and receives nothing: there is no binder,
because the program already knows which key it is and every other key
is exactly what it must not see. The emitted listener also stands down
while focus is inside an editable element, so the key it names cannot
be a character somebody is typing into a field.",
        example: "Rejected — a `static` signal is computed on the build host, and there
is no browser there:

    state shortcut is static Truth from armed

Accepted — write it in the view, where the browser is, and where its
lifetime is the lifetime of the nodes around it:

    if open
        Dialog
        on key \"Escape\"
            set open to no",
    },
    Explanation {
        code: "E0363",
        caret: "there is no browser here to send it",
        name: "a request was reached from outside the browser",
        meaning: "A `request` declaration is `client`-placed: a browser issues it, waits
for it, and holds the `Remote of Text` it produces. This code does not
run in one.",
        why: "A request the *deployment* sends is a different question, not a wider
version of this one. What it would spend is the deployment\'s own
credentials and its position inside a private network, so which hosts
it may reach has to be bounded by whoever owns the deployment rather
than by whoever wrote the program — and §14G.1.3(c) would need a sink
of its own for it, because \"a request the deployment sends\" is a
different medium with a different reader. Neither exists yet, so the
placement is refused rather than quietly given the browser\'s rules.",
        example: "Accepted — the declaration is `client`, and the value it produces is
spent with the three-armed `when` every `Remote` needs:

    request quote is client
        from  \"/quote.txt\"
        gives Text",
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
anonymous session, a second declaration carries its own, and clearing a
cookie mints a fresh one.

Nothing counts them either. The clause changes the call site's type to
`Option of T` so that running out has to be handled, and no counter is
emitted anywhere, so the exhausted case never arrives. This paragraph
used to end \u{201C}until durable storage exists\u{201D}; that storage exists now and
the budget is still wired to nothing.",
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
    Explanation {
        code: "E-TEST-01",
        caret: "this expectation came out `no`",
        name: "a claim the program contradicted",
        meaning: "A `test` declaration states one claim and gives one expectation as the
evidence for it. The expectation was evaluated, it terminated, and it
produced `no`. The compiler is not reporting that the program is
wrong \u{2014} it is reporting that the program and the claim disagree, and
which of the two is mistaken is for the reader to decide.",
        why: "Everything else this compiler says is a property it can establish by
reading the program: a name resolves, a type matches, a secret does not
reach the page. Whether a sort is correct is not that kind of property,
and no analysis will make it one \u{2014} it has to be *run*.

So a claim is checked by running the code the compiler emits, in the
engine the compiler already carries for build-time evaluation. What
holds here is what the browser will run, because it is the same
JavaScript; a separate interpreter would be a second implementation
that could disagree with the shipped one exactly where a test is
supposed to notice.

The claim is quoted back verbatim and the caret points at the `expect`
line, for the same reason every other diagnostic names the claim and
shows the span: a failure that says only `assertion failed` has made
the reader do the work of finding out what was being asserted.",
        example: "Broken \u{2014} the claim and the program disagree:

    function double of n
        give n * 2

    test \"doubling four gives nine\"
        expect (double of 4) is 9

The report shows both sides \u{2014} `Left is 8; right is 9` \u{2014} so the
repair is visible without running anything by hand. Either the claim
is wrong and becomes `is 8`, or `double` is wrong and the claim was
right to catch it.",
    },
    Explanation {
        code: "E-TEST-02",
        caret: "this expectation stopped before it decided anything",
        name: "a claim that could not be decided",
        meaning: "The expectation did not produce `yes` or `no`. It exhausted the
build-time work budget, was refused a capability, or something it
called threw. The claim is therefore neither held nor broken, and it
is reported separately from a false one because they call for
different repairs.",
        why: "Reporting an undecidable claim as a *false* one would tell the reader
that their program contradicts the claim, when what actually happened
is that the claim never got far enough to say. They would go looking
for a bug in the code under test instead of at the expectation that
never ran.

The work budget is a bound on loops and recursion rather than a clock,
so a claim that stops here stops on every machine \u{2014} a suite whose
result depended on how busy the host was would be worse than no suite
(spec \u{00A7}17.4.8). The sandbox is the same one a `static` signal is
given: a claim reads the project directory it was pointed at and
nothing else.",
        example: "Undecidable \u{2014} the expectation never terminates:

    function forever of n
        give forever of n

    test \"this claim never gets an answer\"
        expect (forever of 1) is 1

Decidable \u{2014} the function bottoms out, so the claim can be judged:

    function countdown of n
        if n <= 0
            give 0
        give countdown of (n - 1)

    test \"counting down from ten reaches zero\"
        expect (countdown of 10) is 0",
    },
];
