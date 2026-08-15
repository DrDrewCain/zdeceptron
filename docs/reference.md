# The language reference

This describes ZDeceptron as the compiler in this repository accepts it. It
is not a specification and it is not a wish list: every program in it was run
through `zdc check` before it was pasted, and where the language does not do
something the section says so and names the issue rather than describing the
intention in the present tense.

For a narrative introduction, read [the tutorial](tutorial.md) first. For the
rule behind any diagnostic, run `zdc explain <code>` — 71 of them are written
out in full, with a rejected and an accepted example each.

**Contents**

1. [Layout and lexical structure](#1-layout-and-lexical-structure)
2. [Declarations](#2-declarations)
3. [Placements](#3-placements)
4. [Types](#4-types)
5. [`Remote of T`](#5-remote-of-t)
6. [Expressions](#6-expressions)
7. [Pipelines](#7-pipelines)
8. [Statements](#8-statements)
9. [The view](#9-the-view)
10. [Events](#10-events)
11. [Information flow](#11-information-flow)
12. [Build capabilities](#12-build-capabilities)
13. [Diagnostics](#13-diagnostics)
14. [The wire format](#14-the-wire-format)
15. [Not implemented](#15-not-implemented)

---

## 1. Layout and lexical structure

Indentation is significant and there are no braces. A block is an indented
run of lines; leaving the indent ends the block. Trailing spaces on an
otherwise empty line do not count, so an editor that strips them cannot
change a program.

`#` begins a comment and runs to the end of the line.

Nesting is bounded: source that nests deeper than the compiler will follow is
`E0105` rather than a stack overflow.

### Literals

| form | type |
|---|---|
| `42` | `Whole` |
| `3.5` | `Decimal` |
| `"a string"` | `Text` |
| `yes`, `no` | `Truth` |
| `["a", "b"]` | `List of Text` |
| `empty` | the empty `List` or `Map`, as required |
| `None`, `Some with value is x` | `Option of T` |

There is **no string interpolation**. Text that varies is a separate node in
the view or a `+` in an expression.

### Reserved words

These are keywords everywhere and can never be used as names — `E0102` if you
try:

```
secret trusted release limit state function view record choice component
route use for children client static server durable unique starting emitting
from of to give set add subtract append remove keep sort map take first
where by when each in if otherwise show on with and or not is at contains
yes no empty environment address build media
```

Twenty-one more are *soft* keywords: they mean something only in the one
place a construct expects them, and stay ordinary names everywhere else. They
are `foreign`, `as`, `takes`, `gives`, `anywhere`, `pure`, `per`, `visitor`,
`remembered`, `new`, `nothing`, `do`, `test`, `expect`, `every`, `after`,
`frame`, `key`, `request`, `fold` and `into`. A record may still have a field
called `pure`, `test`, `frame`, `key`, `request` or `into`, a signal may still
be called `nothing`, a function may still be called `fold`, the standard
library still declares `function replace with value, old, new`, and
`state remembered is client Text starting ""` is a signal called
`remembered`.

`remembered` is the one placement that is soft rather than reserved, and it
is soft because the slot allows it: a placement is mandatory after a `state`
declaration's `is`, and a type follows it, so one token settles the
production. The others predate that accounting rather than needing it.

`List`, `Option`, `Remote`, `Map` and `Pair` are type constructors rather
than reserved words, and `first` is a keyword only in `take first`.

---

## 2. Declarations

A program is a sequence of top-level declarations in any order. Names are
resolved across the whole file, so a declaration may refer to one written
below it.

### `state` — a signal

```
state NAME is [PLACEMENT] TYPE starting EXPR
state NAME is [PLACEMENT] TYPE from EXPR
state NAME is client Decimal every "DURATION"
state NAME is client Decimal every frame
state NAME is client Truth   after "DURATION"
```

Every value that changes is a signal, and there is no second mechanism.
`starting` gives a *stored* signal its initial value; `from` makes it
*derived*, in which case it has no initial value and nothing may write to it.
`every` and `after` make it a *clock*; see [the clock](#the-clock) below.

The placement is not optional and the compiler will not infer it — `E0101`.

`secret` and `trusted` are modifiers written before `state`; see
[§11](#11-information-flow).

Two sources are not expressions and may only appear after `from`:

- `environment "NAME"` — a deployment secret. Always `Text`, and always
  secret whether the word `secret` is written or not.
- `address` — the URL the browser loaded, as an `Option of` the program's
  `route` type. A signal initialised from `address` is immutable: the browser
  writes it once at load and the program never does.

#### The clock

```zd
state elapsed is client Decimal every "250ms"   # milliseconds, every 250ms
state motion  is client Decimal every frame     # milliseconds, once per repaint
state ready   is client Truth   after "2s"      # `no`, then `yes`, once
```

A timer is a callback in every other language, and this language has no
callbacks. So `every` and `after` do not take one: they declare a **source
signal whose writer is the browser's scheduler** rather than a handler.
There is no block and nothing to run — the clause says what the cell holds
at each instant, and the program derives the rest with `from`. A clock is a
text box that types by itself.

| clause | type | what it holds |
|---|---|---|
| `every "DURATION"` | `Decimal` | milliseconds since the signal started |
| `every frame` | `Decimal` | the same, written once per animation frame |
| `after "DURATION"` | `Truth` | `no`, then `yes`, once |

A duration is a number and a unit with no space: `ms`, `s` or `m`, as in
`"250ms"`, `"1.5s"` or `"2m"`. The shortest is `"4ms"`, because browsers
clamp anything shorter; the longest is `"60m"`, because a browser timer past
`2^31 - 1` milliseconds fires immediately rather than late. For motion write
`every frame` and not a number near 16 — a 120 Hz display does not have 16 ms
frames.

Four rules follow from what a clock is:

- **`client` only** — `E0322`. A build has no later, a `server` signal does
  not outlive its request, and `durable` is storage rather than something
  still running. `every` on a `server` or `durable` state is asking for a
  *scheduled* state, which the language sketches and does not have; the
  diagnostic says so rather than claiming timers are client-only.
  `remembered` is refused for a different reason, because it is the one that
  *is* on the browser: a clock could run there, and what it would keep is a
  reading taken during a visit that has ended.
- **Nothing may write it.** `set elapsed to 0` is refused, and so is a
  two-way binding onto it. A tick therefore cannot start a request, append
  to a list or reach the store: if an animation is to cause something, the
  program says so with `from`, where it is visible.
- **It is Untrusted.** A visitor controls their own clock, so a reading is
  environmental — the same verdict the prelude's `clock` gets, reached the
  same way.
- **It is disposed with its view.** A clock declared inside a `component`
  belongs to the instance, so hiding the instance stops the timer. One
  declared at the top level lives as long as the page, which is what a page
  wants.

#### Where the reader is

```zd
state travelled is client Decimal from scroll
```

`scroll` is how far down the document the reader has come, as a `Decimal`
from 0 to 100. Like a clock it is a **source signal whose writer is the
browser** rather than a handler, and it is declared with `from` because it
is a quantity rather than an event.

[§10](#10-events) says `resize`, `scroll` and `pointermove` "have no form at
all: they are not events but quantities, and want a different construct."
This is that construct for the first of the three. `on scroll` would be a
handler running as fast as a finger moves, which is the callback shape the
language is built without; a scroll position is a cell, and the language
already has cells the browser writes.

It carries the clock's four rules, for the clock's four reasons:

- **`client` only** — `E0362`. A build host has no reader and a serverless
  invocation has no window, so there is nobody whose scrollbar this could
  be.
- **Nothing may write it.** `set travelled to 0` is refused.
- **It is Untrusted.** A visitor controls their own scrollbar, so a reading
  is environmental — the same verdict `media` and the clock get.
- **It is disposed with its view.**

A percentage rather than a pixel offset, because a pixel offset means
nothing without the document's height and the language exposes no way to
read one. One listener is installed per program however many times it is
read, it is `passive`, and its writes are coalesced to the animation frame:
a scroll fires far faster than a repaint, and a write per event would
schedule work the compositor throws away.

There is no `viewport width` yet. `resize` is the same kind of quantity and
would join this construct, and nothing has asked for it.

### `record` — a product type

```zd
record Book
    title is Text
    read  is Truth

state books is client List of Book starting [(Book with title is "Ada", read is no)]

view
    each book in books
        Row
            Text book.title
            Text book.read
```

Construct one by naming every field: `Book with title is draft, read is no`.
Read a field with `.`.

### `choice` — a sum type

```zd
choice Status
    Active
    Retired with since is Whole

state status is client Status starting Active

view
    when status
        Active
            Text "active"
        Retired with year
            Text year
```

A variant may carry fields. `when` is the only way to take one apart, and it
must cover every variant.

A variant may also carry a **label**: the text a person is shown where the
variant has to be read rather than matched, which today means a `Select`'s
options.

```zd
choice Equipment
    DirtBike  is "Dirt Bike"
    ATV
    LawnMower is "Lawn Mower"
```

A variant's name is an identifier and cannot hold a space, so without this
a `Select` could only ever offer `DirtBike`. The label is the option's
*text*; the option's *value* stays the variant's name, because that is what
the runtime round-trips on the way back — so two variants may share a label
and remain distinct, and a label may repeat another variant's name without
colliding. Nothing inside the program can read a label: `when` dispatches on
the variant, and an arm written with the label does not parse.

An arm with no label shows its name, so adding one to a `choice` changes
nothing about the arms that did not ask for it.

`Name is "text"` is deliberately the same shape as a `route`'s `Home is
"/"`, because it is the same idea: the string the variant is known by
outside the program. A `route`'s variants take no label — the string after
`is` is already spoken for, and it is the URL.

### `route` — a choice with a bijection onto URLs

```zd
state slugs is static List of Text starting ["routing", "folding"]

route Site
    Home    is "/"
    Writing is "/writing"
    Post    is "/writing" with slug is Text in slugs

state page is client Option of Site starting address

view
    when page
        None
            Heading "Nothing here"
        Some with here
            when here
                Home
                    Link Writing
                        Text "writing"
                Writing
                    each slug in slugs
                        Link (Post with slug is slug)
                            Text slug
                Post with slug
                    Heading slug
```

A route is dispatched with `when` exactly as any other choice is; a route
parameter is a variant field. `in slugs` names a `static` collection, which
is what makes the parameter enumerable at build time — the compiler emits one
document per value. The URL must be a canonical absolute literal path or you
get `E0106`.

`in` makes a parameter *enumerable*, and enumerable is all it makes it. The
value is still chosen by the visitor, so it is Untrusted; see
[§11](#11-information-flow).

### `function` — a named computation

Two parameter forms, and the form is part of the call:

```zd
state names  is client List of Text starting ["ada", "grace"]
state shouted is client List of Text from loudly with all is names
state count   is client Whole       from listLength of names

# `with` takes named arguments; called `loudly with all is names`.
function loudly with all
    from all
    map each name to name + "!"

# `of` takes exactly one positional argument; called `listLength of names`.
function describe of total
    if total is 0
        give "empty"
    give "not empty"

view
    Column
        Text (describe of count)
        each name in shouted
            Text name
```

`with` takes named arguments, `of` takes exactly one positional argument.
`give` returns. A body may be a pipeline (see [§7](#7-pipelines)) or a
sequence of statements.

**A function is not a value.** ZDeceptron has no first-class functions:
a `function` can be called and cannot be passed, returned, or stored. A
name used where a value belongs is refused —

```
`double` is a function, and ZDeceptron has no first-class functions, so it
cannot be used as a value. Call it with `double of …`.
```

— and a parameter used where an operation belongs is refused the other way
round, because a local is deliberately skipped in callee position so that a
library function keeps working inside a loop that binds its name:

```
`f` is in scope here, but it names a value, and ZDeceptron has no
first-class functions, so it cannot be the operation in `f of …`. Only a
top-level `function` can be called.
```

This one rule is why several things elsewhere in this document look the way
they do: it is why `anyOf` takes a `List of Truth` rather than a predicate,
and why `mapValues` takes already-computed values rather than a transform.
§5.4 separately rules out typeclasses and higher-rank types, so this is a
decision rather than a gap waiting to be filled.

**Two constructs take a transform anyway, and neither takes a value.** The
pipeline's `fold each` ([§7](#7-pipelines)) and the expression `map each x in
v to e` ([§6](#6-expressions)) both put a name in scope over an expression
that is written where it is used. Nothing is passed, so nothing is a
function: the body is syntax at the site, a call inside it still resolves to
a top-level name at compile time, and the call graph stays exact. That is the
whole of the trade — a *lambda as syntax* costs no value of function type,
and in exchange it can only appear where the grammar puts it.

### `component` — a reusable piece of view

```zd
component Tally with label
    state count is client Whole starting 0

    Column
        Text label
        Text count
        Button "more"
            on click
                add 1 to count

view
    Column
        Tally "left"
        Tally "right"
```

A component is used exactly where a built-in element is: `Tally "left"`. A
component's own `state` is per *instance* — two `Tally`s count independently,
and no line says so; it falls out of the state being declared inside.

`children` is a parameter name with a fixed meaning: the nodes nested at the
call site, placed wherever the body names them.

```zd
component Panel with title, children
    Details
        Summary title
        children

view
    Panel "the details"
        Text "inside the panel"
```

### `view` — the root

```zd
state count is client Whole starting 0

view title is "Shelf"
    Column
        Heading "Shelf"
        Text count
```

Exactly one per program. `title` is optional and sets the document title.

### `use` — reading another file

Given a `library.zd` beside this file that declares `slugs`, `titleOf` and a
`Byline` component:

```zd
use "./library" for slugs, titleOf, Byline

view
    Column
        Byline "ada"
        each slug in slugs
            Text (titleOf with slug is slug)
```

A relative path without the `.zd` extension, and an explicit list of names.
There is no wildcard import. How a program depends on another *program*
(rather than another file) is undecided — issue #174.

### `foreign` — a JavaScript symbol

```zd
foreign gauge is client
    from  "./gauge.js" as "mount"
    takes level is Whole, label is Text
    gives view

state level is client Whole starting 40

view
    Column
        gauge level is level, label is "load"
        Button "more"
            on click
                add 10 to level
```

`is` takes a placement, or the soft keyword `anywhere`. `gives view` makes
the foreign an element and hands the node to the module's export; `gives T`
makes it a computation. `gives pure T` asserts the result is a function of
the arguments, and `takes x is trusted Text` asserts a parameter must be
Trusted — see [§11](#11-information-flow).

A foreign reaches a package directly: a URL specifier is emitted as written,
and a bare one is mapped by the project's `zd.toml` `[packages]` table into
an emitted import map. `examples/tree-webgl/` is `from "three"` and three
files, none of them JavaScript.

#### Classes, methods and properties

Most of what a JavaScript library exports is a class, so more forms say so.
`gives new T` constructs — `new Export(…)` rather than `Export(…)` — and its
result is always `Handle`, the opaque host-object type ([§4](#4-types)).

`on Handle as "m"`, `of Handle as "p"` and `set Handle as "p"` each replace
the `from` line entirely, and nothing is imported by any of them, because a
member comes with the object. They are the three things a host object's
interface consists of: `on` is a **method**, called on the call's first
argument; `of` is a **property**, read off it; `set` is that same property
**written**.

| source line | emits |
|---|---|
| `from "three" as "Scene"` | `Scene(…)`, or `new Scene(…)` with `gives new` |
| `on Handle as "add"` | `receiver.add(…)` |
| `of Handle as "domElement"` | `receiver.domElement` |
| `set Handle as "roughness"` | `receiver.roughness = value` |

None of the three costs a reserved word. `on` is already a keyword — `on
click` — `of` is one in `List of Text` and `length of items`, and `set` is
the verb §8's mutation statement begins with. That last one is the reason it
is also the right word here: a property of a host object is a place, and
`set` is how this language writes a value into a place.

```zd
foreign vector is client
    from "./three.module.js" as "Vector3"
    takes x is Decimal, y is Decimal, z is Decimal
    gives new Handle

foreign plus is client
    on    Handle as "add"
    takes target is Handle, other is Handle
    gives Handle

foreign lengthOf is client
    on    Handle as "length"
    takes of v is Handle
    gives Decimal

state size is client Decimal from lengthOf of (plus with target is (vector with x is 1, y is 2, z is 2), other is (vector with x is 2, y is 4, z is 4))
```

emits `new Vector3(1, 2, 2).add(new Vector3(2, 4, 4)).length()`.

The first parameter of all three is the receiver and must be `Handle`, and
none of them owns a view nor constructs. A property read takes *only* the
receiver: `x.p` has no argument list, so a second parameter is refused
rather than dropped. A write takes exactly two — the object and the one
value — and must `gives nothing`:

```zd
foreign setRoughness is client
    set   Handle as "roughness"
    takes surface is Handle, value is Decimal
    gives nothing

do setRoughness with surface is bark, value is 0.9
```

`x.p = v` does evaluate to `v` in JavaScript, and handing that back would
be a second way to say a value the caller has just written down, so a write
gives no result and is run as a `do` statement like any other effect.

A foreign that mentions `Handle` at all must be `is client`, which is what
keeps a `secret` out of a host object ([§11](#11-information-flow)) — a
property read carries the receiver's label exactly as a method call does, so
nothing can be read back out of a handle that could not be put into one, and
a write is a call whose arguments are the receiver and the value, so nothing
that could not be passed to a method can be written into a field either.

#### `gives nothing` — a call run for its effect

Much of a host library is called for what it does. `gives nothing` says no
ZDeceptron value comes back, which is the claim `gives view` already makes:
it is about this program, not about JavaScript, and it is true of
`scene.add(mesh)` even though `add` returns the object for chaining.

A call to one has no type any expression position accepts, so it can only be
written as a `do` statement ([§8](#8-statements)). It carries no `pure` or
`trusted` grant, because there is no result for one to describe.

```zd
foreign addTo is client
    on    Handle as "add"
    takes parent is Handle, child is Handle
    gives nothing

do addTo with parent is world, child is limb
```

### `release` — a bounded disclosure

```zd
state answer is durable Text            starting "cabbage"
state guess  is client  Text            starting ""
state result is server  Option of Truth from judge with guess, answer

release judge with guess, answer
    gives Truth
    trusted guess
    trusted answer
    limit 20 per visitor
    give guess is answer

view
    Column
        Input guess, hint is "your guess"
        when result
            Loading show Spinner
            Failed with error show ErrorBar message is "the judge is unavailable"
            Ready with verdict
                when verdict
                    None
                        Text "no guesses left"
                    Some with right
                        Text right
```

The clause order is fixed — `gives`, then endorsements, then `limit`, then
statements — so the declaration stays LL(1).

Two consequences worth knowing before you write one:

- **The result type is `Option of` the `gives` type.** A caller writes
  `state result is server Option of Truth from judge with guess, answer`, and
  `None` is what a visitor past the `limit` receives.
- **A release body may not read a signal** (`E-REL-04`) and **every argument
  must be endorsed or traceable to a grant** (`E-REL-08`). Everything the
  body uses arrives as a parameter, and each parameter is accounted for.

`limit N per visitor` bounds evaluations per declaration per anonymous
session. It is not a cumulative disclosure bound and the compiler's own
warning text is forbidden from implying that it is. A release with no `limit`
warns (`W-REL-01`). See [§15](#15-not-implemented) for what `release` does
not yet do.

### `request` — an outbound HTTP request

```zd
request quote is client
    from  "/quote.txt"
    with  topic is subject
    gives Text

state subject is client Text starting "signals"

view
    when quote
        Loading           show Spinner
        Failed with error show ErrorBar message is error.message
        Ready with body   show Text body
```

The declaration **is** a signal. Reading `quote` gives `Remote of Text`, so
it is spent with the same three-armed `when` a `server` signal is
([§5](#5-remote-of-t)), and it re-runs when one of its `with` arguments
changes — a request is a value that depends on other values.

Four rules, and each of them is a security property rather than a
convenience:

- **The destination is a literal**, on its own line. Nothing computed can
  go there. A computed destination could not be checked by any pass and
  could not be named in the emitted policy, and `fetch("https://x/?k=" +
  apiKey)` is a leak with no body at all. A same-origin destination starts
  with `/`; a cross-origin one is written out in full and must be `http:`
  or `https:`.
- **The arguments become the query string.** `with topic is subject` sends
  `?topic=…`, percent-encoded, so a value cannot leave its own parameter.
  Each one is a sink-7 site ([§11](#11-information-flow)): a `secret` in
  any of them is `E-IFC-11`.
- **There is no header clause and no body.** A request is a `GET` carrying
  one `accept` header the runtime chose. `Authorization: Bearer <secret>`
  is the shortest path from a credential to a third party, and the answer
  to it is that the route does not exist.
- **`is client`, and `gives Text`.** Only a browser has somewhere to wait
  for a request; a `server` one would spend the deployment's own
  credentials and network position, which is a different question and is
  not built. A response body is bytes somebody else chose, so `Text` is
  what it is — and `gives Markup` would be a claim that a third party's
  HTML may be parsed as HTML.

A cross-origin destination widens the emitted `connect-src` to name that
origin and nothing else ([§9](#the-emitted-document)). What comes back is
**Untrusted** ([§11](#11-information-flow)), so it cannot reach a `trusted`
place.

---

## 3. Placements

Five, and they are three machines and two stores.

| placement | one value per | when it runs | may be written by |
|---|---|---|---|
| `client` | open tab | in the browser | handlers in that tab |
| `remembered` | browser profile, per origin | in the browser | handlers in **any** tab, and any other script on the origin |
| `static` | build | once, at compile time | nothing — `E0310` |
| `server` | request | a serverless invocation | nothing directly — `E0311` |
| `durable` | program | a store that outlives both | handlers, through generated machinery |

**`durable` is one value per *program*, not per visitor.** The "one value
per" column is the whole of it, and it is the sentence most likely to be
read as something milder than it is: there is no per-visitor durable
storage, every request reads and writes the same store, and a durable row
is visible to any request that computes its key. Scoping a value to one
visitor — `orders at thisPerson`, a `keep` on an owner field — is your
program's job, and **nothing checks that you did it**. A forgotten filter
is a data leak that compiles, and no pass in this compiler will object.
`durable per visitor` is refused with `E0107`, which explains why the
language does not offer to do this for you.

`remembered` is to `client` what `durable` is to `server`. The pairs are not
two machines each: they are one machine and two lifetimes. `server` state is
one value per request and `durable` state outlives every request; `client`
state is one value per open tab and `remembered` state outlives the tab. It
compiles to `localStorage`, so it survives a reload, every tab of that
browser shares it, and no other visitor and no server ever sees it.

```zd
state visits is remembered Whole starting 0

view
    Column
        Text visits
        Button "count this visit"
            on click
                add 1 to visits
```

`starting` means *the value on a browser that has never run this program*.
On every later visit the value is whatever is in the store. Three rules
follow from that and none of them is optional:

- **It may not be `secret`** — `E0313`. Every script on the origin can read
  the store and the entry outlives the visit, so a token in one is a token
  published.
- **Reading one is always Untrusted**, and `trusted remembered` is
  `E-INT-01`. What is in the store was put there by a previous session, by
  another tab, or by another script on the origin — none of which is in this
  program — so a value cannot be laundered to Trusted by writing it to the
  store and reading it back. See [§11](#11-information-flow).
- **It may not be derived** — `E0321`, as for `durable`. A derived signal is
  recomputed on every load, which would overwrite what survived the reload.

The entry's key is `zd:` and the signal's name. There is no way to compute
one: a key a program could choose is a key it could choose from a value, and
that is a way to read a cell the program never declared.

`sessionStorage` has no placement — see [§15](#15-not-implemented).

A `static` signal is computed by the build and inlined into the bundle;
writing to one is `E0310`. A `server` signal is *recomputed from its inputs*
rather than assigned, so a browser cannot write it — `E0311`. Reading state
that does not exist at build time from a `static` signal is `E0301`.

The compiler generates an endpoint for a `server` or `durable` signal exactly
when something on the client reads it. Declare one nothing reads and you get
`W0330` — there is no endpoint, so the declaration is a mistake. The
equivalent for an unread `client` signal is `W0331`.

Two more placement rules have codes of their own: signals defined in terms
of each other are `E0320`, and a `durable` signal that is derived rather than
stored is `E0321`. A `Handle` written anywhere it would have to travel is
`E0317` — which is also the code for a handle that would be *replaced*: a
handle may live in `client` state declared `starting`, acquired once and
never written, and `from` or a `set` is refused because there is no `destroy`
to run on the object dropped. `environment` read outside a server context is
`E0360`.

`durable` is a key-value store. Related data needing queries, joins and
aggregation is issue #36. Per-principal durable state is **not** open work:
`durable per visitor` is refused by `E0107`, because partitioning by
principal needs a principal and the language has no way to establish one.
See [§14](#14-not-implemented).

---

## 4. Types

### Primitives

| type | |
|---|---|
| `Whole` | an integer |
| `Decimal` | a fractional number |
| `Text` | a string |
| `Truth` | `yes` or `no` |
| `Markup` | rendered rather than shown; the only producers are `build markdown` and the `markup` field of a `Part`, and the only consumer is the `Prose` element |
| `Code` | the closed failure choice: `Unreachable`, `Timeout`, `Rejected` |
| `Handle` | an object the host owns and the program only passes around |

`Whole` overflow on the client path is unguarded — issue #5.

A `Handle` has no literal, satisfies no constraint, and cannot be shown,
compared or indexed. It may be written bare in exactly three places — a
`foreign`'s parameter type, a `foreign`'s result type, and the type of a
`client` signal declared `starting` — and `E0317` refuses it anywhere else,
including under `Remote of`, as a record field, and in `server`, `durable`
or `static` state. A handle refers to an object in one JavaScript heap, so
there is no wire form to send: what would be encoded is an identity inside a
running process.

The `client` `starting` signal is where a renderer, a canvas context or an
audio node lives, and the two conditions on it are one condition: **it is
never replaced.** A derived signal recomputes and a `set` overwrites, and
neither has a `destroy` to run on the object dropped, so both are `E0317`
too. What that buys is a lifetime the language can state — the document's.
The handle is acquired once, when the bundle loads, and released when the
page is. Releasing one sooner is a call the program makes:

```zd
foreign disposeOf is client
    on    Handle as "dispose"
    takes of r is Handle
    gives nothing
```

### Constructors

| type | notes |
|---|---|
| `List of T` | |
| `Map of K to V` | `m at k` is `Option of V`, because the key may be absent |
| `Pair of A to B` | |
| `Option of T` | variants `None` and `Some with value is x` |
| `Remote of T` | see [§5](#5-remote-of-t) |

They nest as you would expect: `Map of Text to List of Whole`,
`List of Pair of Text to Whole`.

### Where a type is required

Constraints appear in diagnostics by name:

- **Shown** — what an element may display: `Text`, `Whole`, `Decimal`, `Truth`.
- **Addable** — what `+` accepts: `Whole`, `Decimal`, `Text`.
- **Numeric** — what `-`, `*`, `/` accept: `Whole`, `Decimal`.

### The standard library

The prelude is written in ZDeceptron and lives in `crates/zdc-lib/prelude`:
`list.zd`, `map.zd`, `text.zd`, `number.zd`, `math.zd`, `option.zd`,
`remote.zd`, `time.zd`, `encode.zd`. Its functions are in scope in every
program without an import — `first of items`, `join with parts, using`,
`keys of table`, `valueOr with maybe, fallback`, `slice with value, start,
stop`, and so on.

### `math.zd` — transcendentals, constants and vectors

Angles are **radians**, as every platform's trigonometry is; `radians of`
and `degrees of` convert, and both are total.

| | |
|---|---|
| constants | `pi`, `tau`, `eulerNumber` — zero-argument functions, because a constant has no placement |
| circular | `sin of`, `cos of`, `tan of`, `asin of`, `acos of`, `atan of`, `atan2 with rise, run` |
| growth | `exp of`, `ln of`, `log10 of`, `log2 of` |
| roots | `cbrt of`, `hypotenuse with across, up` |
| vectors | `dot with left, right`, `magnitude of`, `magnitudeSquared of`, `scaled with vector, factor`, `added with left, right`, `subtracted with left, right`, `distance with start, finish`, `axis with vector, index` |
| interpolation | `mix with start, finish, ratio`, `progress with value, low, high`, `clamped with value, low, high`, `clamped01 of` |
| easing | `smoothStep of`, `smootherStep of`, `easeIn of`, `easeOut of` |
| angles | `wrapAngle of`, `angleDelta with start, finish`, `bearing of`, `fromAngle of` |
| geometry | `cross2 with left, right`, `cross3 with left, right`, `normalized of`, `angleBetween with left, right`, `rotated2 with vector, angle`, `projected with vector, onto`, `reflected with vector, normal` |
| matrices | `rowOf with matrix, index`, `columnOf with matrix, index`, `rowCount of`, `columnCount of`, `transposed of`, `applied with matrix, vector`, `matrixProduct with left, right`, `matrixScaled with matrix, factor`, `matrixAdded with left, right` |
| statistics | `mean of`, `variance of`, `standardDeviation of` |
| activations | `sigmoid of`, `rectified of`, `leakyRectified with value, slope`, `hyperbolicTangent of`, `softmax of` |

**Every primitive gives an `Option`, under the rule `sqrt` and `power`
already carry: `None` unless the answer is a finite number.** `ln of 0` is
`None`; so is `sin of (1 / 0)`. Eliminate it with `valueOr`, exactly as for
`quotient`. There is deliberately no total variant — that would be a second
spelling of one operation, which §4.1 refuses.

The vector operations are written in ZDeceptron rather than declared as
primitives, and are total. A vector is a `List of Decimal`; `dot` is built
on `zip`, so two vectors of different lengths give the product over their
common prefix. `atan2` is the one member of the family that knows which
quadrant it is in, and is what turns a vector back into an angle.

`cbrt` is not `power with exponent is (1 / 3)`: a fractional power of a
negative number is `NaN`, and the cube root of −8 is −2. `hypotenuse` is not
`sqrt of (a * a + b * b)`: the naive form overflows when the squares do.

**Vectors and matrices are lists, not types.** A vector is a `List of
Decimal` and a matrix is a `List of List of Decimal` in row-major order.
There is no `Matrix` type and there should not be one until the language
has a shape to check — a type that cannot say "n by m" is a rename of the
list, and a rename is not a guarantee. What that costs is worth stating:
nothing checks that rows are the same length or that two matrices are
conformable, so a ragged input gives a ragged answer rather than a
refusal. Indexing is total, so `axis` of a component that is not there is
`0.0`, which is what lets `cross3` take two 2D vectors and mean it.

**Every literal in this file is written `2.0` and not `2`.** A bare `2` is
a `Whole`, there is no implicit widening, and `Decimal * Whole` is a type
error. `decimalOf` is the widening, written down.

`softmax` shifts by the largest term before exponentiating, so the biggest
exponent is `e^0`. The unshifted form is one line shorter and gives `NaN`
for inputs a real network reaches.

---

## 5. `Remote of T`

This is the type that makes the language's argument, so it gets its own
section.

Reading a `server` or `durable` signal **from a `client` placement** does not
give you `T`. It gives you `Remote of T`: a value that is loading, or has
failed, or is ready. There is no other way to see across that boundary, and
the compiler will not hide the crossing.

```zd-rejected
record Book
    title is Text
    read  is Truth

state books  is durable List of Book starting empty
state unread is client  List of Book from unreadOf with books

function unreadOf with all
    from all
    keep each book where not book.read

view
    each book in unread
        Text book.title
```

```
Error: `all` of `unreadOf` is `Remote of List of Book`, but `List of a type
that is not known here` is expected here.
```

The only thing you can do with a `Remote` is spend it with `when`, and all
three arms are required:

```zd
record Book
    title is Text
    read  is Truth

state books  is durable List of Book starting empty
state unread is server  List of Book from unreadOf with books

function unreadOf with all
    from all
    keep each book where not book.read

view
    when unread
        Loading           show Spinner
        Failed with error show ErrorBar message is error.message
        Ready with list
            each book in list
                Text book.title
```

An arm is either `show ELEMENT` on one line or an indented block. The
`Failed` payload has `.message` (a `Text`) and `.code` (a `Code`), and
matching on the code is how a program says something useful:

```zd
state total is durable Whole starting 0

view
    when total
        Loading show Spinner
        Failed with error
            when error.code
                Unreachable show ErrorBar message is "no answer"
                Timeout     show ErrorBar message is "took too long"
                Rejected    show ErrorBar message is "said no"
        Ready with count show Text count
```

The other answer to a `Remote` is to not create one: move the derivation to
where the data already is, by declaring it `server`. Then the crossing
happens once, at the view, instead of inside every function.

Two current limits, both real:

- **Two `Remote of T`s cannot be combined** — issue #20. You cannot pass two
  of them into one function and add the results; the arguments are rejected
  for not being `Whole`, `Decimal` or `Text`.
- **There is no `map` over `Remote`** — issue #104 — and none over `Option`
  either — issue #103. `remote.zd` gives you `readyOr` and `isReady`;
  `option.zd` gives you `valueOr`, `isSome` and `isNone`.

---

## 6. Expressions

### Operators

Symbolic: `+ - * /` and `< > <= >=`.

Word: `is` (equality), `and`, `or`, `not`, `at` (index), `contains`.

There is no `!=`; inequality is `not (a is b)`. The infix set is closed —
`+` over `Text` is the one text operator the language has.

```zd
state a is client Whole starting 1

view
    Column
        if not (a is 2)
            Text "differs"
        if a >= 1 and a <= 9 or not (a is 0)
            Text "range"
        if ["x"] contains "x"
            Text "contains"
```

### `media` — what the visitor's display asks for

```zd
state dark is client Truth from media "(prefers-color-scheme: dark)"
state calm is client Truth from media "(prefers-reduced-motion: reduce)"
state wide is client Truth from media "(min-width: 48rem)"
```

`media` takes a quoted CSS media query and gives a `Truth`. The query is a
literal and may not be computed: `matchMedia` subscribes for the life of the
page, so a query built from a value would have to re-subscribe and nothing
in the language says when.

**It is a signal, not a read.** When the visitor changes their system
theme, resizes the window, or turns animation off, every view that shows one
changes with it. Reading the answer once and keeping it is the ordinary bug
in hand-written code and it is not writable here.

Only the browser can answer, so `media` outside client context is `E0362` —
the same shape of rule as `environment` (`E0360`) and `build` (`E0361`), and
refused for the same reason: there is nobody to ask. The answer is the
visitor's own preference, so it is Untrusted (see
[§11](#11-information-flow)); it is never secret.

The style vocabulary's `dark:` prefix already handles *colours* under a dark
scheme without any of this. `media` is for when the program has to choose
something other than a colour.

### Calls

The call form matches the declaration: `f with a is x, b is y` for a `with`
function, `f of x` for an `of` function. A call in an argument position is
parenthesised: `Text (titleOf with slug is slug)`.

### Places

```
place := IDENT (("at" primary) | ("." IDENT))*
```

`.` reads a record field or a variant field; `at` indexes a `List` or `Map`.
A `Map` index is an `Option`, so it is spent with `when` or with
`atOr with table, key, fallback`.

### Conditionals

`if`/`otherwise` is a statement in a body and a region in a view. A function
may also fall through a series of `if`s to a final `give`:

```zd
state left   is client Whole starting 0
state advice is client Text  from adviceFor with count is left

function adviceFor with count
    if count is 0
        give "Nothing waiting. Add something."
    if count > 5
        give "That is a queue, not a shelf."
    give "A reasonable shelf."

view
    Text advice
```

### `when`

`when` dispatches on a choice, a route, an `Option`, a `Remote` or a `Code`.
Every variant must be covered. In a body, each arm is a pattern line followed
by an indented block:

```zd
choice Status
    Active
    Retired with since is Whole

state status is client Status starting Active
state label  is client Text   from describe of status

function describe of mode
    when mode
        Active
            give "active"
        Retired with year
            give "retired"

view
    Text label
```

A variant's fields are bound positionally by the `with` on the pattern.

### `map each … in …` — transforming what is inside

`when` is a statement, and a sort key, an argument and a `from` are all
expressions. `valueOr` exists because of that gap; this is the other half of
it, for the case where the container should survive rather than collapse:

```zd
state chosen  is client Option of Whole starting None
state doubled is client Option of Whole from map each n in chosen to n * 2
```

`n` is bound to the payload for the duration of the body. `None`, `Loading`
and `Failed` pass through untouched, which is the thing `readyOr` cannot do:
a program can transform what came back without also deciding what to do about
a request that has not answered.

**`Option` and `Remote` only.** A list is refused, and the refusal names the
pipeline, because the language already has a phrase for walking a sequence
and one construct may not have two spellings. The two forms are told apart by
the word after the binder — `to` is the pipeline clause, `in` is this — and
in statement position `map` is always the clause.

`andThen` is this plus one library function: `flattenOption of (map each x in
maybe to (lookup of x))`, with `flattenRemote` beside it for a `Remote`.

---

## 7. Pipelines

A function body may be a pipeline. `from` names the collection; every clause
after it transforms the value.

```zd
record Player
    name   is Text
    score  is Whole
    active is Truth

state players is client List of Player starting empty
state board   is client List of Text   from topNames with all is players

function topNames with all
    from all
    keep each player where player.active
    keep each player where player.score > 0
    sort each player by player.score
    map each player to player.name
    take first 10

view
    each name in board
        Text name
```

The clause order is fixed and is the grammar rather than a convention:
`from` first, then any number of `keep` / `sort` / `map` in any order, then
an optional `take first N`, then an optional `fold each`.

### `fold each` — the clause that accumulates

```zd
function revenue of rows
    from rows
    keep each row where row.active
    fold each row into total starting 0 to total + row.amount
```

`starting` is evaluated once, before anything is walked. `step` — everything
after `to` — is evaluated once per element with both names in scope, and its
type is the seed's type: a fold does not change what it is accumulating. A
fold over an empty list is the seed.

**It ends the pipeline.** Every other clause takes a sequence and gives a
sequence; this one gives a single value, so `keep`, `sort`, `map each` and
`take first` have nothing left to walk and are refused by name. A second
`from` starts a new pipeline.

The accumulator is any expression, which is what makes it useful: a record
literal accumulates several answers at once, and `append item to taken`
accumulates a list. `examples/sorting.zd` folds a `Run` — the sorted list and
the comparison count together — and `crates/zdc-lib/prelude/list.zd` writes
`sumOf`, `countOf`, `minOf`, `maxOf` and `flatten` this way.

`fold` and `into` are soft keywords, so both are still available as names.

Three of the library's list functions are *not* written with this clause and
say why at their definitions: `anyOf`, `allOf` and `listContains` stop early,
which a clause that visits every element cannot do.

---

## 8. Statements

A handler or function body is a sequence of statements. There is no
assignment operator: a mutation names the verb, the value and the place, and
a call made for its effect is introduced by `do`.

| statement | on |
|---|---|
| `set PLACE to EXPR` | anything |
| `add EXPR to PLACE` | numbers |
| `subtract EXPR from PLACE` | numbers |
| `append EXPR to PLACE` | a `List` |
| `remove EXPR from PLACE` | a `List` |
| `give EXPR` | returns from a function or release |
| `do EXPR` | a call to a `foreign … gives nothing`, run for its effect |

Mutating a `Map` entry — `set tally at draft to 5`, `add 1 to tally at draft`
— works on a **`durable`** place only. On a `client` place it is refused:

```
Error: A mutation through a path such as `scores at player` needs an
immutable-update helper the runtime does not have and §14B.3 has not settled.
```

On a `server` place any write at all is `E0311`. Writing to a value rather
than a place is `E0314`.

---

## 9. The view

The view is a function of the signals, and nothing in the program says so.
There is no subscription, no dependency array and no re-render call: the
compiler reads the dependencies out of the program.

### Elements

Built-in elements are capitalised and take a positional argument and named
arguments: `Text count`, `Input draft, hint is "a book"`,
`Meter level, least is 0, most is 100, low is 20, high is 80, best is 60,
label is "load"`, `Row padding is 8`. Nesting is indentation.

A component is called the same way, which is the point.

The element vocabulary is the widest thing in the language and is documented
per element in [the standard library
pages](https://zdeceptron.marksturman.com/docs/standard-library); the
argument for its size and shape is issue #241.

**Accessibility is what the vocabulary is closed for.** Some of it is a
refusal: an `Image` must be given `alt`, a `Frame` a `title`, a `Radio`, a
`Label` and a `Dialog` their names, and a `Fieldset` and a `Details` must lead
with the child that names them. Some of it is fixed: a `Spinner` is
`aria-busy`, an `ErrorBar` is `role="alert"`, a `HeaderCell` is
`scope="col"`, a `Video` and an `Audio` have controls that cannot be turned
off, and a `Heading`'s level is its nesting depth, so an outline that skips a
level is not expressible.

The rest a program asks for, and the argument set it asks through is
**closed**: a name the compiler does not know is a diagnostic rather than an
attribute, because `onclick`, `style` and `srcdoc` are attribute names too.
`aria-*` is reached through that closed set rather than around it — an
argument name is a UAX#31 identifier and admits no hyphen, so eleven
arguments translate:

| written | reaches | takes |
| --- | --- | --- |
| `selected` `expanded` `pressed` `checked` `disabled` | `aria-selected` … | a `Truth` |
| `decorative` | `aria-hidden` | a `Truth` |
| `controls` `describedBy` `labelledBy` | `aria-controls` … | an `id`, or `for` on a `Label` |
| `current` | `aria-current` | `page` `step` `location` `date` `time` |
| `live` | `aria-live` | `polite` `assertive` `off` |

A state's value is the word `true` or the word `false`, never the presence or
absence of the attribute, and that holds whether it is written down or bound
to a signal. `label` reaches `aria-label`, except on a `Checkbox` or a `Radio`,
which wrap their box in a `<label>` holding it.

And one of the eleven the compiler will write on its own. In a routed program,
a `Link` whose destination is the document it is written in gets
`aria-current="page"`, which is what marks the current item of a navigation for
anyone not looking at the screen. It is written into the markup rather than
computed in the browser, because the address fold knows both the document's URL
and the link's destination while it emits. Writing `current` on that `Link`
yourself replaces it, so a wizard's self-link can still say `step`. An unrouted
program gets nothing, and that is deliberate: its `index.html` can be hosted at
any path, so "the URL this document is served at" is not a fact the compiler
has.

### `Dialog` — a modal

```zd
state confirming is client Truth starting no

view
    Column
        Button "Delete"
            on click
                set confirming to yes
        Dialog confirming, label is "Delete this file?"
            Text "This cannot be undone."
            Button "Cancel"
                on click
                    set confirming to no
```

Whether the modal is showing is the `client Truth` it binds, and the binding
is two-way in the sense every field's is: writing the signal opens and closes
the dialog, and the dialog writes it back whenever the browser closes it. That
second half is not optional — Escape closes a modal without asking the
program, and a signal left saying `yes` is a page whose next click does
nothing.

Focus moves into the dialog when it opens, is trapped while it is open — the
rest of the page is *inert*, so the pointer and find-in-page are shut out
too — and **returns to whatever opened it** when it closes. None of that is
written here or emitted by the compiler: it is what `showModal()` does, and
this element exists to reach it. `label` is required because a modal moves
focus into itself, and what a screen reader says on arrival is the dialog's
name.

There is no `open` argument and no non-modal dialog. A panel that is merely
shown and hidden is an `if`.

### Regions

Four forms, and all four are descriptions of what the page contains rather
than instructions to build it:

```zd
state shown is client Truth starting yes
state words is client List of Text starting ["alpha", "beta"]
state pick  is client Option of Text starting None

view
    Column
        if shown                     # a conditional region
            Text "visible"
        otherwise
            Text "hidden"

        each word in words           # a repeated region
            Text word

        when pick                    # a dispatching region
            None
                Heading "Nothing chosen"
            Some with here
                Text here
```

plus a plain nested element. A binder introduced by `each` is in scope
throughout the region, including inside the handlers on elements within it —
which is how `remove book from books` inside an `each book in books` knows
which book.

### Navigation

`Link` is the only navigation, and it renders a real anchor. Following one
ends this program instance and begins a new one at the target document, which
is why `client` state does not survive a navigation — that is not a rule, it
is what "a new program instance" means.

Because `address` is immutable, programmatic navigation is not expressible:

```
Error: `page` is initialised from `address`, and a signal initialised from
`address` is immutable: the browser writes it once at load and the program
never does. Navigate with a `Link`, which renders a real anchor and starts a
fresh program instance at the target document.
```

### The emitted document

A build writes one `index.html` per URL. It loads exactly one module —
`boot.js`, whose two lines import `main` and call it — and carries a
Content-Security-Policy the compiler can prove the program satisfies:

```
default-src 'none'; script-src 'self'; style-src 'self';
img-src 'self' http: https:; font-src 'self' http: https:;
media-src 'self' http: https:; frame-src 'self' http: https:;
connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'
```

`connect-src` is the one directive a program can widen, and a `request`
declaration is the only thing that widens it. A cross-origin destination
adds **that origin** — `connect-src 'self' https://api.example.org` — taken
from the `from` line. Not `https:`, which would permit every host on the
web: the destination has to be a literal, which is what makes writing the
origin possible at all. A program with no cross-origin request emits the
policy above unchanged.

There is no `'unsafe-inline'` and no `'unsafe-eval'`, and both are absences
the compiler earns rather than aspirations. The page has no inline script,
which is why `boot.js` is a file. Nothing in the runtime or in generated
code evaluates a string. A `style` argument is refused outright: a static
declaration folds into a generated class in `styles.css`, and a reactive one
is a `setProperty` call, which is CSSOM and outside the policy's reach — so
no `style` attribute and no `<style>` element is ever written. `object` and
`embed` are not in the element vocabulary, nothing emits a `<base>`, and a
`Form` has no `action`, so those three are refused outright.

The four URL-bearing directives are `http:` and `https:` because those are
the schemes a program may name. A `Link`, an `Image` or a `Frame` takes a
URL a program computes, and `http`, `https`, `mailto` and `tel` are the only
schemes the compiler lets reach an attribute; the policy is the browser
enforcing the same allowlist a second time, at the point of use.

`frame-ancestors`, `report-uri` and `sandbox` are absent because a browser
ignores all three inside a `<meta>` element. They belong in a response
header, which is the deploy target's to set.

---

## 10. Events

```zd
state x is client Decimal starting 0
state y is client Decimal starting 0

view
    Column
        Button "where did I land?"
            on click with press
                set x to press.x
                set y to press.y
        Text x
        Text y
```

`on EVENT` runs a handler; `on EVENT with NAME` binds the payload. The
payload is typed, and typed **per event**: `press.x` is a `Decimal` on a
click and does not exist on a keystroke; `stroke.key` is a `Text` on a
keystroke and does not exist on a click. The set of events is closed, so
reaching for the wrong field is a compile error rather than `undefined`.

```zd
state typed     is client Text  starting ""
state lastKey   is client Text  starting "none"
state chords    is client Whole starting 0
state committed is client Text  starting "nothing yet"

view
    Column
        Input typed
            on keydown with stroke
                set lastKey to stroke.key
                if stroke.control
                    add 1 to chords
            on blur with leaving
                set committed to leaving.value
        Text lastKey
        Text chords
        Text committed
```

`Input name` wires `on input` itself, so a second handler for that event on
the same element is refused. `keydown` and `blur` are free.

There is no `preventDefault` and no `stopPropagation`: no built-in element
has a default action to prevent, `Button` being emitted `type="button"`
precisely so that it has none.

### Keys the whole page hears

`Escape` closes a dialog wherever the reader's focus happens to be, and the
arrow keys move a board. There is no element to attach that to, because the
element it listens to is the page. `on key` is that listener:

```zd
state open is client Truth starting no

view
    Column
        Button "open"
            on click
                set open to yes
        if open
            Column
                Text "press Escape to close"
            on key "Escape"
                set open to no
```

**It names its key, and it receives nothing.** There is no `with` binder and
the production has none to write. A handler learns one bit about one key
written in the source — not what was typed, not how many keystrokes there
were, not when. `on keydown with stroke` on an `Input` still binds its
payload, unchanged, because a field's own keystrokes were addressed to that
field; a document listener's were addressed to whatever the reader was
looking at, which may be a password box in a page region this program never
declared.

**The listener stands down while a field has focus.** The compiled listener
returns without looking whenever the event's target is an `input`,
`textarea`, `select` or anything `contenteditable`. That is what makes a
printable key safe: `on key "r"` cannot see the `r` in a password. Nothing in
the program asks for it — it is emitted.

**A key is one of `Escape`, `Enter`, `Tab`, `Backspace`, `Delete`,
`ArrowUp`, `ArrowDown`, `ArrowLeft`, `ArrowRight`, `Home`, `End`, `PageUp`,
`PageDown`, or any single character** — `"r"`, `"~"`, `" "` for the space
bar. The spelling is `KeyboardEvent.key`'s, exactly, and it is checked: `on
key "Esc"` is a listener that could never run, and the compiler says so
rather than leaving it as silence.

**Its position is its lifetime, and only that.** It goes beside the elements
— at the top of a view, a component body, or a branch — and is refused
indented under an element, because `on key` under a `Button` reads as "while
this button has focus" and does not mean it. Written inside `if open` as
above, the listener is registered when the branch appears and removed from
the document when it goes. A component's own `on key` belongs to the
instance, so two instances are two listeners and each dies with its instance.

Modifier chords such as `control k` are not expressible, and neither is
`keyup`. `resize`, `scroll` and `pointermove` have no form at all: they are
not events but quantities, and want a different construct.

### When a handler throws

**A handler that throws is contained to that handler and reported. The rest
of the page goes on working, and the writes the handler made before it threw
stand.**

Concretely: the runtime wraps every handler it attaches, so an exception does
not escape the listener. It is handed to `reportError`, the platform's own
uncaught-error channel — the one an exception nobody caught reaches anyway —
so `window.onerror`, the `error` event, and any error monitor already
installed on the page all see it, unchanged and once. Nothing is unmounted,
no signal is reset, and the next event is handled normally.

A handler is implicitly batched, so the bindings that read what it wrote are
still flushed: a handler that writes two signals and then throws repaints
both. A *binding* that throws during that flush is contained the same way,
because `flush` already keeps one failing computation from stopping the drain
and the exception it re-raises lands inside the handler's containment.

Two alternatives were considered, and ruling them out is the decision.

**Killing the runtime** — unmounting the view, or replacing it with an error
screen, on the first exception. Rejected. A page here is not a component tree
that a failure can be scoped to; it is a set of bindings, and the language's
whole model is that a write reaches exactly the bindings that read it. A
failure should reach exactly what it touched, for the same reason. One
mistyped handler on one button would otherwise destroy a page whose twenty
other controls are correct, including the ones a reader would use to get
their work out of it.

**Rolling the handler's writes back** — treating a handler as a transaction
over the signal graph and discarding its writes if it does not finish.
Rejected, and this is the one worth stating, because it sounds better than it
is. It would need a journal of every write, kept for every handler on every
event, in a runtime whose size gate has single-digit bytes of headroom. And
it could not deliver what it promises: a handler can call an endpoint, and
that request has already left; it can move focus, scroll, or open a dialog.
Rolling back the half of a handler's effects that happen to live in the graph
while the half that left the browser stands is not atomicity — it is a
*different* inconsistent state, reached less predictably. Durable writes are
the case that genuinely needs atomicity, and they already have it: one
handler gets one transaction, decided at compile time.

What is deliberately not offered is a way for a program to observe the
failure. There is no `on error` and no handler-failure hook, because the
language has a failure channel already — `Remote of T` and its `Failed` arm —
and a second one carrying JavaScript exceptions would be a way to smuggle
untyped values into a typed program.

---

## 11. Information flow

Two lattices, checked independently, and both are static.

### Secrecy — `secret`

`secret` is a modifier on a `state` declaration. It propagates through every
derivation, and `environment "NAME"` is always secret whether the word is
written or not (`E-IFC-02` if you leave it off).

```zd
secret state apiKey is server Text from environment "API_KEY"
state   status is server Text from statusFor with key is apiKey

# The secret stops here: `statusFor` returns a constant rather than
# carrying the key onward, so `status` need not be declared `secret`.
function statusFor with key
    give "ok"

view
    when status
        Loading           show Spinner
        Failed with error show ErrorBar message is "the service is unavailable"
        Ready with text   show Text text
```

The sinks, each with its own code and its own explanation:

| code | the sink |
|---|---|
| `E-IFC-01` | a secret declared on a placement that cannot hold one |
| `E-IFC-02` | a secret derivation whose target is not declared `secret` |
| `E-IFC-03` | a secret written into a place that is not secret |
| `E-IFC-05` | a secret would be rendered |
| `E-IFC-06` | a secret would be stored in browser memory |
| `E-IFC-07` | a secret would be baked into the build artefact |
| `E-IFC-08` | a secret would be sent in a response body |
| `E-IFC-09` | a secret would be written to a platform log |
| `E-IFC-10` | a secret would be observable through live sync |
| `E-IFC-11` | a secret would choose where the browser sends a request |
| `E-IFC-13` | a secret is passed to a `client` foreign |

One consequence is easy to trip over and worth stating on its own. If the
endpoint behind a signal read a secret, then **the `Failed` payload of that
signal is worth what the endpoint read** — the host wrote that error text
while holding the secret. So `error.message` is refused there and only a
literal will do:

```zd
secret state apiKey is server Text from environment "API_KEY"
state   status is server Text from statusFor with key is apiKey

function statusFor with key
    give "ok"

view
    when status
        Loading           show Spinner
        Failed with error show ErrorBar message is "the service is unavailable"
        Ready with text   show Text text
```

A signal whose endpoint read no secret has an ordinary failure payload, and
`error.message` is allowed.

### Integrity — `trusted`

`trusted` is a modifier on a `state` declaration, and a `foreign` parameter
may be declared `takes key is trusted Text`. It marks a place whose contents
an untrusted value may not choose.

```zd-rejected
trusted state orders is durable Map of Text to Text starting empty

state candidate is client Text starting ""
state mine      is server Option of Text from orders at candidate

view
    Input candidate, hint is "order id"
```

```
Error: [E-INT-02] this key was chosen by the browser, and the collection it
indexes is `trusted`. Site A1 (§18.1 semantics 8): an index into a `trusted`
place must itself be Trusted.
```

That rejection is IDOR, caught at compile time. Note that a route parameter
declared `in someStaticList` is still Untrusted: matching proves membership
in the operator's enumeration, not provenance — the visitor still chooses
which URL to visit.

A signal nothing writes normally counts as Trusted, because it still holds
the initialiser it was declared with. That premise is false for two
placements, and both are excluded: a `durable` cell may have been written by
a previous deployment, and a `remembered` cell may have been written by a
previous *visit*, by another tab, or by another script on the origin. So a
value cannot be laundered to Trusted by writing it to the browser's store
and reading it back — the read is Untrusted whatever the `starting`
expression says, and `trusted remembered` is `E-INT-01`.

| code | |
|---|---|
| `E-INT-01` | `trusted` on a placement that cannot carry it |
| `E-INT-02` | an untrusted value chose which entry was written |
| `E-INT-03` | an untrusted value was written to a `trusted` place |
| `E-INT-04` | a write happened under an untrusted decision |
| `E-INT-05` | an untrusted argument to a `trusted` foreign parameter |

### Declassification — `release`

`release` is the construct for a bounded disclosure, and its integrity rules
are enforced: `E-REL-04` (a release body read a signal), `E-REL-08` (an
unendorsed argument the compiler cannot trace to a grant), `E-REL-10` (a
release body reached a foreign declaring neither `pure` nor `trusted`), and
`W-REL-01` (a release with no `limit`).

What it does **not** yet do is in [§15](#15-not-implemented).

---

## 12. Build capabilities

Four expressions exist only in a `static` placement, and asking for one
outside the build is `E0361`:

- `build list DIRECTORY` — the files in a directory, sorted, so a build is
  reproducible across machines.
- `build read PATH` — a file's contents as `Text`.
- `build markdown TEXT` — `Markup`, which `Prose` renders and `Text` does
  not.
- `build parts TEXT` — one document as a `List of Part`, split at the
  fences that name a widget. See [§12.1](#121-a-post-that-names-a-component).

Every path is resolved against the project directory before it is opened: a
build reads the project it is building and nothing else.

```zd
record Page
    path is Text
    body is Markup

state pages is static List of Page from readPages with directory is "content"

function readPages with directory
    from build list directory
    map each path to pageFrom with path

function pageFrom with path
    give Page with path is path, body is build markdown (build read path)

view
    Column
        each page in pages
            Column padding is 8
                Text page.path
                Prose page.body
```

Because no boundary is crossed at runtime, `pages` is `List of Page` and not
`Remote of List of Page`.

`emitting` writes a generated file into the bundle; writing one from
something that is not text is `E0315`, and writing one outside the bundle is
`E0316`.

### 12.1 A post that names a component

`Prose` renders one `Markup` and has no children, because interleaving parsed
nodes with templated ones would make the sibling offsets every binding is
scheduled against depend on how many nodes a *file* parsed into. So a post
that wants an interactive chart in the middle of it is not one node. It is a
list, and `build parts` is what makes one:

```
record Part
    markup   is Markup   # the prose run; empty for a widget
    widget   is Text     # the widget's name; "" for a prose run
    argument is Text     # what the file passed it
```

`Part` is declared in the prelude. A file names a widget with a fence whose
info string begins with `zd`:

    ```zd RingChart
    slug: spacetrader-wars
    ```

Every other fence stays a code block, and every run of prose is rendered by
the same pass `build markdown` uses — a `<script>` in a post is shown rather
than run here too.

**The widget set is closed and the program declares it**, as a `choice`
named `Widget`:

```zd
choice Widget
    RingChart
    StackBars
```

A document naming anything else is a failed build (`E11`), naming the widget
and listing the ones on offer. A component cannot be resolved from a file's
text — components are resolved statically and a name out of a `.md` is not a
name the compiler saw — so the program dispatches on the name itself:

```zd
each part in post.parts
    if isBlank of part.widget
        Prose part.markup
    if not (isBlank of part.widget)
        PostWidget part.widget, part.argument
```

What this does **not** do is let a post pass a *value* to a widget. The
argument is `Text` the widget reads, which is what a `slug:` line already
implies. `examples/parts.zd` is the whole of it, running.

---

## 13. Diagnostics

Every rule-bearing diagnostic has a code, and `zdc explain CODE` prints the
rule behind it in full — what it means, why the rule exists, and a rejected
and an accepted example. 73 are written out: 70 errors and three warnings.

The families, in pipeline order:

| prefix | |
|---|---|
| `E01xx` | syntax and layout |
| `E02xx` | types |
| `E03xx` | placement and the signal graph |
| `E-IFC-xx` | secrecy |
| `E-INT-xx` | integrity |
| `E-REL-xx` | release |
| `E-URL-01` | a URL whose scheme executes rather than fetches |
| `W03xx`, `W-REL-01` | warnings |

The explanations are hand-written rather than generated, because hand-written
expert explanations were measured to beat both conventional compiler messages
and generated ones on time-to-fix and on satisfaction.

The type family lands with issue #148. Its decades group the way the
placement family's do: `E020x` is inference proper, `E021x` is `when`,
`E022x` is names, calls and construction, `E023x` pipelines, `E024x` writes,
`E025x` reads across a placement boundary, `E026x` the view, `E027x` the
JavaScript boundary, `E028x` a function's own shape, and `E029x` events.

**Two kinds of type diagnostic still have no code.** The routing rules in
`crates/zdc-types/src/routing.rs` — the URL bijection, route-parameter
enumerability, `Link` written as a literal URL, and the immutability of a
signal initialised from `address` — are a family about URLs rather than
about types, and are the stated remainder of #148. So is the one message
that reports the compiler's own prelude is incomplete, which is not a rule a
program can break.

Warnings are not yet separated from errors by level — issue #154.
Diagnostics are not yet available as JSON (issue #152), and the parser stops
at the first error rather than recovering (issue #151).

---

## 14. The wire format

A value that crosses between the browser and a server function travels as
JSON with one addition: a `Map of K to V` rides as `{"$map": [[key, value],
…]}`, because `JSON.stringify` turns a `Map` into `{}` and says nothing.
`Whole`, `Decimal`, `Text` and `Truth` are their JSON counterparts, a `List
of T` is an array, a `record` is an object keyed by field name, and a
`choice` is `{tag, fields}`. `$` cannot begin a ZD identifier, so no record
field can ever be spelled `$map`.

### The compatibility rule

**No compatibility is promised between wire format versions. A mismatch is
refused, and the refusal names both versions.**

Not "new servers accept old clients", not "old clients accept new servers":
an end that speaks version N reads bytes written by version N and refuses
everything else.

This is not a hypothetical. During any rolling deploy the client is a page
already loaded in a browser and the server has just been replaced underneath
it, so for the length of the deploy — and for as long afterwards as somebody
leaves a tab open — the two ends are built by different compilers. That is
normal operation, not an edge case.

The reason the answer is "refuse" rather than "cope" is an asymmetry in what
a decoder can notice. A *malformed* `$map` fails loudly, because this version
knows what a `$map` should look like:

```text
{"$map": "pairs"}   →   A map on the wire is an array of [key, value] pairs.
```

A marker from a *later* version cannot fail at all. It is a well-formed
object with an unfamiliar field name, so it decodes as an ordinary record,
re-encodes as itself, and reaches the program as `Ready` holding a value
nobody wrote:

```text
{"$set": [1, 2]}    →   a record with one field named $set
```

Refusing that would require knowing `$set` was ever meant to be a marker,
which is exactly the knowledge the older end does not have. So the choice is
not between refusing and coping. It is between refusing, and silently
inventing values whenever the two ends disagree.

### The version is the format's, not the compiler's

The number changes when the bytes change — a new marker, a retired one, a
different shape for one of the types above. It does not change when the
compiler's version does. `zdc 0.1.0` and `0.1.1` write identical bytes, and
a rule that refused across them would break every redeploy and buy nothing.
Most releases will not move it, which is what makes refusing affordable.

### Where the version rides

Beside the bytes, not inside them:

| transport | how |
|---|---|
| a call to a server function | a `zd-wire` header, on the request and on the response |
| the live-sync stream and poll | a `wire=` query parameter |

It is not an envelope around the value, because `stringify` and `parse` are
also the **persistence** format — a `durable` key is written to the store
with the same two functions. Wrapping the value would version every stored
value and rewrite every store on upgrade, which is a different question
(#37's migration story) and not this one's to answer.

The stream carries it in the query rather than a header because `EventSource`
cannot set one. The poll spells it identically, so the two remain one
protocol at two stream lengths.

The cost is twelve bytes on a request, twelve on the response, and seven once
per subscription — never per event.

### What a program sees

`Failed`, with `code` of `Rejected` and a `message` naming both versions:

```text
greeting was called in wire format 2 and this server reads 1.
The page was built by a different compiler; reload it.
```

`Rejected` and not a fourth `Code`. [`Code`](#4-types) is closed at three,
and each of the three is decided by the client's own control flow rather than
by anything a server sends — that is what lets a program match on it without
the answer being steerable by whoever it is talking to. A version a server
picks by writing a header is a value the server chooses, so a code of its own
would put a fourth arm under a server's control. The detail goes in
`message` instead, which is host text and carries the label to match.

A request naming **no** version is the same refusal. An end that does not
name the format was built before the format had one, which makes it a
different format; treating silence as agreement would leave the rule open in
the one case guaranteed to occur, which is a rollback to a build from before
the version existed.

The one exception is deliberate: `zdc dev`'s own reload channel carries no ZD
value, so it neither names a version nor is refused for lacking one.

### What is *not* versioned by this

The live-sync **event protocol** — the event names, the sequence numbers,
`resync` — is separately and deliberately forward-compatible: a client
ignores an event it does not recognise rather than failing, so a stale page
across a deploy learns nothing new instead of breaking. That is safe for an
event name and unsafe for a value, and the difference is the whole of the
rule above: you can ignore an event you do not understand, because you know
you did not understand it. You cannot ignore a value you have misread,
because you do not know you misread it.

---

## 15. Not implemented

Most of these parse. That is a deliberate choice — the grammar is settled
ahead of the semantics — and it means the compiler can tell you precisely
what is missing instead of failing to read the line at all.

The exception is the last one. `durable per visitor` is refused **at** the
parser rather than past it, because the others are constructs waiting on
work and that one is a construct the language has decided not to have. A
grammar settled ahead of its semantics is a promise that the semantics are
coming, and admitting a placement whose meaning cannot be delivered would
be making that promise in bad faith.

**`record … unique`** — identity keys for lists. Refused past the parser:

```
Error: `Row` declares `id` as its identity, and `unique` is not implemented
past the parser yet (#2). Removing the word compiles, and reconciles by
position.
```

Until it lands, identity-keyed reordering is O(n) moves — issue #207.

**`state … takes`** — the externally-initiated effect construct:

```
Error: `ticks` declares an effect with `takes`, and that construct is not
implemented past the parser yet (§14G.8 item 14).
```

This is issue #211, and it blocks four other designs. The related `every` and
`inbound` signal initialisers are issue #18.

**`sessionStorage`** — no placement. `remembered` is `localStorage`, and
the tab-scoped store has none. The survey that motivated `remembered` found
`sessionStorage` used six times against thirty for `localStorage`, and every
one of those six held half of an OAuth exchange — which is a `secret`, which
no browser store may hold. A second placement whose rules would be identical
to `remembered`'s is not obviously worth a second word until something needs
it that is not a secret.

**Observers** — `IntersectionObserver`, `ResizeObserver` and
`MutationObserver` have no construct. They are per *element* rather than per
program, so they want an event on a view node rather than a signal, and that
is the `on` grammar rather than this one. Issue #19 tracks them with the
frame loop and timers.

**Reading a chosen file** — `FileInput` exists and yields an `Option of
Text`: the *name* of the file a reader picked, and nothing else about it.
There is no `File` type, no `Bytes` type, and no upload path.

The placement rules are `Text`'s, unchanged, and that is the reason the type
is what it is: **no file is ever a value, so nothing new crosses a
boundary.** The binding is §14B.5's — a `client` or `remembered` signal
declared `starting` — a name may travel to a server because it is text, and
the name is untrusted, because whoever made the file chose it. A `Handle`
would be the honest type for what the browser hands a script and cannot be
used here: `E0317` admits one in state only in a `client` signal acquired
once and never written, and a picker writes its signal every time somebody
chooses.

What is therefore missing: the bytes, the size, the media type, the
last-modified time, choosing more than one file, and any way to send a file
anywhere. `accept is "image/*"` narrows the browser's dialog and guarantees
nothing about what arrives. A program can pre-fill nothing — no script may
put a file into a picker — so the write half of the binding is a clear:
`None` empties the control and a `Some` the program invented names a file
the control has never held.

**Mutation through a path on a non-`durable` place** — see
[§8](#8-statements). `set m at k to v` compiles for a `durable` map and is
refused for a `client` one, because the runtime has no immutable-update
helper and §14B.3 has not settled.

**`release` does not declassify.** The construct parses, typechecks, and its
integrity rules fire — but the *secrecy* lattice does not treat it as a
declassifier. A `secret` value routed through a release still cannot reach
the browser: you get `E-IFC-02` at the derivation, or `E-IFC-05` and
`E-IFC-08` if the result is declared `secret`. So a release today is a
bounded, audited, integrity-checked computation, and not yet an escape hatch
for a secret. The open work is issues #26 (the non-interference proof), #29
(nothing bounds cumulative disclosure), #30 and #31.

**Combining two `Remote of T`s** — issue #20.

**Programmatic navigation** — not expressible in v1; navigate with a `Link`.

**Queries over `durable`** — it is key-value; joins and aggregation are issue
#36, and there is no migration story at all (issue #37).

**A public API surface** — there is none, and no second client; issue #38.

**Per-visitor durable state, and cross-visitor confidentiality** — not open
work, and not coming in v1. `durable per visitor` is refused by `E0107`.
Partitioning storage by principal requires a principal, and the language has
no way to establish one: a request here is an endpoint name and a JSON array
of arguments, with no headers, no cookies and no session. Minting an identity
means minting and checking a credential, which is authentication, which is a
v1 non-goal in the same breath as per-user durable scoping — the two are
listed together because they are one problem.

The consequence is the limitation to read twice: **one visitor reading
another visitor's durable row is a leak that compiles.** The confidentiality
lattice (`secret`) and the integrity lattice (`trusted`) are both over
*placements*, not over principals, so neither has anything to say about which
visitor a value belongs to. A durable row is visible to every request that
computes its key, filtering by owner is the program's job, and no pass checks
that the filter is there. Do not reach production believing otherwise.

---

## See also

- [The tutorial](tutorial.md) — one program, five steps.
- [The examples](../examples) — twenty-eight programs, each commented with
  what it demonstrates and what it could not have.
- [`ROADMAP.md`](../ROADMAP.md) and the issue tracker — the remaining work
  lives in the issues, indexed by #35.
