# `widgets/` — composed widgets, written in ZDeceptron

Sixteen components — fourteen that a program names, two that a module
instantiates for itself — and two records, in nine modules, built from
nothing but the built-in elements — 68 of them, counted from
`BuiltinElement::NAMES`, where #241's title says 65. These are ordinary
`component` declarations that a program imports with `use`, exactly as
`examples/blog.zd` imports `examples/layout.zd`.

An earlier revision of this file said no compiler support was added and
none was needed. The first half was true and the second was not: the
`aria-*` arguments described at the bottom of this document landed
because of what is written here, and `tabs.zd`, `breadcrumbs.zd`,
`pagination.zd`, `search.zd` and the new `toggle.zd` are what they are
because of it.

**They ship no appearance.** §6.1's answer to how something looks is
`class is "…"`, so every widget takes a `class` argument — several take two
or three, one per element that a caller might need to reach — and none of
them writes a colour, a size, a border or a spacing. A widget set that
shipped a look would compete with Tailwind instead of composing with it.

## Using them

```zd
use "./card" for Card, Badge, EmptyState

view
    Card "Issues", class is "rounded border p-4"
        Badge "open", class is "text-xs"
        Paragraph "Six of them."
```

⚠️ **The modules must sit inside your program's own directory.** The
project is the directory holding the file the build started from, and `use`
reaches files under it and nowhere else — `use "../widgets/card"` is
refused with *"names a file that climbs out of the project"*. So this
directory is not yet a library anyone can depend on from where their
program already lives; it is a directory you copy next to your program.
That is the honest state of "beside the prelude rather than in it", and it
is the first thing #241's question 1 runs into.

`widgets/dashboard.zd` is the worked example. It composes all twelve of the
named widgets into an issue tracker and is the reason each of them is known to render
rather than merely to typecheck.

---

## What each one is

### `card.zd` — `Card`, `Badge`, `EmptyState`

| widget | takes | what it is |
| --- | --- | --- |
| `Card` | `title`, `class`, *children* | A `Section` with a `Heading`. |
| `Badge` | `label`, `class` | A `Text`. |
| `EmptyState` | `title`, `detail`, `class`, *children* | Heading, explanation, and the way out as children. |

`Card` is a `Section` and not a `Column`, which is the whole reason it
exists: `Section` is a sectioning element, so the heading inside it is one
level deeper than the heading outside it, chosen by nesting. A card built
by hand from a `Column` and a `Heading` puts a second `h1` on the page.

`Badge` is one element deep on purpose. What a badge gets wrong is never
its markup — it is that the meaning lives in the colour. `label` is
required, so the meaning is in the text.

**Does not do:** `Card` has no untitled form. A box with no heading is
`Column class is "…"` and needs no widget; a `Card` with an empty title
would put an empty heading in the document outline.

### `accordion.zd` — `Accordion`, `AccordionSection`

| widget | takes | what it is |
| --- | --- | --- |
| `Accordion` | `class`, *children* | A `Column` around the sections. |
| `AccordionSection` | `title`, `class`, *children* | A `Details` with a `Summary`. |

Complete, because the browser owns it. A `details` is keyboard-operable
with no handler, announced as expanded or collapsed, expanded by
find-in-page, and printed open — and the user agent maintains
`aria-expanded` on the summary, so nothing here has to. The vocabulary now
has an `expanded` argument for a disclosure the *program* renders; this
widget still does not use it, because writing a second `aria-expanded`
beside the browser's own is how the two come to disagree.

**Does not do:** exclusivity. Opening one section does not close the
others. HTML's answer is the `name` attribute on `details`, which is not in
the vocabulary's closed argument set, so there is no way to ask for it.

### `tabs.zd` — `Tabs`, `Tab`, `TabPanel`

| widget | takes |
| --- | --- |
| `Tabs` | `labels` (`List of Text`), `selected` (a `Whole` signal), `group`, `class`, `tabClass`, `currentClass` |
| `TabPanel` | `index`, `selected`, `group`, `class`, *children* |

**This is an ARIA tablist, and it now says so.** `role="tablist"`,
`role="tab"` with `aria-selected` on every tab — `false` on the closed
ones, which is the half that matters — `aria-controls` from the open tab
to its panel, and `role="tabpanel"` with `aria-labelledby` pointing back.
Six of the pattern's seven attributes.

