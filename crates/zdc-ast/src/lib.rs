#![forbid(unsafe_code)]

//! Plain data types for the ZDeceptron syntax tree.
//!
//! No logic lives here. The parser builds these; later passes lower them
//! into HIR.

use zdc_lexer::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Ident {
    pub text: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub decls: Vec<Decl>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decl {
    State(StateDecl),
    Function(FunctionDecl),
    View(ViewDecl),
    Record(RecordDecl),
    Choice(ChoiceDecl),
    Component(ComponentDecl),
    Use(UseDecl),
    Foreign(ForeignDecl),
    Route(RouteDecl),
    Release(ReleaseDecl),
    Test(TestDecl),
    Request(RequestDecl),
}

// --- tests (issue #169) ---

/// `test "…"` and one indented `expect <expr>` — a claim about the program
/// and the evidence for it, in one declaration.
///
/// # Why a declaration and not a convention
///
/// Three shapes were on the table and two of them are cheaper to build.
///
/// * **A naming convention** — a function called `testSomething`, or a
///   `static` signal of type `Truth` that the runner looks for. It needs no
///   grammar at all, and it is wrong for this language for the same reason
///   §14G.2 refuses to derive routes from a directory layout: it puts a
///   construct's meaning somewhere the compiler cannot check. Rename the
///   function and the claim silently stops being checked; misspell the
///   prefix and it never was. §4.1 admits one phrasing per construct, and a
///   convention is a phrasing the grammar does not know about.
///
/// * **A separate file format** — `.zdtest` with its own grammar. It needs
///   a second parser, a second resolver and a second answer to every
///   question the first one already answers, and the thing it buys — tests
///   that cannot accidentally ship — is bought here instead by placement
///   (see the lowering in `zdc-resolve`: a test is a build-time value, so
///   there is no bundle for it to reach).
///
/// * **A declaration.** The biggest change, and the only one where a claim
///   is a thing the compiler *knows about*: it is resolved, so a claim
///   about a function that no longer exists fails to compile; it is
///   typechecked, so a claim that is not a `Truth` is a diagnostic rather
///   than a silent pass; and it is placed, so what a claim may read is
///   decided by the same pass that decides it for everything else.
///
/// # Why exactly one `expect`
///
/// A test with several expectations has one name and several claims, so a
/// failure report can name the test or name the claim but not both. One
/// expectation per `test` keeps the name and the assertion in bijection,
/// which is what lets the diagnostic say *this sentence is false* and
/// point at the line that says it. Several claims about one function are
/// several `test` declarations, and the cost of that is one line each.
#[derive(Debug, Clone, PartialEq)]
pub struct TestDecl {
    /// The sentence the test asserts, exactly as written. It is prose, not
    /// an identifier: it is what the report prints and what a reader
    /// searches for, so it is not folded, trimmed or shortened anywhere.
    pub claim: String,
    pub claim_span: Span,
    /// The expression after `expect`, which must be a `Truth`.
    pub expectation: Expr,
    /// The span of the whole `expect` clause, keyword included. This is
    /// what a broken claim's caret covers: pointing at the expression
    /// alone would leave the reader looking at a subexpression with no
    /// indication of which construct rejected it.
    pub expectation_span: Span,
    pub span: Span,
}

// --- routing (spec §14G.2) ---

/// `route Site` — the set of URLs this program answers to.
///
/// A route is a `choice` plus a bijection between its values and URLs
/// (§14G.2). It is declared, not derived from a directory layout: a
/// file-based convention would put the URL space in the file system,
/// which invariant 5 forbids as configuration the compiler cannot check.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteDecl {
    pub name: Ident,
    pub variants: Vec<RouteVariantDecl>,
    pub span: Span,
}

/// `BlogPost is "/blog" with slug is Text in postSlugs`
#[derive(Debug, Clone, PartialEq)]
pub struct RouteVariantDecl {
    pub name: Ident,
    /// The literal prefix, exactly as written. `[slug]` meta-syntax inside
    /// a string is refused for the same reason §6 refuses embedded markup.
    pub path: String,
    pub path_span: Span,
    pub params: Vec<RouteParamDecl>,
    pub span: Span,
}

/// `slug is Text in postSlugs` — one route parameter.
///
/// `in` takes a bare name, never an expression (§14G.2 revision 4): an
/// undelimited expression before a comma-separated list is swallowed by
/// the greedy argument list.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteParamDecl {
    pub name: Ident,
    pub ty: TypeExpr,
    /// The `static` signal this parameter ranges over, if it is
    /// enumerable. A parameter with no `in` is not enumerable, and §18.1
    /// semantics 5 makes it **untrusted**.
    pub enumerated_in: Option<Ident>,
    pub span: Span,
}

// --- modules (spec §14D.2) ---

/// `use "./model" for Item, Status` — the names this file borrows from
/// another one.
///
/// The path is relative to the importing file and the `.zd` extension is
/// implied. One phrasing per construct (§4.1): no wildcard, no aliasing,
/// and no re-export in v1.
#[derive(Debug, Clone, PartialEq)]
pub struct UseDecl {
    pub path: String,
    pub path_span: Span,
    pub names: Vec<Ident>,
    pub span: Span,
}

// --- components (spec §14D.1) ---

/// `component VoteCard with item, votes` — a named run of view nodes,
/// used at the call site exactly as a built-in element is.
///
/// `children` is not in `params`. It is not passed at the call site; it is
/// the nodes nested *under* the call site, so it is recorded separately
/// and positional arguments never have to step over it.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentDecl {
    pub name: Ident,
    pub params: Vec<Ident>,
    /// Where `children` was written in the parameter list, if it was.
    pub children: Option<Span>,
    pub body: Vec<ComponentItem>,
    pub span: Span,
}

/// One line of a component's body: either its own state, or a view node.
#[derive(Debug, Clone, PartialEq)]
pub enum ComponentItem {
    State(StateDecl),
    Node(Node),
}

// --- type declarations (spec §4.4 `typeDecl`, §14B.1 as amended by §14G.1.2) ---

/// One `name is type` line, in a `record` body or a variant's payload.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl {
    pub name: Ident,
    /// `unique id is Whole` — this field is the row's identity (#2).
    ///
    /// What it buys is reconciliation by identity rather than by position.
    /// `BENCHMARKS.md` measures the difference at N = 1,000: removing a row
    /// costs 2,986 crossings positionally and 1 by identity.
    ///
    /// It is not uniformly better, and the trade is real rather than a
    /// rounding error — swapping two rows costs 6 positionally and 997 by
    /// identity, and replacing every row costs 3,000 against 8,000. A list
    /// that churns wholesale is better off without one.
    pub unique: bool,
    pub ty: TypeExpr,
    pub span: Span,
}

/// `record Todo` — a product type whose fields are named.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordDecl {
    pub name: Ident,
    pub fields: Vec<FieldDecl>,
    pub span: Span,
}

/// `choice Status` — a tagged union whose variants carry named fields.
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceDecl {
    pub name: Ident,
    pub variants: Vec<VariantDecl>,
    pub span: Span,
}

/// One variant of a `choice`.
///
/// §14G.1.2: `variant := IDENT ["is" TEXT] ["with" variantField (","
/// variantField)*]`, and a `variantField` is `IDENT "is" type` — the same
/// `name is type` line a record field is, which is why both use
/// [`FieldDecl`].
#[derive(Debug, Clone, PartialEq)]
pub struct VariantDecl {
    pub name: Ident,
    /// What a person is shown where this variant has to be read rather
    /// than matched — today, the text of a `Select`'s option.
    ///
    /// A variant's name is an identifier, so it cannot hold a space, and
    /// without this the only text a `Select` could offer was `DirtBike`.
    /// The alternative considered and rejected was splitting the
    /// identifier at its humps: it gets `DirtBike` right, gets `ATV`
    /// right by accident, and can never say "Dirt bike" or "ATV / Quad" —
    /// a label that is always a mechanical function of the name is not a
    /// label, it is a rendering.
    ///
    /// `Name is "text"` deliberately reads the same as a `route`'s `Home
    /// is "/"`, because it is the same idea: the string a variant is
    /// known by outside the program. Inside it, `when` still dispatches on
    /// the name and nothing else, so this can never change what a program
    /// *means* — only what it shows.
    pub label: Option<String>,
    pub label_span: Option<Span>,
    pub fields: Vec<FieldDecl>,
    pub span: Span,
}

