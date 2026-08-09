# A tutorial

You will build one program: a shelf of books you mean to read. It starts as
a page in a browser and ends as a page in a browser — but along the way one
word changes, and that word moves the shelf out of the tab and into a store
that outlives it, generates the endpoint the browser now needs, and makes
the compiler tell you about every consequence you did not think through.

That is the point of the language, and it is the point of this tutorial.

Everything below has been compiled. If you paste a step into a file and the
compiler disagrees with the text, the text is wrong — please open an issue.

## Before you start

Build the compiler once:

```
cargo build --release -p zdc-cli
```

That gives you `./target/release/zdc`. Two commands matter here:

```
zdc check shelf.zd    # parse, resolve, typecheck — says nothing when it is happy
zdc dev   shelf.zd    # serve it, rebuilding as you edit
```

`zdc check` printing nothing is the success case. Every error it does print
carries a code, and `zdc explain E0101` prints the rule behind the code in
full, with a rejected and an accepted example.

---

## Step 1 — a signal and a view

Put this in `shelf.zd`:

```zd
state title is client Text starting ""

view
    Column
        Heading "Shelf"
        Input title, hint is "a book you mean to read"
        Text title
```

`zdc check shelf.zd` says nothing, and `zdc dev shelf.zd` gives you a page
where typing in the box changes the line below it.

Three things are already true and worth naming.

**`state` is the only way to declare a value that changes.** There is no
second mechanism — no store, no context, no hook. A `state` declaration is
called a *signal*.

**Every signal says where it lives.** That is the word `client` on line one.
It is not optional and the compiler will not guess it: `client` means one
value per open tab, and the other three (`static`, `server`, `durable`) are
other machines entirely. Leave it out and you get `E0101`, which exists
because choosing on your behalf would produce a program that runs and is
wrong.

**The view is a function of the signals, and you never say so.** Nothing
above subscribes `Text title` to `title`, and nothing re-renders it. The
compiler read the dependency out of the program and wired it.

---

## Step 2 — a record and a list

One text box is not a shelf. Give a book a shape, keep a list of them, and
add to the list from a handler:

```zd
record Book
    title is Text
    read  is Truth

state books is client List of Book starting empty
state draft is client Text starting ""

view
    Column
        Heading "Shelf"

        Row
            Input draft, hint is "a book you mean to read"
            Button "add"
                on click
                    append (Book with title is draft, read is no) to books
                    set draft to ""

        each book in books
            Row
                Text book.title
                Button "remove"
                    on click
                        remove book from books
```

`record Book` declares a product type; `Book with title is draft, read is no`
builds one, naming every field. `Truth` is the boolean type and its values
are `yes` and `no`.

`each book in books` is a region of the view rather than a loop in a
handler: it is a description of what the page contains for each element, and
`book` is in scope inside it — including inside the `on click` on the button,
which is how `remove book from books` knows which book.

The handler statements are the whole mutation vocabulary you need so far:
`set`, `append`, `remove`. There is no assignment operator and no method
call; a mutation names the verb, the value and the place.

---

## Step 3 — a derived signal

The shelf should tell you how much of it you have not read. Do not compute
that in the view, and do not compute it in a handler. Declare it:

```zd
record Book
    title is Text
    read  is Truth

state books is client List of Book starting empty
state draft is client Text starting ""

state unread is client List of Book from unreadOf with books
state left   is client Whole        from listLength of unread

function unreadOf with all
    from all
    keep each book where not book.read

view
    Column
        Heading "Shelf"

        Row
            Input draft, hint is "a book you mean to read"
            Button "add"
                on click
                    append (Book with title is draft, read is no) to books
                    set draft to ""

        Row
            Text "still to read: "
            Text left

        each book in unread
            Row
                Text book.title
                Button "remove"
                    on click
                        remove book from books
```

A signal declared with `from` is *derived*: it has no initial value and
nothing may write to it. `unread` is whatever `unreadOf with books` says it
is, at every moment, and `left` is derived from `unread` in turn. Writing to
either is a compile error rather than a bug.

`unreadOf` is written as a **pipeline**: `from` names the collection and each
following clause transforms it. `keep each book where …` is the filter. The
clauses are a closed set in a fixed order — `from`, then `keep`/`sort`/`map`
in any number and order, then `take first` — and that order is the grammar,
not a convention.

Note the two `Text` nodes on the "still to read" row. There is no string
interpolation in ZDeceptron. Text that is part of the layout is a node in the
layout.

---

## Step 4 — one word

Everything so far lives in the tab. Close it and the shelf is gone. So make
the shelf outlive the tab: change `client` to `durable` on the `books` line,
and change nothing else.

```
state books is durable List of Book starting empty
```

`zdc check` now says:

```
Error: `all` of `unreadOf` is `Remote of List of Book`, but `List of a type
that is not known here` is expected here.
   ╭─[shelf.zd:8:56]
   │
 8 │ state unread is client List of Book from unreadOf with books
   │                                                        ──┬──
   │                                                          ╰────
───╯
```

This is the tutorial's point, so read the message slowly.

