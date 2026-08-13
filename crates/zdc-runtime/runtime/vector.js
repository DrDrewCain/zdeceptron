// `templateSvg` — the one thing about drawing the HTML parser gets wrong.
//
// # The bug this exists to prevent
//
// An element's namespace is decided by *where it appears*. Inside an
// `<svg>`, `<path>` is an `SVGPathElement`; on its own it is an HTML
// element that happens to be called `path`. The second one has no
// geometry, paints nothing, and serialises identically to the first —
// `outerHTML` cannot tell them apart, a DOM dump cannot, and no error is
// reported anywhere. The drawing is simply missing.
//
// A whole `<svg>…</svg>` subtree parses correctly with no help, so this
// is not needed for the common case. What needs it is a *row*: `each ring
// in rings` emits a template of one `<path d="…">`, cloned per item, and
// that fragment reaches the parser with nothing around it.
//
// Parsing it inside an `<svg>` and lifting the children out gives them
// the namespace, which they keep when adopted anywhere else.
//
// # Why it is not a flag on `template`
//
// `dom.js` ships to every program there is, including the one that draws
// nothing, and `zdc-bench`'s size gate makes that a build failure rather
// than an opinion — a null program has a byte ceiling and this branch
// would come out of it. So it is its own module, linked only when the
// emitter actually flags a fragment. `list.js`, `markup.js` and
// `foreign.js` are each split on exactly this rule.

// The string is the emitter's own markup, escaped where a value reached
// it (spec §16.3.4) — the same guarantee `template` in `dom.js` rests on,
// and the reason neither parses anything a program wrote directly.
export function templateSvg(html) {
  let content;
  return () => {
    if (content === undefined) {
      const element = document.createElement('template');
      element.innerHTML = `<svg>${html}</svg>`;
      content = document.createDocumentFragment();
      content.append(...element.content.firstChild.childNodes);
    }
    return content.cloneNode(true);
  };
}
