// DOM rendering for ZDeceptron.
//
// Direct DOM manipulation, no virtual DOM. Every binding is an `effect`,
// so a signal write reaches exactly the text nodes and attributes that
// read it — a component never re-renders as a unit because there are no
// components at this layer, only bindings.
//
// Generated code calls into this module. It is not written by hand and is
// not a user-facing API, which is why it optimises for the code generator
// rather than for ergonomics.

import { effect, batch } from './signal.js';

/** A value that may be a signal getter or a constant. */
function readNode(value) {
  return typeof value === 'function' ? value() : value;
}

/**
 * Create an element with reactive properties and children.
 *
 * `props` values may be getters; each becomes its own effect, so changing
 * one attribute does not touch the others.
 */
export function el(tag, props = {}, children = [], ns) {
  // `ns` is the SVG namespace and nothing else. An element's namespace is
  // not derivable from its tag — `<a>` is both — so the caller states it.
  const node = ns ? document.createElementNS(ns, tag) : document.createElement(tag);

  for (const [name, value] of Object.entries(props)) {
    if (name.startsWith('on')) {
      on(node, name.slice(2).toLowerCase(), value);
    } else if (name === 'style' && typeof value === 'object') {
      for (const [prop, v] of Object.entries(value)) {
        effect(() => {
          node.style.setProperty(prop, String(readNode(v)));
        });
      }
    } else if (typeof value === 'function') {
      effect(() => setAttribute(node, name, value()));
    } else {
      setAttribute(node, name, value);
    }
  }

  appendChildren(node, children);
  return node;
}

function setAttribute(node, name, value) {
  if (name === 'value' && 'value' in node) {
    if (node.value !== String(value)) node.value = String(value);
  } else if (name === 'checked' && 'checked' in node) {
    node.checked = Boolean(value);
  } else if (value === false || value === null || value === undefined) {
    node.removeAttribute(name);
  } else if (value === true) {
    node.setAttribute(name, '');
  } else {
    node.setAttribute(name, String(value));
  }
}

function appendChildren(parent, children) {
  for (const child of [].concat(children)) {
    if (child === null || child === undefined) continue;
    if (child instanceof Node) {
      parent.appendChild(child);
    } else if (typeof child === 'function') {
      parent.appendChild(dynamic(child));
    } else {
      parent.appendChild(document.createTextNode(String(child)));
    }
  }
}

/**
 * A text node bound to a getter.
 *
 * Updating it writes `nodeValue` rather than replacing the node, so the
 * browser keeps selection and caret position — one of the things a
 * virtual-DOM diff has to work to preserve and this gets for free.
 */
export function text(getter) {
  const node = document.createTextNode('');
  effect(() => {
    const value = readNode(getter);
    node.nodeValue = value === null || value === undefined ? '' : String(value);
  });
  return node;
}

// --- the template surface (spec §16.2 R2) --------------------------------
//
// Generated code does not build the DOM node by node. It parses one static
// HTML string per view region into a `<template>`, clones it per
// instantiation, walks to compile-time-computed offsets, and attaches a
// binding only at the holes. Everything below is what that emission needs;
// it is additive, and `dynamic`, `each` and `when` are re-expressed as thin
// wrappers over it so there is one implementation of each rather than two.

/**
 * Parse `html` once, then hand out a fresh clone per call.
 *
 * The returned value is a *fragment*, not its first child. A view region
 * may legally have several roots — `view`, a `when` arm and an `each` body
 * are all node lists — and returning `content.firstChild` silently discards
 * every root but the first (spec §16.9, finding 8).
 *
 * `html` is never a runtime value. The compiler interpolates only
 * compile-time string *literals* into it, HTML-escaped (spec §16.3.5);
 * every value a program computes reaches the DOM through `nodeValue`,
 * `setAttribute`, `.value` or `.checked`, none of which parses HTML. So
 * template cloning adds no injection surface over the node-by-node path.
 */
export function template(html) {
  let content;
  return () => {
    if (content === undefined) {
      const element = document.createElement('template');
      element.innerHTML = html;
      content = element.content;
    }
    return content.cloneNode(true);
  };
}

/**
 * A fragment holding an empty anchored region: a start and an end comment.
 *
 * A region that is nothing but a hole has no markup to clone, so the
 * emitter calls this instead of parsing a template made of two comments.
 *
 * The two comments carry `[` and `]`, which is the same pair the emitted
 * template markup carries and for the same reason (#208): a clone leaves
 * them adjacent, a served document does not, and a reader of the served
 * bytes needs to be able to tell one end of a region from the other.
 */
export function anchors() {
  const fragment = document.createDocumentFragment();
  fragment.append(document.createComment('['), document.createComment(']'));
  return fragment;
}