`books` is now in a store on the other side of a network. `unread` is still
declared `client`, so it is computed in the browser, and a browser that wants
`books` has to ask for it. Asking can be slow and asking can fail. So the
type of `books` *as seen from the client* is no longer `List of Book`: it is
`Remote of List of Book` — a value that is loading, or failed, or ready.

The compiler did not insert a spinner, cache the result, or quietly await
anything. It changed the type, and the type is now wrong for a function that
wanted a list. Every consequence of moving that data is in front of you at
the moment you moved it.

There are two honest answers. The first is to handle the three states in the
view. The second is to move the derivation to where the data already is —
which is one word again, on two lines:

```zd
record Book
    title is Text
    read  is Truth

state books is durable List of Book starting empty
state draft is client Text starting ""

state unread is server List of Book from unreadOf with books
state left   is server Whole        from listLength of unread

function unreadOf with all
    from all
    keep each book where not book.read

view
    Column
        Heading "Shelf"

        Row
            Input draft, hint is "a book you mean to read"
            Button "add"
                on click
                    append (Book with title is draft, read is no) to books
                    set draft to ""

        Row
            Text "still to read: "
            when left
                Loading           show Spinner
                Failed with error show ErrorBar message is error.message
                Ready with count  show Text count

        when unread
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with list
                each book in list
                    Row
                        Text book.title
                        Button "remove"
                            on click
                                remove book from books
```

`unread` and `left` are now `server`: they run in a serverless invocation
next to the store, where `books` is an ordinary list and the filter is an
ordinary filter. The view reads them from the browser, so *there* they are
`Remote`, and `when` is how you spend one — three arms, all three required.

Count what you did not write. There is no endpoint, no route table, no
serialiser, no fetch, no `useEffect`, no loading flag, no error state, no
cache key. The compiler generated an endpoint for `unread` and one for
`left` because something on the client reads them, and it will warn you
(`W0330`) if you declare a `server` signal that nothing reads, because then
there is no endpoint to generate and the declaration is a mistake.

`append … to books` in the handler still reads the same. It is a write to a
durable store now, from a browser, through generated machinery — and the
line did not change, because the line was never about transport.

---

## Step 5 — a secret

Last step: the shelf gets an opinion, and the opinion comes from a service
that needs a key. The key must never reach the browser. Say so, and let the
compiler enforce it:

```zd
record Book
    title is Text
    read  is Truth

secret state apiKey is server Text from environment "SHELF_API_KEY"

state books is durable List of Book starting empty
state draft is client Text starting ""

state unread is server List of Book from unreadOf with books
state left   is server Whole        from listLength of unread

state advice is server Text from adviceFor with left, apiKey

function unreadOf with all
    from all
    keep each book where not book.read

function adviceFor with count, key
    if count is 0
        give "Nothing waiting. Add something."
    if count > 5
        give "That is a queue, not a shelf."
    give "A reasonable shelf."

view
    Column
        Heading "Shelf"

        Row
            Input draft, hint is "a book you mean to read"
            Button "add"
                on click
                    append (Book with title is draft, read is no) to books
                    set draft to ""

        Row
            Text "still to read: "
            when left
                Loading           show Spinner
                Failed with error show ErrorBar message is error.message
                Ready with count  show Text count

        when advice
            Loading show Spinner
            Failed with error
                when error.code
                    Unreachable show ErrorBar message is "no answer from the shelf service"
                    Timeout     show ErrorBar message is "the shelf service took too long"
                    Rejected    show ErrorBar message is "the shelf service said no"
            Ready with words show Text words

        when unread
            Loading           show Spinner
            Failed with error show ErrorBar message is error.message
            Ready with list
                each book in list
                    Row
                        Text book.title
                        Button "remove"
                            on click
                                remove book from books
```

`secret` is a modifier on the declaration and it propagates. Try to derive
anything from `apiKey` without saying `secret`, and you get `E-IFC-02`; get a
secret to the view and you get `E-IFC-05`; put one in a response body and you
get `E-IFC-08`. `advice` is allowed to reach the browser because
`adviceFor` does not carry the key onward — it returns one of three
sentences.

Notice the failure arm for `advice`. `error.code` is a closed choice of
three — `Unreachable`, `Timeout`, `Rejected` — and matching on it is how you
say something useful about a failure. Notice also that those three messages
are literals. Because the endpoint behind `advice` read a secret, the
compiler will not let `error.message` reach the browser there either: the
host wrote that text while holding the key.

---

## What you built

Five steps, one program, and the diff that mattered was one word. What the
word changed:

| | step 3 | step 4 |
|---|---|---|
| `books` lives | in the tab | in a durable store |
| `unread` runs | in the tab | in a serverless invocation |
| the view sees `unread` as | `List of Book` | `Remote of List of Book` |
| endpoints you wrote | — | — |
| endpoints that exist | 0 | 2 |

The compiler's job in this language is to make the second column's costs
visible at the moment you ask for them, in the type, rather than at three in
the morning in production.

## Where to go next

- [The language reference](reference.md) — every declaration, every
  placement, the type system, pipelines, events, and the information-flow
  rules, systematically.
- [The examples](../examples) — twenty-seven programs, each one commented
  with what it exists to demonstrate and what it could not have.
- `zdc explain <code>` — the rule behind any diagnostic you hit, with a
  rejected and an accepted example.