// --- state ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Client,
    /// §14C.3b. Read once at build time and inlined into the bundle.
    /// Reading it from the client yields `T` rather than `Remote of T`,
    /// because no boundary is crossed — Rule 1 (§5.2) is satisfied, not
    /// excepted.
    Static,
    Server,
    Durable,
    /// The browser's own store: one value per browser profile and origin,
    /// surviving a reload, shared between that browser's tabs and shared
    /// with nobody else.
    ///
    /// **`remembered` is to `client` what `durable` is to `server`.** The
    /// language already distinguishes two placements on the far side of
    /// the network by how long their store lives rather than by which
    /// machine runs the code — `server` state is one value per request and
    /// `durable` state outlives every request. This is that same
    /// distinction on the near side, where it was missing: `client` state
    /// is one value per open tab and this outlives the tab.
    ///
    /// It is a placement and not a modifier on `client` because the rules
    /// that matter are keyed on the placement. `Writers::of` decides
    /// whether a cell has a writer the program cannot see by matching on
    /// this enum; `may_be_secret` decides whether a secret may live here
    /// by matching on it; `int_01` decides whether `trusted` is spellable
    /// by matching on it. A flag beside `Placement::Client` would be
    /// invisible at all three, and each would have had to grow a conjunct
    /// nobody was compelled to write.
    Remembered,
}

impl Placement {
    /// Every placement, in §5.1's order. Anything that must consider all
    /// of them iterates this rather than writing the list out again.
    pub const ALL: [Placement; 5] = [
        Placement::Client,
        Placement::Static,
        Placement::Server,
        Placement::Durable,
        Placement::Remembered,
    ];

    /// A placement's position in [`Placement::ALL`].
    ///
    /// Total, and that is the whole point: a fifth placement makes this
    /// match non-exhaustive, and the only index left to give it is one
    /// `ALL` does not have — so `ALL` has to grow too. Between them they
    /// are the mechanism that makes "every site that enumerates the
    /// placements" a compile-time obligation rather than a convention.
    pub const fn index(self) -> usize {
        match self {
            Placement::Client => 0,
            Placement::Static => 1,
            Placement::Server => 2,
            Placement::Durable => 3,
            Placement::Remembered => 4,
        }
    }

    /// The one English spelling, for diagnostics that name the placement
    /// a program wrote.
    pub fn word(self) -> &'static str {
        match self {
            Placement::Client => "client",
            Placement::Static => "static",
            Placement::Server => "server",
            Placement::Durable => "durable",
            Placement::Remembered => "remembered",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Init {
    /// `starting <expr>` — a source signal, mutable.
    Starting(Expr),
    /// `from <expr>` — a derived signal, recomputed, not directly mutable.
    From(Expr),
    /// `takes <params>` and an indented block — a one-shot, externally
    /// initiated effect (§14G.8 item 14).
    ///
    /// The construct §14G.7.6 called the one that blocks four designs:
    /// forms' submit, authentication, relational persistence, and write
    /// outcomes. `from` recomputes, so an effect cannot live there;
    /// `on click` is client context, so a server effect cannot live there
    /// either.
    ///
    /// **The cell is what makes it work.** §18.6 records three attempts to
    /// give a write an outcome by routing its failure into the
    /// corresponding read's `Failed` arm, all three dying on
    /// `examples/voting-board.zd`, where `votes` is written and never read
    /// so no cell and no arm exist. Declaring the effect in the `init` slot
    /// *creates* the cell the outcome lands in, which is why this shape
    /// generalises where the alternatives did not.
    ///
    /// Placement stays on the left-hand side of the declaration, so
    /// invariant 1 is untouched and functions stay colorless — the cost
    /// §14G.7.6 attaches to the rejected `action` construct.
    Effect {
        /// Typed because they cross a boundary, exactly as a `foreign`'s
        /// do. `trusted` on one is a demand on the caller (site A2).
        params: Vec<ForeignParam>,
        body: Block,
    },
    /// `every "250ms"`, `every frame`, `after "2s"` — a signal the clock
    /// writes (#19's timer and frame-loop half).
    ///
    /// **This is the construct that keeps a timer from being a callback.**
    /// A `setInterval` in a host language takes a function and runs it; a
    /// clause here takes nothing and runs nothing. What it declares is a
    /// *source* whose writer happens to be the browser's scheduler rather
    /// than a handler, and everything downstream is the `from` and the
    /// bindings the language already had. So the program still says what
    /// its state *is* at every instant, and there is no position anywhere
    /// in the grammar for "and then do this, later".
    Clock(Clock, Span),
    /// `every "90ms" starting <value> to <next>` — a clock that *folds*.
    ///
    /// The gap this closes is the one every simulation falls into. A
    /// plain clock signal reads elapsed milliseconds and nothing else, so
    /// a program can watch time pass and cannot advance anything with it:
    /// a board that steps, a queue that drains, a physics tick. Deriving
    /// the nth state from the elapsed time works only when the state is a
    /// closed-form function of `t`, and Conway is the standard example of
    /// one that is not.
    ///
    /// So the clause carries the fold the `from`/`fold` form already
    /// spells elsewhere: a resting value, and a step whose only new power
    /// is that it may **read the cell it writes**. That is a cycle in the
    /// dependency graph and it is a legal one here for the same reason a
    /// `fold`'s accumulator is legal — the read is of the *previous*
    /// value, taken before the write, and there is exactly one write per
    /// tick with nothing else able to observe the interval between.
    ///
    /// `after` is deliberately absent. A clock that fires once has one
    /// value after it fires, and a fold over one step is `starting`.
    Stepping {
        clock: Clock,
        start: Box<Expr>,
        step: Box<Expr>,
        span: Span,
    },
}

/// What drives a clock signal, and how often.
///
/// Exhaustive and small on purpose: each variant is one browser primitive
/// — `setInterval`, `requestAnimationFrame`, `setTimeout` — and a fourth
/// would have to be ruled on at every match rather than falling into a
/// wildcard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Clock {
    /// `every "250ms"` — a wall-clock interval, held here in milliseconds.
    Interval(f64),
    /// `every frame` — the display's refresh rate, whatever it is.
    ///
    /// Deliberately not spelled as a duration. `every "16ms"` on a 120 Hz
    /// display is a lie about what the machine will do, and a program that
    /// wants smooth motion wants the *frame*, not a number close to one.
    Frame,
    /// `after "2s"` — fires once, then never again.
    Delay(f64),
}

impl Clock {
    /// The type a signal driven by this clock must be declared with.
    ///
    /// Fixed rather than inferred, because the value is the compiler's and
    /// not the program's: nothing in the source produces it, so there is
    /// no expression whose type could be joined with the annotation.
    pub fn value_type(self) -> &'static str {
        match self {
            // Milliseconds since the signal started. `Decimal` and not
            // `Whole`: a frame timestamp has a fraction, and a cell typed
            // `Whole` holding `16.67` would be a lie the type system had
            // signed off on.
            Clock::Interval(_) | Clock::Frame => "Decimal",
            // It has happened, or it has not.
            Clock::Delay(_) => "Truth",
        }
    }

    /// What this clause means, in one phrase, for a diagnostic or a
    /// generated document.
    pub fn describe(self) -> String {
        match self {
            Clock::Interval(ms) => format!("the milliseconds elapsed, every {}", written_ms(ms)),
            Clock::Frame => "the milliseconds elapsed, once per animation frame".to_string(),
            Clock::Delay(ms) => format!("whether {} has passed", written_ms(ms)),
        }
    }