/**
 * Bind an existing text node to a getter.
 *
 * The write is guarded by a comparison (spec §16.2 R7). A list re-supplies
 * every surviving row's item on every change, which re-runs every row's
 * binding; without the guard, one changed row dirties layout for all of
 * them. `setAttribute` below already does exactly this for `value`.
 */
export function bindText(node, getter) {
  effect(() => {
    const value = readNode(getter);
    const next = value === null || value === undefined ? '' : String(value);
    if (node.nodeValue !== next) node.nodeValue = next;
  });
}

/**
 * The schemes a URL-bearing attribute may name (spec §16.3.5, corrected).
 *
 * §16.3.5's escaping argument is about the *markup* grammar: it
 * establishes that a value cannot close a tag or open one. It says nothing
 * about `href` and `src`, which the browser hands to the URL parser
 * instead. `setAttribute('href', v)` stores `v` verbatim, and
 * `javascript:alert(1)` in an `href` executes on click — there is nothing
 * in it for an HTML escaper to escape. Escaping for HTML text is not
 * escaping for a URL; they are different grammars.
 *
 * An allowlist, not a list of the dangerous schemes. `javascript:`,
 * `data:` and `vbscript:` are the three usually named, but which schemes a
 * browser executes is the browser's decision and it changes; a denylist is
 * out of date the day it is written.
 *
 * The compiler settles every URL it can see — a literal in an `href` is a
 * compile error, not a value filtered here — so this runs only on values
 * the compiler could not see. It is the Rust half's exact mirror
 * (`zdc_hir::url_is_safe`), and `crates/zdc-codegen/tests/url.rs` runs the
 * two against one table so that changing one without the other fails.
 *
 * A refused URL becomes the empty string, not `#`: a link that goes
 * nowhere should not scroll the page to the top when it is clicked.
 */
const URL_SCHEMES = ['http', 'https', 'mailto', 'tel'];

export function safeUrl(value) {
  const url = value === null || value === undefined ? '' : String(value);
  // Leading whitespace is stripped by the browser before it parses the
  // scheme, so `\njavascript:alert(1)` is a `javascript:` URL.
  const trimmed = url.trimStart();
  const colon = trimmed.indexOf(':');
  if (colon === -1) return url;
  const scheme = trimmed.slice(0, colon);
  // A colon inside a path or a query is not a scheme: `/a:b` is relative.
  if (/[/?#]/.test(scheme)) return url;
  return URL_SCHEMES.includes(scheme.toLowerCase()) ? url : '';
}

/** Bind an existing element's attribute to a getter. */
export function bindAttr(node, name, getter) {
  effect(() => setAttribute(node, name, readNode(getter)));
}

/** Bind one CSS property of an existing element to a getter. */
export function bindStyle(node, property, getter) {
  effect(() => {
    node.style.setProperty(property, String(readNode(getter)));
  });
}

/**
 * Attach an event listener to an existing element.
 *
 * Batched, so generated code emits no `batch(...)` of its own, and `el`
 * routes here rather than repeating the listener: one place decides what a
 * handler is.
 *
 * **A handler that throws is contained and reported (#139)** — the page
 * keeps running, its writes stand, and `reportError` is the platform's own
 * uncaught-error channel. `docs/reference.md` §10 argues it.
 */
export function on(node, event, handler) {
  node.addEventListener(event, (e) => {
    try {
      batch(() => handler(e));
    } catch (failure) {
      reportError(failure);
    }
  });
}

/**
 * A region between two existing anchors whose content is replaced when its
 * getter changes.
 */
export function dynamicInto(start, end, getter) {
  effect(() => {
    const value = readNode(getter);
    clearBetween(start, end);
    const rendered = value instanceof Node ? value : document.createTextNode(String(value ?? ''));
    end.parentNode.insertBefore(rendered, end);
  });
}

/**
 * A region whose content is replaced when its getter changes.
 *
 * Anchored between two comment nodes so the region's extent is known
 * without wrapping it in an element the program did not ask for.
 */
export function dynamic(getter) {
  const fragment = anchors();
  dynamicInto(fragment.firstChild, fragment.lastChild, getter);
  return fragment;
}

/** Construct a variant value. */
export function variant(tag, ...fields) {
  return { tag, fields };
}

/** Mount a rendered tree into a container, replacing its contents. */
export function mount(node, container) {
  container.replaceChildren(node);
  return node;
}

/**
 * Empty an anchored region, leaving its two anchors in place.
 *
 * Exported because `branch.js` tears a region down for the same reason
 * `dynamicInto` above does, and one implementation of "empty the region"
 * is better than two that have to agree.
 */
export function clearBetween(start, end) {
  let node = start.nextSibling;
  while (node && node !== end) {
    const next = node.nextSibling;
    node.remove();
    node = next;
  }
}
