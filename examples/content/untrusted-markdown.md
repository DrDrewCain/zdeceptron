# A post the author of this program did not write

This file is the hostile case, checked in on purpose. `static` state is
read at build time, and "at build time" is a claim about *when*, not about
*who*: a repository's markdown can come from a contributor, a submodule, a
content management system, or a pull request nobody read closely.

So the renderer is not allowed to trust it, and the four constructs below
are the ones that were measured against `pulldown-cmark` and found to pass
straight through to the page.

A raw block:

<script>window.__zdOwned = true;</script>

An inline tag with an event handler:

Text before <img src=x onerror="window.__zdOwned = true"> and text after.

A block-level one:

<div onclick="window.__zdOwned = true">Click me</div>

And the one that contains no HTML at all — ordinary CommonMark link
syntax, which is why turning off inline HTML would not have caught it:

[an innocent-looking link](javascript:window.__zdOwned=true)

Ordinary links must keep working, so: [the placement note](./on-placement.md)
and [an external one](https://example.com/x).