    /// The clause as a program would write it, for `zdc doc` and `zdc fmt`.
    pub fn written(self) -> String {
        match self {
            Clock::Interval(ms) => format!("every \"{}\"", written_ms(ms)),
            Clock::Frame => "every frame".to_string(),
            Clock::Delay(ms) => format!("after \"{}\"", written_ms(ms)),
        }
    }
}

/// A duration in milliseconds, written back in the shortest form that
/// round-trips through [`parse_duration`].
fn written_ms(ms: f64) -> String {
    let render = |value: f64, unit: &str| {
        if value.fract() == 0.0 {
            format!("{value:.0}{unit}")
        } else {
            format!("{value}{unit}")
        }
    };
    if ms >= 60_000.0 && (ms / 60_000.0).fract() == 0.0 {
        render(ms / 60_000.0, "m")
    } else if ms >= 1_000.0 && (ms / 1_000.0).fract() == 0.0 {
        render(ms / 1_000.0, "s")
    } else {
        render(ms, "ms")
    }
}

/// The longest interval a clock clause may name: one hour.
///
/// Not an arbitrary tidiness rule. `setInterval` takes a 32-bit signed
/// delay, and a browser silently fires *immediately* on anything past
/// `2^31 - 1` milliseconds — so a program asking for a day would get a
/// tight loop rather than a daily tick, which is the worst possible
/// failure mode for a construct whose whole job is "not very often". An
/// hour is comfortably inside the representable range and is already far
/// past what a browser tab stays open and unthrottled for; anything
/// genuinely periodic at that scale is §14G.4's scheduled state, which is
/// a `server` construct and is refused here by placement anyway.
pub const LONGEST_CLOCK_MS: f64 = 3_600_000.0;

/// The shortest interval a clock clause may name.
///
/// Four milliseconds is the floor browsers clamp nested timers to, so
/// anything smaller is a number the program does not get. A program that
/// wants "as often as possible" wants `every frame`, and the diagnostic
/// says so.
pub const SHORTEST_CLOCK_MS: f64 = 4.0;

/// Read `"250ms"`, `"1.5s"`, `"2m"` as a count of milliseconds.
///
/// The unit lives inside the literal rather than in the grammar, which is
/// the whole reason this construct costs one soft keyword instead of four:
/// `ms`, `s` and `m` never become words a program cannot use as a name.
///
/// Returns the reason on failure rather than a bare `None`, because every
/// caller is about to write a diagnostic and the reason is the useful half.
pub fn parse_duration(text: &str) -> Result<f64, DurationError> {
    let (digits, per_unit) = if let Some(rest) = text.strip_suffix("ms") {
        (rest, 1.0)
    } else if let Some(rest) = text.strip_suffix('s') {
        (rest, 1_000.0)
    } else if let Some(rest) = text.strip_suffix('m') {
        (rest, 60_000.0)
    } else {
        return Err(DurationError::NoUnit);
    };
    if digits.is_empty() {
        return Err(DurationError::NoNumber);
    }
    // Parsed by hand rather than with `f64::from_str`, which also accepts
    // `inf`, `NaN`, `+1`, `1e9` and `_`-free hex — none of which is a
    // duration, and all of which would arrive here as a plausible-looking
    // number of milliseconds.
    if !digits.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Err(DurationError::NoNumber);
    }
    let Ok(value) = digits.parse::<f64>() else {
        return Err(DurationError::NoNumber);
    };
    let ms = value * per_unit;
    if ms < SHORTEST_CLOCK_MS {
        return Err(DurationError::TooShort);
    }
    if ms > LONGEST_CLOCK_MS {
        return Err(DurationError::TooLong);
    }
    Ok(ms)
}

/// Why a duration literal was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationError {
    /// It does not end in `ms`, `s` or `m`.
    NoUnit,
    /// What precedes the unit is not a plain decimal number.
    NoNumber,
    /// Below [`SHORTEST_CLOCK_MS`], which a browser would clamp.
    TooShort,
    /// Above [`LONGEST_CLOCK_MS`], which a browser would overflow.
    TooLong,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StateDecl {
    pub secret: bool,
    /// `trusted state orders …` — spec §18.1.1.
    ///
    /// The integrity direction's one declaration-level grant on state
    /// (`G-SIG` clause 1, spec §21.7.3). It is the *obligation* marker too:
    /// declaring it is what makes every write to the place (A3) and every
    /// index into it (A1) a checked site.
    pub trusted: bool,
    pub name: Ident,
    pub placement: Placement,
    pub ty: TypeExpr,
    pub init: Init,
    /// §14C.3b's sub-requirement: where this value is **written** at build
    /// time, relative to the bundle root.
    ///
    /// `rss.xml` and `llms.txt` are generated *files*, not endpoints, and
    /// deriving them from the same state the pages are built from is what
    /// keeps them from drifting. Only a `static` signal may carry one,
    /// because only a `static` signal has a value at build time.
    pub emits: Option<Emitted>,
    pub span: Span,
}

/// A build-time output path, and where it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Emitted {
    pub path: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    Named(Ident),
    List(Box<TypeExpr>),
    Map(Box<TypeExpr>, Box<TypeExpr>),
    /// `Pair of K to V`: two values in one, written with the `to` a
    /// `Map of K to V` already spends between two type operands.
    ///
    /// The type §17.7 said was missing when it recorded that `bothOf` had
    /// no return type to give. A `record` in the library would have been
    /// the other answer and cannot be: a `record` declares concrete field
    /// types, so `zip` over two lists of anything is not a record anybody
    /// can write down.
    Pair(Box<TypeExpr>, Box<TypeExpr>),
    Option(Box<TypeExpr>),
    Remote(Box<TypeExpr>),
}

// --- functions and statements ---

/// How a callable's arguments are written, and therefore how every call to
/// it must be written.
///
/// §17.4.2: a function is called in exactly one form, and the declaration
/// decides which. `length with posts` where `length` was declared
/// `function length of value` is an error naming the one valid form, and
/// vice versa — which is what keeps §4.1's one-phrasing rule while giving
/// unary accessors the `of` spelling §14F.1 asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallForm {
    /// `f with a, b` — any number of parameters.
    With,
    /// `length of items` — exactly one, a unary accessor.
    Of,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDecl {
    pub name: Ident,
    pub form: CallForm,
    pub params: Vec<Ident>,
    pub body: Block,
    pub span: Span,
}

/// Where a `foreign` may run (§14E.2).
///
/// **This answers one question and only one: which output bundles may this
/// library be linked into.** It is not a purity classification, it never
/// was, and reading it as one is residual risk R1 — see [`ForeignResult`],
/// which is the classification built for the other question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignSite {
    Client,
    Server,
    Anywhere,
}

/// What a `foreign` declares about **its result**, on the `gives` line
/// (§21.9).
///
/// Two questions were spelled with one word until §21.8. [`ForeignSite`]
/// answers *where may this be linked*; this answers *is the result a
/// function of the arguments*. They are independent — a query-string
/// reader is honestly `is anywhere` and is not pure, and a password hash
/// is honestly `is server` and is — so they get separate declarations.
///
/// **The default is [`ForeignGrant::Opaque`]**, and the default is the
/// design: an unmarked `foreign` is never mistaken for pure. The failure
/// mode of the other default is a silent leak, which is the same reason
/// `Authority` defaults to `Untrusted`.
///
/// Deliberately an enum rather than two `bool`s: `gives pure trusted T` is
/// not a state the type can hold, so no consumer has to decide what it
/// would mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ForeignGrant {
    /// `gives T` — no claim. The result is whatever the JavaScript did,
    /// which for all the compiler knows is the wall clock or the request
    /// URL.
    #[default]
    Opaque,
    /// `gives pure T` — grant `G-FGN-P`: the result is a function of the
    /// arguments, so its integrity is their join.
    ///
    /// Asserted by a human and checked by nobody. §14E.4's dev-mode check
    /// validates the shape of a return value and cannot detect impurity.
    /// What changed in §21.9 is not that the claim became checkable — it is
    /// that it became *declared* rather than inferred from an unrelated
    /// property.
    Pure,
    /// `gives trusted T` — grant `G-FGN-T`: the result is Trusted whatever
    /// the arguments were. Strictly stronger than [`ForeignGrant::Pure`],
    /// and strictly more of a human's word.
    Trusted,
}