`group` is the caller's word, exactly as `SearchField`'s `field` is, and
every `id` in the pair is derived from it. Nothing in the language gives a
component instance an identity, so a widget that generated its own would
put two tab sets on one page under one id.

`aria-controls` is written **only on the open tab**, because `TabPanel`
renders nothing when its panel is closed and a reference to a removed
element is one a reader is invited to follow to nothing.

**Does not do:** the roving `tabindex`, and therefore the arrow keys. The
strip is one tab stop per tab rather than one in total. `tabindex` is not
in the closed argument set and nothing in the language moves focus, so an
arrow key could only move *selection*, leaving the focused button and the
open panel disagreeing — and a keyboard user then presses Enter and
watches the selection jump back. That is not a partial implementation of
the pattern, it is a different and worse one. The panel is not focusable
either, for the same reason.

### `toast.zd` — `Toast`

Takes `message` (a `Text` signal — empty means nothing to say), `class`,
`messageClass`.

**The one thing it gets right that a hand-rolled one does not:** the
`role="status"` live region is in the document from mount, and only the
message inside it is conditional. A region created already holding its text
has not *changed*, and most screen readers say nothing. The obvious
spelling — `if message is not "" ` wrapped *around* the region — renders
correctly, passes every check, and is silent.

**Does not do:** dismiss itself. Auto-dismiss needs a timer and there is
none. Dismissal is a `Button`, which is what an accessibility guideline
asks for anyway; what is missing is the choice.

### `breadcrumbs.zd` — `Breadcrumbs`, `record Crumb`

Takes `trail` (`List of Crumb`, each `label` and `href`), `here`, `class`,
`linkClass`, `currentClass`.

A `Navigation` landmark named by `aria-label`, holding a `NumberedList` —
the order is the hierarchy, and a screen reader reads the position out. The
last step is a `Text`, not a `Link`, because a link to the page you are on
is a control that does nothing, and it carries `aria-current="page"` so
that a reader who cannot see which entry is not a link is told which one it
is.

This used to be named by `title`, which reaches the accessible name by the
computation's fallback route and draws a tooltip nobody asked for.

### `pagination.zd` — `Pagination`, `PageNumber`

Takes `page` (a `Whole` signal, counted from zero), `count`, `class`,
`pageClass`, `currentClass`.

*Previous* on the first page is present and announced unavailable, by
`aria-disabled`. It used to be absent: the choice was between a control
that vanishes and a live one that silently does nothing, and a tab stop
leading nowhere is worse than a missing control. `aria-disabled` is the
third answer — the button stays in the document and in the tab order, a
reader who arrives at it is told why it does nothing, and the row does not
reflow as you page. The branch that carries it has no `on click` under it,
so it is inert because it has no behaviour rather than because something
suppressed one. It is **not** HTML's `disabled`, which would take the
control out of the tab order and its announcement with it.

The current number carries `aria-current="page"`, and the landmark is named
by `aria-label`.

**Does not do:** ellipsis for long ranges. Every page gets a number.

### `menu.zd` — `NavMenu`, `record MenuLink`

Takes `label`, `links` (`List of MenuLink`), `class`, `linkClass`.

The W3C **disclosure navigation** pattern: a `Details` holding a `List` of
`Link`s. Complete, because the browser owns all of it, and the pattern does
not ask for arrow keys — Tab through a short list of links is the specified
behaviour. Closing on select is moot: following a link ends this program
instance and begins another (§14G.2).

**Does not do:** it is not an action menu. See below.

### `search.zd` — `SearchField`

Takes `query` (a `Text` signal), `label`, `field` (the `id`), `class`,
`inputClass`.

Takes `note` too, and the note is the reason this section changed.

The one keyboard behaviour in this directory that is the program's rather
than the browser's: **Escape clears the field.** It works because the
keystroke arrives at an element that is already focused, so no focus has to
move — which is the entire intersection between what the ARIA patterns ask
for and what this language can do.

That behaviour is **invisible**. A reader looking at the box cannot see it
and a reader listening to the box was never told. `note` is a `Small` tied
to the field by `aria-describedby`, so a screen reader reads the name, then
the value, then the note — which puts "Escape clears this" before the
reader starts typing rather than after.

