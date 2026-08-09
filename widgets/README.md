# `widgets/` — composed widgets, written in ZDeceptron

Fourteen components — twelve that a program names, two that a module
instantiates for itself — and two records, in eight modules, built from
nothing but the built-in elements — 67 of them, counted from
`BuiltinElement::NAMES`, where #241's title says 65. No compiler support was added and none is needed:
these are ordinary `component` declarations that a program imports with
`use`, exactly as `examples/blog.zd` imports `examples/layout.zd`.

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

The only widget here that is *complete*, because the browser owns it. A
`details` is keyboard-operable with no handler, announced as expanded or
collapsed, expanded by find-in-page, and printed open. This is also the one
route by which anything in this directory gets `aria-expanded` at all — the
user agent maintains it on the summary.

**Does not do:** exclusivity. Opening one section does not close the
others. HTML's answer is the `name` attribute on `details`, which is not in
the vocabulary's closed argument set, so there is no way to ask for it.

### `tabs.zd` — `Tabs`, `Tab`, `TabPanel`

| widget | takes |
| --- | --- |
| `Tabs` | `labels` (`List of Text`), `selected` (a `Whole` signal), `class`, `tabClass`, `currentClass` |
| `TabPanel` | `index`, `selected`, `class`, *children* |

⚠️ **This is not an ARIA tablist and does not claim to be.** It emits a row
of ordinary `Button`s and a region that swaps. Each tab is a real button,
so it is fully operable from the keyboard and accurately announced — as a
button, which is what it is.

**Does not do:** `role="tablist"`, `aria-selected`, `aria-controls`,
roving `tabindex`, or arrow keys. See *What the language is missing* below
for why each is impossible rather than merely absent. Arrow keys are
withheld deliberately: nothing can move focus, so an arrow key could only
move *selection*, leaving the focused button and the open panel
disagreeing. That is worse than no arrow keys.

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

A `Navigation` landmark named by `title`, holding a `NumberedList` — the
order is the hierarchy, and a screen reader reads the position out. The
last step is a `Text`, not a `Link`, because a link to the page you are on
is a control that does nothing.

**Does not do:** `aria-current="page"`. `title` names the landmark by the
accessible-name computation's fallback route rather than by `aria-label`.

### `pagination.zd` — `Pagination`, `PageNumber`

Takes `page` (a `Whole` signal, counted from zero), `count`, `class`,
`pageClass`, `currentClass`.

There is no disabled *Previous*: on the first page no button is rendered at
all. The vocabulary has no `disabled` attribute — only a `disabled` style
*prefix* for colouring a control disabled by some other means — so the
alternative was a live button that silently does nothing, which is a tab
stop leading nowhere.

**Does not do:** `aria-current="page"`, or ellipsis for long ranges. Every
page gets a number.

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

The one keyboard behaviour in this directory that is the program's rather
than the browser's: **Escape clears the field.** It works because the
keystroke arrives at an element that is already focused, so no focus has to
move — which is the entire intersection between what the ARIA patterns ask
for and what this language can do.

`field` is the caller's because an `id` must be unique in a document and a
component can be instantiated twice. Nothing in the language gives an
instance an identity, so a widget that generated its own id would produce
two fields with one id and a `Label` pointing at whichever came first.

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
4. **Announce it as a dialog.** `role is "dialog"` is spellable, and
   `aria-modal` is not (see below). A `role="dialog"` with no `aria-modal`
   and no focus move is a region a screen reader user never enters.

There is also no `dialog` element in the 67-name vocabulary, so the native
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
| `aria-expanded` / `aria-haspopup` on the trigger | unspellable — see below |
| close on select | needs a `Truth` signal and an `if`, which works, but without `aria-expanded` the trigger announces nothing |
| close on Escape | after a pointer click the focus is the trigger; nothing can move it into the list, and a keystroke only reaches the focused element |
| close on click-outside | `on` attaches to the element it is written under; there is no document-level listener |
| arrow keys within the list | roving `tabindex` plus a focus call |

`NavMenu` is shipped instead because the disclosure-navigation pattern is
genuinely complete, not because it is a smaller version of this one.

### The gap under all of it: `aria-*` cannot be written

Every widget above loses something to one cause. §16.3.6 states it
directly: an argument name is a UAX#31 identifier, which admits **no
hyphen**, so `aria-selected` is not merely absent from the closed argument
set — it is not spellable as an argument name at all. `ariaExpanded` is
refused as an unknown argument. What survives is `role`, `id`, `title`,
`lang` and `hidden`, which are the five global attributes the vocabulary
does carry.

The consequence is specific: **this language can express the structure and
the state of every widget in #241's list, and the ARIA half of none of
them.** `role` alone is frequently worse than nothing — `role="tab"` with
no `aria-selected` tells a reader that a control is a tab and never which
one is chosen — so where the pair could not be completed, no role was
written.

The things that would change this, in the order they would unblock the most:

1. **A way to write `aria-*`.** Either a fixed set of named arguments
   (`selected`, `expanded`, `controls`, `describedBy`, `current`), the way
   `expansion` already stands for `title` and `least` for `min`, or an
   `aria` argument taking a record. It is a table entry per attribute, and
   it unblocks Tabs, Pagination, Breadcrumbs and half of Modal.
2. **`tabindex`, and something that moves focus.** A statement, or an
   element argument that requests focus when a condition becomes true.
   Without it no roving-tabindex widget and no dialog is reachable, ever.
3. **A timer.** `clock` reads the time; nothing schedules anything. Toast
   auto-dismiss and every debounce need one.
4. **`use` that can reach a shared directory.** Until then a widget module
   has to be copied into each program's own project directory.

What was *not* missing, and is worth recording because it was expected to
be: **keyboard event payloads.** `on keydown with press` and `press.key`
exist and work — `widgets/search.zd` uses them. Keystrokes were never the
obstacle. Focus was.

---

## Verified

Every file in this directory compiles. `zdc check` on each exits 0 with no
output, and `widgets/dashboard.zd` — which imports all eight modules —
builds to a bundle. `crates/zdc-codegen/tests/widgets.rs` drives the
rendered tree in the DOM shim and asserts the behaviour rather than the
source: that Escape clears the search field, that the live region is
present before it has a message, and that selecting a tab swaps the panel.