impl ForeignGrant {
    /// The one valid spelling of the modifier, or `None` where there is no
    /// modifier to spell.
    pub fn describe(self) -> Option<&'static str> {
        match self {
            ForeignGrant::Opaque => None,
            ForeignGrant::Pure => Some("pure"),
            ForeignGrant::Trusted => Some("trusted"),
        }
    }
}

impl ForeignSite {
    pub fn describe(self) -> &'static str {
        match self {
            ForeignSite::Client => "client",
            ForeignSite::Server => "server",
            ForeignSite::Anywhere => "anywhere",
        }
    }
}

/// One parameter of a `foreign`: a name and the type it asserts.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignParam {
    pub name: Ident,
    /// `takes key is trusted Text` — a **requirement on the caller**,
    /// discharged at obligation site A2 (spec §18.1 semantics 8). The same
    /// word on a `release` clause is a *grant*; §19.10.2 records why the
    /// two live in different syntactic slots.
    pub trusted: bool,
    pub ty: TypeExpr,
    pub span: Span,
}

/// Whether `name` is a bare JavaScript identifier, conservatively.
///
/// This is the *only* implementation of the rule, and it lives beside
/// [`ForeignDecl`] rather than inside any one pass because more than one
/// of them needs the same answer: the parser refuses the literal, and
/// `zdc-codegen` refuses again at the point of emission. Two copies of a
/// security rule is one copy that can be relaxed without the other
/// noticing.
///
/// ASCII only. `IdentifierName` is far wider than this, and narrowing it
/// costs a program nothing it can act on — an export whose name is not
/// ASCII is vanishingly rare and the diagnostic says exactly what to
/// write — while widening it would put this check in the business of
/// tracking two Unicode tables it could get wrong.
pub fn is_javascript_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_' || first == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// The name a `foreign` imports from its module — the `as` operand of a
/// `from` clause (spec §14E.1).
///
/// Written as a text literal, but it is not text: it reaches the generated
/// `import { … } from …` clause as **syntax**, so there is no escape that
/// makes an arbitrary string safe there. `as "m } from 'evil'; //"` closes
/// the clause and opens another, and every character after it is
/// JavaScript the program's author chose.
///
/// The field is private and [`ExportName::parse`] is the only constructor,
/// so a `ForeignDecl` carrying an export that is not an identifier does
/// not exist to be lowered or emitted. That is what this type buys over a
/// `String` some pass remembers to check: a `String` field is only ever as
/// safe as the last pass that looked at it, and a pass can grow a path
/// around its own check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportName(String);