`field` is the caller's because an `id` must be unique in a document and a
component can be instantiated twice. Nothing in the language gives an
instance an identity, so a widget that generated its own id would produce
two fields with one id and a `Label` pointing at whichever came first.

### `toggle.zd` — `Switch`, `ToggleButton`

| widget | takes | what it is |
| --- | --- | --- |
| `Switch` | `label`, `setting` (a `Truth` signal), `class` | A `Button` with `role="switch"` and `aria-checked`. |
| `ToggleButton` | `label`, `down` (a `Truth` signal), `class` | A `Button` with `aria-pressed`. |

**The only two widgets here that give up nothing**, and neither existed
before `aria-checked` and `aria-pressed` were spellable. A `button` is
focusable, is in the tab order, is operated by Enter and by Space, and
needs no handler for any of that; nothing has to move focus, and the
keystroke arrives at the control that is already focused. The one thing
missing was a way to say what state the button is *in*, and a button that
toggles something without announcing whether it is on is a control a
reader can operate and never read.

Two and not one because they are announced differently and mean different
things. A switch is a setting that takes effect immediately;
`aria-pressed` is a button that stays down, where what changed is the view
rather than a setting. Bold in a toolbar is pressed, "Notify me" is
checked. §4.1's rule against two ways to write one thing is untouched,
because these are two things.

The signal is the whole state — the control reads it to announce itself
and writes it when pressed — so there is no way for what is shown and what
is held to disagree.

**Does not do:** nothing. There is no focus to move, no second element to
point at, and no keyboard behaviour the browser does not already supply.

---

## What is not here, and exactly what was missing

Two of the widgets #241 asks for are absent. Neither is hard; both are
impossible, and the reasons are worth more than a broken version.

### `Modal` — not expressible

A modal dialog is defined by what it does to focus. All four parts are
unreachable:

1. **Move focus into the dialog when it opens.** There is no statement that
   calls a method on a node. `zdc check` on `focus "id"` reports
   *"`focus` cannot begin a statement"* (E0104) and lists the fifteen words
   a statement can begin with — `from` `keep` `sort` `map` `take` `set`
   `add` `subtract` `append` `remove` `give` `with` `when` `each` `if`.
   None of them is a call for effect.
2. **Trap focus inside it.** Needs `tabindex` on the container and a
   `keydown` handler that redirects Tab. `tabindex` is not in the closed
   argument set — `Button "x", tabindex is 0` is refused — and redirecting
   Tab needs the same missing focus call.
3. **Restore focus to the trigger on close.** Same missing call.
4. **Announce it as a dialog.** `role is "dialog"` is spellable and
   `aria-modal` is not: it is deliberately absent from the ARIA table,
   because it is an attribute of a widget that owns focus and nothing here
   moves focus. A `role="dialog"` with no `aria-modal` and no focus move
   is a region a screen reader user never enters. This is the part of
   Modal the `aria-*` arguments did **not** unblock, and the reason is the
   same one that blocks the other three.

There is also no `dialog` element in the 68-name vocabulary, so the native
top-layer, the backdrop and `inert` on the rest of the page are all out of
reach, and nothing renders outside its place in the tree — there are no
portals. Scroll lock would need to write `overflow` on the document
element, which no widget can name.

A `Column` with `role="dialog"` inside an `if` compiles. It is not a modal,
and shipping it under that name is how a program ends up believing it has
one.

### `Menu` (the action menu) — not expressible

A button that opens a list of commands needs five things; each is missing
for its own reason:

| behaviour | why not |
| --- | --- |
| `aria-expanded` on the trigger | **now spellable** — `expanded is open` |
| `aria-haspopup` on the trigger | absent from the ARIA table: it names a kind of popup this language cannot build |
| close on select | needs a `Truth` signal and an `if`, which works |
| close on Escape | after a pointer click the focus is the trigger; nothing can move it into the list, and a keystroke only reaches the focused element |
| close on click-outside | `on` attaches to the element it is written under; there is no document-level listener |
| arrow keys within the list | roving `tabindex` plus a focus call |

One of the five is now answered and the other four are not, so this is
still not shippable — a menu that announces itself correctly and cannot be
closed from the keyboard is a trap rather than a partial menu. `NavMenu`
is shipped instead because the disclosure-navigation pattern is genuinely
complete, not because it is a smaller version of this one.

### The gap that used to be under all of it: `aria-*` could not be written

This was the first entry on the list below, and it is done. What it said
was that §16.3.6 makes an argument name a UAX#31 identifier, which admits
**no hyphen**, so `aria-selected` was not merely absent from the closed
argument set but unspellable — and that the consequence was specific:
*this language can express the structure and the state of every widget in
#241's list, and the ARIA half of none of them.*

The answer taken is the first of the two this file proposed: **a table of
named arguments that translate**, the way `expansion` already stands for
`title` and `decoration is "struck"` for `text-decoration-line:
line-through`. Eleven rows, in `crates/zdc-codegen/src/elements.rs`:

| written | reaches | takes |
| --- | --- | --- |
| `selected` | `aria-selected` | a `Truth` |
| `expanded` | `aria-expanded` | a `Truth` |
| `pressed` | `aria-pressed` | a `Truth` |
| `checked` | `aria-checked` | a `Truth` |
| `disabled` | `aria-disabled` | a `Truth` |
| `decorative` | `aria-hidden` | a `Truth` |
| `controls` | `aria-controls`, or `for` on a `Label` | an `id` |
| `describedBy` | `aria-describedby` | an `id` |
| `labelledBy` | `aria-labelledby` | an `id` |
| `current` | `aria-current` | `page`, `step`, `location`, `date` or `time` |
| `live` | `aria-live` | `polite`, `assertive` or `off` |

`label` became global at the same time and reaches `aria-label` on
anything with no text beside it to wrap, which is how the two `Navigation`
landmarks here are named.

**Not the record.** The other proposal was an `aria` argument taking one,
and a record is *open*: `aria is (with modal is yes)` names an attribute
the compiler has never heard of, which is the open attribute set §16.3.6
exists to close, and it arrives as a value rather than as a name so
nothing can check the spelling.

The one thing worth knowing about the table is that an ARIA state is not a
boolean attribute. `setAttribute` implements HTML's booleans — `false`
removes the attribute — and `aria-selected` is an *enumerated* attribute
whose values are the words `true` and `false`. A tab strip whose closed
tabs simply lack the attribute is announced as one with nothing chosen,
and it renders identically; `crates/zdc-codegen/tests/widgets.rs` asserts
both halves for exactly that reason.

Two of the eleven have no use in this directory, and that is worth
recording rather than hiding. `live` is unused because `role="status"` on
`Toast` and `role="alert"` on `ErrorBar` already carry the two manners a
region can interrupt in; `live` is for a region that is neither.
`decorative` is unused because **a library that ships no appearance has
nothing decorative in it** — the arrows and icons `aria-hidden` exists for
are the caller's CSS.

What is still missing, in the order it would unblock the most:

1. **`tabindex`, and something that moves focus.** A statement, or an
   element argument that requests focus when a condition becomes true.
   Without it no roving-tabindex widget and no dialog is reachable, ever.
   This is now the whole of what stands between `tabs.zd` and the ARIA
   tabs pattern, and the whole of what stands between this directory and
   `Modal` and the action `Menu`.
2. **A timer.** `clock` reads the time; nothing schedules anything. Toast
   auto-dismiss and every debounce need one.
3. **`use` that can reach a shared directory.** Until then a widget module
   has to be copied into each program's own project directory.

What was *not* missing, and is worth recording because it was expected to
be: **keyboard event payloads.** `on keydown with press` and `press.key`
exist and work — `widgets/search.zd` uses them. Keystrokes were never the
obstacle. Focus was.

---

## Verified

Every file in this directory compiles. `zdc check` on each exits 0 with no
output, and `widgets/dashboard.zd` — which imports all nine modules —
builds to a bundle. `crates/zdc-codegen/tests/widgets.rs` drives the
rendered tree in the DOM shim and asserts the behaviour rather than the
source: that Escape clears the search field, that the live region is
present before it has a message, that selecting a tab swaps the panel,
that the strip says which tab is chosen **and says of the others that they
are not**, that a switch announces the state it holds and then the one it
changes to, that both landmarks are named and both current positions
announced, and that Previous on the first page is present and announced
unavailable.