impl ExportName {
    /// `name` as an export name, or `None` if it is not an identifier.
    pub fn parse(name: &str) -> Option<ExportName> {
        is_javascript_identifier(name).then(|| ExportName(name.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ExportName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a `foreign` hands back (spec §14E.1).
///
/// `gives view` is the DOM-owning form: the foreign is given a node it
/// owns and returns **no ZDeceptron value at all**. Reusing the existing
/// `view` keyword is what makes the form cost zero reserved words, and
/// giving back nothing is what keeps §19.2 rule 12's laundering question
/// from arising for it — there is no result to launder.
///
/// `Value` is the ordinary value-returning form the prelude is written
/// with (§17.4.10). The two are one enum rather than two declaration
/// forms because §4.1 admits exactly one phrasing per construct: a reader
/// asking "what does this foreign hand back?" reads one clause.
/// Named apart from [`ForeignGrant`] because the two answer different
/// questions about the same clause: this one is *what* comes back, and
/// [`ForeignGrant`] is *what is claimed* about it. `gives pure view`
/// therefore parses and is inert rather than refused — a view hands back
/// no value, so there is nothing for a grant to be about, which is the
/// same reason the laundering question does not arise for it.
#[derive(Debug, Clone, PartialEq)]
pub enum ForeignResult {
    /// `gives view` — the foreign owns a DOM node.
    View,
    /// `gives Text` — an ordinary value-returning foreign.
    Value(TypeExpr),
    /// `gives new Handle` — the export is a class, and the call
    /// **constructs** rather than invokes.
    ///
    /// The third form of the one `gives` clause, for the same reason
    /// [`ForeignResult::View`] is the second: what a foreign hands back is
    /// one question, a reader answers it by reading one line, and §4.1
    /// admits one phrasing. `new` is a *soft* keyword, so it stays an
    /// ordinary identifier everywhere else — `function replace with value,
    /// old, new` in the prelude still parses — and it costs nothing
    /// against §14G.7.7's reserved-word budget.
    ///
    /// The type is carried rather than assumed so that the refusal of
    /// `gives new Text` has a span to point at. Only [`HANDLE_TYPE_NAME`]
    /// is admitted, and the check is the type checker's: `new` on a class
    /// yields a host object, and the language's word for one is `Handle`.
    New(TypeExpr),
    /// `gives nothing` — the foreign is called for its **effect**, and no
    /// ZDeceptron value comes back from it.
    ///
    /// The fourth form of the one `gives` clause, and the one that makes
    /// `scene.add(mesh)` writable. Much of what a host library's API is
    /// made of is called for what it does: `renderer.render(scene,
    /// camera)`, `node.appendChild(child)`, `controls.update()`. Declaring
    /// one of those `gives Whole` compiles, emits, and hands the program
    /// `undefined` wearing a number's type — the silent acceptance §4.1
    /// refuses, and one no later pass can catch, because the assertion is
    /// the program's own.
    ///
    /// **The claim is about this program, not about JavaScript.** Plenty of
    /// the calls written this way do return something — `add` returns the
    /// object for chaining, `appendChild` returns the child — and
    /// `gives nothing` is still the truth about the declaration: nothing
    /// comes *back into the language*. That is the same claim
    /// [`ForeignResult::View`] makes, in the same words §14E.1 uses for it,
    /// and it is why neither carries a grant: there is no result for one to
    /// be about.
    ///
    /// Deciding it at the declaration rather than at each call is the
    /// point. A `do` that discarded whatever a call happened to return
    /// would put the decision at every call site, where a reader has to
    /// notice it; written here it is one line, read once, and the checker
    /// holds every call to it to the same answer.
    ///
    /// A call to one has type `Nothing`, which unifies with nothing at all,
    /// so the only place it can be written is a [`Stmt::Do`]. That is what
    /// makes "nothing comes back" a checked claim rather than a comment.
    Nothing,
}

/// The written name of the opaque host-object type.
///
/// It lives here, one level below the type checker, because three passes
/// before the checker have to recognise the word: the parser refuses
/// `gives new` on anything else, name resolution refuses a `foreign`
/// touching one that is not `is client`, and the split refuses one written
/// anywhere it could cross a boundary. `zdc_types::Type::from_name` reads
/// this same constant, so the spelling exists once and the passes cannot
/// disagree about it.
pub const HANDLE_TYPE_NAME: &str = "Handle";

/// The written name of the text type.
///
/// Here for the reason [`HANDLE_TYPE_NAME`] is: name resolution has to
/// recognise it before the checker exists, because a `request`'s `gives`
/// line admits this word and no other and the refusal of anything else is
/// written where the declaration is lowered.
pub const TEXT_TYPE_NAME: &str = "Text";

impl TypeExpr {
    /// Whether [`HANDLE_TYPE_NAME`] appears anywhere in this written type.
    ///
    /// Written over the *syntax* rather than over a checked type because
    /// every caller runs before the checker does, and because the question
    /// is about what the program wrote: `Remote of Handle` is refused for
    /// naming a handle inside a wire type, whether or not anything would
    /// ever have produced one.
    pub fn mentions_handle(&self) -> bool {
        match self {
            TypeExpr::Named(name) => name.text == HANDLE_TYPE_NAME,
            TypeExpr::List(inner) | TypeExpr::Option(inner) | TypeExpr::Remote(inner) => {
                inner.mentions_handle()
            }
            TypeExpr::Map(key, value) | TypeExpr::Pair(key, value) => {
                key.mentions_handle() || value.mentions_handle()
            }
        }
    }

    /// Whether this written type is exactly `Handle`, with nothing around
    /// it. The one position a handle is admitted in.
    pub fn is_bare_handle(&self) -> bool {
        matches!(self, TypeExpr::Named(name) if name.text == HANDLE_TYPE_NAME)
    }
}

/// `request weather is client` — an outbound HTTP request (#19).
///
/// The declaration *is* a signal: it carries no call syntax, and reading
/// its name anywhere on the client yields `Remote of Text`. That is the
/// whole of the design's shape argument. §5's three-armed `when` already
/// models "not here yet, may have failed", the browser is already the
/// thing that waits, and a request that recomputes when its arguments
/// change is what a reactive dataflow language calls a derived signal.
///
/// **The destination is written down.** [`RequestDecl::destination`] is a
/// `Text` literal and the parser admits nothing else in that position, so
/// the host a program talks to is decided by reading the program. A
/// computed destination could not be checked by the flow pass, could not
/// be named in the emitted `connect-src`, and is the shape of
/// `fetch("https://x/?k=" + apiKey)` — a leak with no body at all.
///
/// **The arguments are the query string**, which is why they are the flow
/// site. `with topic is subject` becomes `?topic=…` on the destination, so
/// an argument *is* part of the URL and reaches §14G.1.3(c)'s sink 7.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestDecl {
    pub name: Ident,
    /// Where the request runs. `client` is the only placement this admits;
    /// see `zdc_resolve` for the refusal and its reason.
    pub placement: Placement,
    /// Where the placement word was written, so a refusal points at it
    /// rather than at the whole declaration.
    pub placement_span: Span,
    /// The destination, exactly as written. A literal, never an
    /// expression — see the type's own documentation.
    pub destination: String,
    pub destination_span: Span,
    /// `with topic is subject` — the query parameters, in source order.
    ///
    /// Every one is [`Arg::Named`]: a query parameter has a name in the
    /// URL, so there is nothing for a positional argument to be called.
    pub args: Vec<Arg>,
    /// The `gives` line. `Text` is the only type admitted, and the clause
    /// exists so that the refusal of anything else — `gives Markup` above
    /// all — has a span to point at.
    pub gives: TypeExpr,
    pub gives_span: Span,
    pub span: Span,
}

/// `foreign textLength is anywhere` — spec §14E.1, as amended by §17.4.2.
///
/// The types are *asserted*, not inferred: there is no body to infer them
/// from. §17.4.10 lists the seventeen operations that need one, and every
/// `foreign` outside that list is the program's own claim about a platform
/// function.
///
/// One declaration form covers both the value-returning FFI and the
/// DOM-owning one; they differ only in the `gives` clause. Two spellings
/// of `foreign` would be the §4.1 violation this language was designed
/// against.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignDecl {
    pub name: Ident,
    pub site: ForeignSite,
    /// Where the site word was written, so a refusal points at it rather
    /// than at the whole declaration.
    pub site_span: Span,
    /// Where the symbol comes from: a module, or the call's first
    /// argument.
    pub source: ForeignSource,
    /// The symbol itself — an export name under [`ForeignSource::Import`],
    /// a method name under [`ForeignSource::Receiver`]. Validated at parse
    /// time, and the type is what carries that refusal across every later
    /// pass: both positions reach the emitted JavaScript as *syntax*, one
    /// inside an `import` clause and one after a dot.
    pub export: ExportName,
    pub export_span: Span,
    pub form: CallForm,
    pub params: Vec<ForeignParam>,
    /// What the `gives` line claims about the result — `gives T`,
    /// `gives pure T` or `gives trusted T` (spec §21.9).
    ///
    /// Orthogonal to [`ForeignDecl::result`], which answers *what* is handed
    /// back rather than *what is claimed about it*. `gives view` hands back
    /// nothing, so it carries no grant and this stays
    /// [`ForeignGrant::Opaque`] for it.
    pub result_grant: ForeignGrant,
    pub result: ForeignResult,
    pub result_span: Span,
    pub span: Span,
}

impl ForeignDecl {
    /// Whether this foreign owns a DOM node rather than returning a value.
    pub fn owns_view(&self) -> bool {
        matches!(self.result, ForeignResult::View)
    }

    /// Whether a call to this foreign constructs — `new Export(…)`.
    pub fn constructs(&self) -> bool {
        matches!(self.result, ForeignResult::New(_))
    }

    /// Whether a call to this foreign is a method call on its first
    /// argument — `receiver.Export(…)`.
    pub fn is_method(&self) -> bool {
        matches!(self.source, ForeignSource::Receiver { .. })
    }

    /// Whether a call to this foreign is a property read off its first
    /// argument — `receiver.Export`, with no call at all.
    ///
    /// A *write* is not one. The two share a member name and share nothing
    /// else: a read is an expression of the property's type and takes only
    /// the receiver, a write is a statement and takes the value as well.
    /// Folding them into one predicate would make every site that asks
    /// "is this a property?" have to ask a second question to know what it
    /// had.
    pub fn is_property(&self) -> bool {
        matches!(self.source, ForeignSource::Property { .. })
    }

    /// Whether a call to this foreign writes a property of its first
    /// argument — `receiver.Export = value`.
    pub fn writes_property(&self) -> bool {
        matches!(self.source, ForeignSource::Write { .. })
    }

    /// Whether the symbol is looked up on the call's first argument rather
    /// than imported — a method, a property read or a property write.
    ///
    /// The three share one set of rules about the receiver, and this is
    /// what lets that set be written once.
    pub fn has_receiver(&self) -> bool {
        !matches!(self.source, ForeignSource::Import { .. })
    }

    /// Where the source line was written, for a refusal that wants to
    /// point at it rather than at the whole declaration.
    pub fn source_span(&self) -> Span {
        match &self.source {
            ForeignSource::Import { module_span, .. } => *module_span,
            ForeignSource::Receiver { span }
            | ForeignSource::Property { span }
            | ForeignSource::Write { span } => *span,
        }
    }

    /// The module this is imported from, or `None` for a method or a
    /// property, none of which imports anything.
    pub fn module(&self) -> Option<&str> {
        match &self.source {
            ForeignSource::Import { module, .. } => Some(module),
            ForeignSource::Receiver { .. }
            | ForeignSource::Property { .. }
            | ForeignSource::Write { .. } => None,
        }
    }
}

/// Where a `foreign`'s symbol is found (spec §14E.1, as this branch
/// amends it).
///
/// The four answers to "where does this name live" are alternatives, they
/// occupy the same line of the declaration, and each is followed by the
/// same `as` clause naming the symbol. So they are one enum and one
/// production rather than four, which is what §4.1 asks of a construct
/// with one phrasing.
///
/// Three of the four look the symbol up on the call's first argument and
/// differ only in what is then done with it — called, read, or written.
/// That is the whole of what a host object's interface is, and each of the
/// three is a word the language already reserves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForeignSource {
    /// `from "three" as "Scene"` — a module export. The bundle imports it,
    /// and a `zd:` prefix names the language's own primitive layer
    /// (§17.4.10) rather than a package.
    Import { module: String, module_span: Span },
    /// `on Handle as "add"` — a **method**, looked up on the call's first
    /// argument.
    ///
    /// **Nothing is imported, and there is nothing to import.** A method
    /// comes with the object: `scene.add(mesh)` names no module, and a
    /// declaration that spelled one would put a class into the bundle for
    /// the sake of a name that is resolved at run time anyway.
    ///
    /// This costs no reserved word. `on` is already a keyword — `on click`
    /// — and `as` is already the soft keyword that names a symbol on the
    /// line this replaces. The `Handle` after `on` is the receiver's type
    /// written out, which is the only type a receiver may have and is what
    /// makes the line say what it does.
    Receiver { span: Span },
    /// `of Handle as "domElement"` — a **property**, read off the call's
    /// first argument and not called.
    ///
    /// The minimal pair with [`ForeignSource::Receiver`], and the pair is
    /// the whole design: `on` a handle is something you *do* to it, `of` a
    /// handle is something it *has*. `renderer.domElement` is a canvas the
    /// renderer already owns, and writing it `on Handle as "domElement"`
    /// would emit `renderer.domElement()` and call a canvas.
    ///
    /// **It costs no reserved word either.** `of` is already a hard
    /// keyword — `List of Text`, `length of items` — and no module
    /// specifier, no `on` and no `of` can begin one of the others, so the
    /// source line stays LL(1) on one token.
    ///
    /// A property takes exactly one parameter, because a property read has
    /// no arguments: there is nowhere for a second one to go. Name
    /// resolution refuses a second, so no declaration with one reaches an
    /// emission.
    Property { span: Span },
    /// `set Handle as "roughness"` — a **property write**, assigned on the
    /// call's first argument.
    ///
    /// The third member of the pair [`ForeignSource::Receiver`] and
    /// [`ForeignSource::Property`] make: `on` a handle is something you do
    /// to it, `of` a handle is something it has, and `set` a handle is
    /// something it has being given a value. `material.roughness = 0.9` is
    /// a whole third of what driving a host library consists of, and
    /// before this it was unwritable: a library that exposes a setting as
    /// a field rather than as a `setX` method could not be told anything.
    ///
    /// **The word is not new and it is not arbitrary.** `set` is already a
    /// hard keyword and it is already the language's one verb for writing
    /// a value into a place — `set depth to 4` is §8's mutation statement.
    /// A property of a host object is a place, so it is written with the
    /// word every other place is written with; a fourth spelling would be
    /// §4.1's second phrasing for one construct.
    ///
    /// Two parameters exactly, and `gives nothing` exactly:
    ///
    /// * the receiver, and the value — an assignment has one left side and
    ///   one right side, so a declaration naming one parameter has nothing
    ///   to write and one naming three describes an emission that does not
    ///   exist;
    /// * no result, because `x.p = v` *evaluates* to `v` in JavaScript and
    ///   taking that back would be a second way to say a value the program
    ///   already has. `gives nothing` is the same claim `gives view`
    ///   makes: about this program, not about JavaScript.
    ///
    /// Nothing about information flow is special-cased for it. A write is
    /// a call whose arguments are the receiver and the value, both walked
    /// by the same rule every other foreign call's arguments are, and
    /// `is client` obliges each of them Public (§14E.3 row 1) — so a
    /// `secret` cannot be put into a host object through a field any more
    /// than through a method.
    Write { span: Span },
}

// --- declassification (spec §19.1, §19.10.2) ---

/// `limit 10 per visitor` — the per-evaluation budget clause.
///
/// **This bounds nothing cumulatively.** It counts evaluations of *one*
/// declaration against *one* anonymous session: `k` declarations give `kN`,
/// clearing a cookie mints a fresh budget, and until `DurableStore` exists
/// it is not enforced at all. Spec §21.8.7 and residual risk R3.
#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseLimit {
    pub count: u32,
    pub span: Span,
}

/// `release judge with guess, answer` — spec §19.1 as amended by §19.10.2.
///
/// ```text
/// releaseDecl := "release" IDENT ["with" params] NEWLINE INDENT
///                  "gives" type NEWLINE
///                  { "trusted" IDENT NEWLINE }
///                  [ "limit" NUMBER "per" "visitor" NEWLINE ]
///                  stmt+ DEDENT
/// ```
///
/// Clause order is fixed — `gives`, then endorsements, then `limit`, then
/// statements — so `releaseDecl` stays LL(1) and the parser never
/// backtracks.
#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseDecl {
    pub name: Ident,
    pub params: Vec<Ident>,
    /// The declared bandwidth per evaluation (spec §19.2 rule 4).
    pub gives: TypeExpr,
    /// The parameters named by a `trusted` clause — `endorsed(f)` in
    /// REL-ARG. Site-local and result-transparent: an endorsement discharges
    /// REL-ARG at this release's call sites and does nothing anywhere else
    /// (spec §19.10.3(a)).
    pub endorsed: Vec<Ident>,
    pub limit: Option<ReleaseLimit>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Pipeline(PipelineClause),
    Mutation(Mutation),
    Give(Expr),
    When(WhenStmt),
    Each(EachStmt),
    If(IfStmt),
    /// `with total is 0` — a local binding (spec §17.4.10).
    Bind(BindStmt),
    /// `do render with r is gl, scene is world` — run a call for its
    /// effect (§14E.1, as this branch amends it).
    ///
    /// **The statement the language was missing, and the shape of the gap
    /// is what picks the spelling.** Every other statement form consumes a
    /// value: `give` returns one, `set`/`add`/`append` put one somewhere, a
    /// pipeline accumulates one, `with` names one. A call to a
    /// [`ForeignResult::Nothing`] foreign produces none, so before this
    /// there was no position in the grammar it could be written in at all.
    ///
    /// `do` is a soft keyword and costs nothing against §14G.7.7's budget:
    /// no statement in this language may begin with an identifier, so the
    /// leading word is either a statement keyword or a parse error, and the
    /// decision point stays LL(1) on one token. A program may still name a
    /// field, a parameter or a signal `do`.
    ///
    /// **It discards nothing, which is the point.** The type checker
    /// admits exactly one type here — `Nothing` — so `do` cannot be used
    /// to throw away a result the program should have used. A `foreign`
    /// whose result the program does not want says so once, on its own
    /// `gives` line, where a reader meets it before any call.
    Do(DoStmt),
}

/// `do <call>` — one call, run for its effect (spec §14E.1).
///
/// The expression is held whole rather than split into a callee and
/// arguments, so that every pass reaches the call through the same
/// expression walk it uses everywhere else. A `do` that named a callee
/// directly would be a second call site the information-flow walk had to
/// know about, and the one it does not know about is the one that leaks.
#[derive(Debug, Clone, PartialEq)]
pub struct DoStmt {
    pub call: Expr,
    pub span: Span,
}

/// One `name is value` pair of a binding statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub name: Ident,
    pub value: Expr,
    pub span: Span,
}

/// `with total is 0, index is 1` — spec §17.4.10's local binding.
///
/// The construct §17.4.10 asks for, spelled with the word the language
/// already uses for it. `with` binds names to values everywhere else it
/// appears — `function f with a, b`, `f with a is 1`, `Photo with album is
/// slug`, `Archived with why` — and a local binding is that same act
/// applied to the rest of the block. Reusing it is the reuse §14G.7.7
/// licenses for `in`, and it costs no reserved word: `with` cannot begin a
/// statement in any other production, so the grammar stays LL(1) at the
/// decision point and §14G.7.7's budget is untouched.
#[derive(Debug, Clone, PartialEq)]
pub struct BindStmt {
    pub bindings: Vec<Binding>,
    pub span: Span,
}

/// One clause of a pipeline, applied to the sequence the clauses before it
/// produced.
///
/// **`Sort` is stable, and that is a decision rather than an accident.**
/// Two elements whose keys compare equal come out in the order they went
/// in, so `sort each row by row.name` followed by `sort each row by
/// row.rank` leaves the name order intact inside each rank. That is the
/// behaviour a table with two clickable headings needs, and the behaviour
/// it is wrong without, which is why the guarantee is stated rather than
/// left to whatever the emitter happens to do.
///
/// It was previously true by inheritance: the emitted form is
/// `Array.prototype.sort`, stable by specification since ES2019, so it
/// would have been stable whether or not anybody had chosen it. What the
/// decision adds is that the emitter may not stop being stable quietly.
/// `zdc-codegen/src/stmt.rs` emits a three-way comparator whose last arm
/// is `0`, so keys that are neither less nor greater are reported equal
/// and the elements holding them are left where they were; `zdc-codegen`'s
/// `tests/emission.rs` pins that comparator and the order a two-pass sort
/// produces when the bundle is actually run.
///
/// **`Fold` ends the pipeline, and nothing may follow it.** Every other
/// clause takes a sequence and gives a sequence; this one takes a sequence
/// and gives one value, so `keep`, `sort`, `map each` and `take first`
/// have nothing left to walk. The type checker says so by name rather than
/// letting the emitter call `.filter` on a number.
#[derive(Debug, Clone, PartialEq)]
pub enum PipelineClause {
    From(Expr),
    Keep {
        var: Ident,
        cond: Expr,
    },
    Sort {
        var: Ident,
        key: Expr,
    },
    MapEach {
        var: Ident,
        to: Expr,
    },
    /// `fold each n into total starting 0 to total + n` (#33).
    ///
    /// Two binders and two expressions: `starting` is evaluated once, in
    /// the scope *outside* the clause, and `step` is evaluated once per
    /// element with both names in scope. The step's type is the seed's
    /// type — a fold does not change what it is accumulating — which is
    /// what lets the clause be checked without any notion of a function
    /// type, and it is why this is a clause rather than a call taking a
    /// function: there is no function value here, only a binder and an
    /// expression, exactly as `map each` has always had.
    Fold {
        item: Ident,
        total: Ident,
        starting: Expr,
        step: Expr,
    },
    TakeFirst(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mutation {
    Set {
        place: Place,
        value: Expr,
    },
    /// Numbers only (spec §14B.2).
    Add {
        value: Expr,
        place: Place,
    },
    /// Numbers only (spec §14B.2).
    Subtract {
        value: Expr,
        place: Place,
    },
    /// Collections only (spec §14B.2).
    Append {
        value: Expr,
        place: Place,
    },
    /// Collections only (spec §14B.2).
    Remove {
        value: Expr,
        place: Place,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub base: Ident,
    pub path: Vec<PathSeg>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PathSeg {
    Field(Ident),
    Index(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhenStmt {
    pub scrutinee: Expr,
    pub arms: Vec<Arm>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arm {
    pub pattern: Pattern,
    pub body: ArmBody,
    pub span: Span,
}

/// A `when` arm's pattern: a variant name and the names it binds.
///
/// A variant declares *named fields* (`Archived with reason is Text`), and
/// a pattern binds a fresh name to each of them positionally
/// (`Archived with why, moment`). A pattern may therefore bind several
/// names, so this is a list rather than a single optional binder — the
/// grammar is `pattern := IDENT ["with" IDENT ("," IDENT)*]` (spec
/// §14G.1.2). A payload-free variant such as `Loading` binds none, and
/// the list is empty.
#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub name: Ident,
    pub bindings: Vec<Ident>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArmBody {
    Show(Expr),
    Block(Block),
}

#[derive(Debug, Clone, PartialEq)]
pub struct EachStmt {
    pub var: Ident,
    pub iter: Expr,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfStmt {
    pub cond: Expr,
    pub then: Block,
    pub otherwise: Option<Block>,
    pub span: Span,
}

// --- view ---

#[derive(Debug, Clone, PartialEq)]
pub struct ViewDecl {
    /// The document's metadata: `view title is "…", description is "…"`.
    /// Named arguments, exactly as an element's are.
    pub args: Vec<Arg>,
    pub nodes: Vec<Node>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Element(Element),
    Each(EachNode),
    When(WhenNode),
    Handler(Handler),
    /// `if open` with an indented body, and an optional `otherwise`.
    ///
    /// §4.4 gave `if` to statements only, and §14D.1's own `Disclosure`
    /// writes one in node position. The view needs it for the same reason
    /// a block does: showing a node conditionally is not the same question
    /// as matching a variant, and spelling it `when` would need a `choice`
    /// nobody declared.
    If(IfNode),
    /// `children` — the nodes nested under this component at its call site.
    Children(Span),
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfNode {
    pub cond: Expr,
    pub then: Vec<Node>,
    pub otherwise: Option<Vec<Node>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub name: Ident,
    pub args: Vec<Arg>,
    pub children: Vec<Node>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Positional(Expr),
    Named { name: Ident, value: Expr },
}

#[derive(Debug, Clone, PartialEq)]
pub struct EachNode {
    pub var: Ident,
    pub iter: Expr,
    pub body: Vec<Node>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhenNode {
    pub scrutinee: Expr,
    pub arms: Vec<NodeArm>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeArm {
    pub pattern: Pattern,
    pub body: NodeArmBody,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeArmBody {
    Show(Element),
    Nodes(Vec<Node>),
}

/// What raises the event a handler runs on.
///
/// Two things wearing one word, and they are separated here rather than
/// distinguished by inspecting `event` later, because the difference is
/// **what the handler can observe** and not which node it is written under.
/// An element handler sees only what its own element was sent. A document
/// handler sees a keystroke aimed at anything on the page, which is a
/// strictly larger capability and is what [`HandlerTarget::Document`]'s
/// shape is designed around.
#[derive(Debug, Clone, PartialEq)]
pub enum HandlerTarget {
    /// The element this handler is nested under.
    Element,
    /// The document, for one named key — `on key "Escape"` (§16.3.7a).
    ///
    /// **The key is a literal and there is no binder.** Both halves are the
    /// design and neither is an omission; see `zdc_types::DOCUMENT_KEY_RULE`
    /// and the `E0364` explanation.
    Document { key: String, key_span: Span },
}

/// `on click` — a listener on the element it is nested in — or
/// `on key "Escape"`, a listener on the document.
///
/// `payload` is the optional binder of `on click with press`: the event the
/// browser raised, as a value. It reuses the `with`-introduces-binders
/// phrasing `function f with a`, `component C with label` and
/// `Archived with reason` already have, so it costs no reserved word
/// (§4.1, §14G.7.7). Omitting it is the whole of the old form, which is
/// why every existing program is unaffected.
///
/// A [`HandlerTarget::Document`] handler always has `payload: None`, and
/// the parser is where that is enforced — the production has no `with` in
/// it, so the observation is not expressible rather than checked for.
#[derive(Debug, Clone, PartialEq)]
pub struct Handler {
    pub event: Ident,
    pub target: HandlerTarget,
    pub payload: Option<Ident>,
    pub body: Block,
    pub span: Span,
}

// --- expressions ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Or,
    And,
    Is,
    IsNot,
    /// `body contains query` — §14F.1's one addition to the closed infix
    /// set. Which of `textContains`, `listContains` and `mapContains` it
    /// means is chosen by the head constructor of the left operand
    /// (§17.4.3), which only the type checker knows.
    Contains,
    Less,
    Greater,
    LessEq,
    GreaterEq,
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// `value if condition otherwise other` — a conditional *expression*.
    ///
    /// # Why the language needed one
    ///
    /// `if` is a statement, so until now a conditional *value* had
    /// nowhere to live: picking between two of them meant declaring a
    /// named function whose whole body was the choice. A real port
    /// accumulated a dozen of those — `oneUnless`, `detailAfter`,
    /// `kindLabel`, `shadeFactor` — each taking a name in a flat module
    /// namespace, each separating the question from where it is asked,
    /// and none of them saying anything a reader wanted to know.
    ///
    /// # Why it reads value-first
    ///
    /// `ALTERNATE if alternating otherwise 1.0` puts the answer where the
    /// eye already is: the expression is being read for its *value*, and
    /// the condition is the qualification on it. Leading with `if` would
    /// also have made the first token of an expression the first token of
    /// a statement, and this way there is no position where the two
    /// forms compete — a statement `if` opens a line, and this one never
    /// can.
    ///
    /// It is the lowest-precedence form there is and right-associative,
    /// so `a if p otherwise b if q otherwise c` chains the way a reader
    /// expects and needs no brackets to say so.
    Conditional {
        value: Box<Expr>,
        condition: Box<Expr>,
        otherwise: Box<Expr>,
        span: Span,
    },
    Number {
        value: f64,
        span: Span,
    },
    Text {
        value: String,
        span: Span,
    },
    Truth {
        value: bool,
        span: Span,
    },
    Empty {
        span: Span,
    },
    /// `["red", "green"]` — spec §14B.4. `[]` is the empty list; the empty
    /// map has no bracket form, because `[]` cannot be both.
    List {
        items: Vec<Expr>,
        span: Span,
    },
    /// `["a" to 1, "b" to 2]` — spec §14B.4, reusing the `to` of
    /// `Map of K to V` so one word means one thing in type and value
    /// position alike.
    Map {
        entries: Vec<(Expr, Expr)>,
        span: Span,
    },
    Var {
        name: Ident,
        span: Span,
    },
    Call {
        name: Ident,
        args: Vec<Arg>,
        span: Span,
    },
    /// `length of posts` — §14F.1's `of` prefix for unary accessors, and
    /// §17.4.2's `ofExpr`. Right-associative, so `text of day of moment`
    /// is `text of (day of moment)`.
    Of {
        name: Ident,
        operand: Box<Expr>,
        span: Span,
    },
    Environment {
        key: String,
        span: Span,
    },
    /// `address` — the URL this document was served at, as a value of the
    /// program's `route` type wrapped in `Option` (spec §14G.2).
    ///
    /// A signal initialised from it is immutable: the browser writes it at
    /// load and the program never does, which is what makes per-URL
    /// constant folding ordinary constant propagation rather than a new
    /// evaluation mode (§14G.2 revision 1).
    Address {
        span: Span,
    },
    /// `media "(prefers-color-scheme: dark)"` — whether the browser
    /// matches a CSS media query, as a `Truth` that changes when the
    /// browser's answer does.
    ///
    /// The query keeps its written spelling here and is never computed.
    /// `matchMedia` subscribes for the life of the page, so a query built
    /// from a value would have to re-subscribe, and the language has no
    /// moment at which that would happen.
    /// `scroll` — how far the reader has scrolled the document, as a
    /// `Decimal` from 0 to 100.
    ///
    /// A *quantity*, not an event, which is the distinction §10 draws when
    /// it says `resize`, `scroll` and `pointermove` "have no form at all".
    /// `on scroll` would be a handler running sixty times a second, which
    /// is the callback shape this language exists without; a scroll
    /// position is a cell the browser writes, which is what `every frame`
    /// already is. So it joins that family rather than the event set, and
    /// carries the clock's four rules: `client` only, nothing may write it,
    /// it is Untrusted, and it is disposed with its view.
    Scroll {
        span: Span,
    },
    Media {
        query: String,
        span: Span,
    },
    /// `build read "content/hello.md"` — a compiler-provided capability.
    ///
    /// The capability keeps its written spelling here: whether it names
    /// one of the closed set is a resolution question, so the parser does
    /// not answer it and a misspelling gets a diagnostic that can list the
    /// alternatives.
    Build {
        capability: Ident,
        argument: Box<Expr>,
        span: Span,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
        span: Span,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Field {
        base: Box<Expr>,
        name: Ident,
        span: Span,
    },
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    /// `append piece to pieces` — the list construction form.
    ///
    /// The same three words §14B.2 already spends on the mutation, in the
    /// one position the mutation cannot occupy. A mutation names a place
    /// and changes what is in it; this names a list and yields a longer
    /// one, leaving its operand alone as every ZDeceptron value is
    /// unaliased. Reusing the verb costs no reserved word — §14G.7.7's
    /// budget is untouched — and it keeps §4.1 because `append` means
    /// exactly one thing in both positions: this element goes into that
    /// collection.
    Append {
        item: Box<Expr>,
        list: Box<Expr>,
        span: Span,
    },
    /// `set key to value in table`: the map construction form.
    ///
    /// What `append item to list` is to a list, in the three words §14B.2
    /// already spends on the `set` mutation plus the `in` §14G.2 already
    /// spends on a route parameter's source. A mutation names a place and
    /// changes what is in it; this names a map and yields another one with
    /// the key set, leaving its operand alone. No reserved word is added,
    /// and `set` means one thing in both positions: this key now holds
    /// that value.
    ///
    /// Only this one form, and not a removal form beside it. A map is
    /// immutable, so every construction copies, so a removal written as a
    /// fold above this form costs the same order as a native delete on a
    /// copy would. `prelude/map.zd` records the trade.
    Insert {
        key: Box<Expr>,
        value: Box<Expr>,
        table: Box<Expr>,
        span: Span,
    },
    /// `map each x in maybe to x * 2` — transform the payload of a
    /// container that holds zero or one (#103, #104).
    ///
    /// **The one expression in the language that binds a name, and still
    /// not a function value.** `var` is bound to the payload for the
    /// duration of `to`, `None`, `Loading` and `Failed` pass through
    /// untouched, and nothing is passed anywhere: the body is a syntactic
    /// expression, so a call inside it still resolves to a top-level name
    /// at compile time and the call graph stays exact (§17.2.5). It is the
    /// same trade `map each row to …` has always made in a pipeline —
    /// there is no lambda, only a binder.
    ///
    /// **`Option` and `Remote` only, and the checker enforces it.** A
    /// `List` already has this phrase in the pipeline, and admitting one
    /// here would give a single construct two spellings, which is what
    /// §4.1 forbids. The pipeline walks sequences; this walks the
    /// containers holding zero or one, which the pipeline cannot reach.
    ///
    /// The spelling is settled by the token after the binder and needs no
    /// lookahead: `to` is the pipeline clause, `in` is this form. In
    /// statement position `map` is always the clause, because a statement
    /// is never parsed as an expression.
    MapInside {
        var: Ident,
        source: Box<Expr>,
        to: Box<Expr>,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Conditional { span, .. }
            | Expr::Number { span, .. }
            | Expr::Text { span, .. }
            | Expr::Truth { span, .. }
            | Expr::Empty { span }
            | Expr::List { span, .. }
            | Expr::Map { span, .. }
            | Expr::Var { span, .. }
            | Expr::Call { span, .. }
            | Expr::Of { span, .. }
            | Expr::Environment { span, .. }
            | Expr::Scroll { span }
            | Expr::Address { span }
            | Expr::Media { span, .. }
            | Expr::Build { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Field { span, .. }
            | Expr::Index { span, .. }
            | Expr::Append { span, .. }
            | Expr::Insert { span, .. }
            | Expr::MapInside { span, .. } => *span,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_expr_span_covers_both_operands() {
        let lhs = Expr::Number {
            value: 1.0,
            span: Span::new(0, 1),
        };
        let rhs = Expr::Number {
            value: 2.0,
            span: Span::new(4, 5),
        };
        let sum = Expr::Binary {
            op: BinOp::Add,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span: Span::new(0, 5),
        };
        assert_eq!(sum.span(), Span::new(0, 5));
    }

    #[test]
    fn span_is_available_for_every_expression_kind() {
        let s = Span::new(2, 6);
        assert_eq!(Expr::Empty { span: s }.span(), s);
        assert_eq!(
            Expr::Truth {
                value: true,
                span: s
            }
            .span(),
            s
        );
        assert_eq!(
            Expr::Text {
                value: "x".into(),
                span: s
            }
            .span(),
            s
        );
    }

    #[test]
    fn all_lists_every_placement_exactly_once() {
        // "Every placement", so the count is written out by hand: an
        // emptied or shortened `ALL` would otherwise make the loop below
        // agree with itself about nothing.
        assert_eq!(Placement::ALL.len(), 5, "{:?}", Placement::ALL);
        for (position, placement) in Placement::ALL.iter().enumerate() {
            assert_eq!(placement.index(), position, "{placement:?} is out of order");
        }
    }
}
