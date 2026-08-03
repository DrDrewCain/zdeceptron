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

import { signal, effect, batch, owned } from './signal.js';

/** A value that may be a signal getter or a constant. */
function read(value) {
  return typeof value === 'function' ? value() : value;
}

/**
 * Create an element with reactive properties and children.
 *
 * `props` values may be getters; each becomes its own effect, so changing
 * one attribute does not touch the others.
 */
export function el(tag, props = {}, children = []) {
  const node = document.createElement(tag);

  for (const [name, value] of Object.entries(props)) {
    if (name.startsWith('on')) {
      // Handlers are batched: `add 1 to a` then `set b to 2` in one
      // handler repaints once, not twice.
      const event = name.slice(2).toLowerCase();
      node.addEventListener(event, (e) => batch(() => value(e)));
    } else if (name === 'style' && typeof value === 'object') {
      for (const [prop, v] of Object.entries(value)) {
        effect(() => {
          node.style.setProperty(prop, String(read(v)));
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
    const value = read(getter);
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
 */
export function anchors() {
  const fragment = document.createDocumentFragment();
  fragment.append(document.createComment(''), document.createComment(''));
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
    const value = read(getter);
    const next = value === null || value === undefined ? '' : String(value);
    if (node.nodeValue !== next) node.nodeValue = next;
  });
}

/**
 * Replace an element's content with parsed HTML.
 *
 * **This is the only function in the runtime that parses HTML, and it is
 * the only assignment to `innerHTML` anywhere in it.** Everything else a
 * program renders reaches the DOM through `nodeValue`, `setAttribute`,
 * `.value` or `.checked`, none of which parses (spec §16.3.5). Adding
 * this narrows that claim rather than dropping it, and the narrowing is
 * carried by the compiler, not by anything here:
 *
 * * The emitter calls this from one place — `Slot::Rendered`, which only
 *   `Prose` has.
 * * `Prose`'s argument must have type `Markup`, which `Text` is not and
 *   does not convert to.
 * * The one producer of a `Markup` is `build markdown`, which runs inside
 *   the compiler over a file in the project directory, and which escapes
 *   every raw HTML span and rewrites every non-http(s) URL before
 *   returning.
 *
 * So this function trusts its argument, and the reason that is sound is
 * that no user-supplied value can ever become one. It performs no
 * sanitising of its own: a sanitiser here would be a second, weaker copy
 * of a guarantee the type system already makes, and the failure mode of
 * two disagreeing checks is worse than one.
 */
export function markup(node, value) {
  node.innerHTML = value === null || value === undefined ? '' : String(value);
}

/**
 * The same, re-parsed whenever the value changes.
 */
export function bindMarkup(node, getter) {
  effect(() => {
    const value = read(getter);
    const next = value === null || value === undefined ? '' : String(value);
    if (node.innerHTML !== next) node.innerHTML = next;
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
  effect(() => setAttribute(node, name, read(getter)));
}

/** Bind one CSS property of an existing element to a getter. */
export function bindStyle(node, property, getter) {
  effect(() => {
    node.style.setProperty(property, String(read(getter)));
  });
}

/**
 * Attach an event listener to an existing element.
 *
 * Batched, exactly as `el` batches the handlers it is given, so generated
 * code never has to emit a `batch(...)` wrapper of its own.
 */
export function on(node, event, handler) {
  node.addEventListener(event, (e) => batch(() => handler(e)));
}

/**
 * A region between two existing anchors whose content is replaced when its
 * getter changes.
 */
export function dynamicInto(start, end, getter) {
  effect(() => {
    const value = read(getter);
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

/**
 * Keyed list rendering — `each item in list`.
 *
 * Keys are required, not optional. Without identity, reordering destroys
 * and recreates nodes, which loses focus, scroll position, and the
 * contents of any input inside a row. That is a correctness bug, not a
 * performance one, which is why `keyOf` has no default.
 *
 * `render` receives a GETTER for its item, not the item. Reusing a node
 * across an update is a decision about DOM identity only; the row's
 * content still flows through a signal, so a changed value reaches the
 * bindings that read it without rebuilding the row.
 */
export function each(listGetter, keyOf, render) {
  const fragment = anchors();
  eachInto(fragment.firstChild, fragment.lastChild, listGetter, keyOf, render);
  return fragment;
}

/**
 * Keyed list rendering between two existing anchors.
 *
 * Two passes, and the order matters. Departed rows are retired *before*
 * anything is placed: a node about to be removed must not block the
 * cursor, or every row after a deletion gets moved. Measured at N=1000,
 * removing one row cost 994 moves under a single pass and 0 under this one.
 */
export function eachInto(start, end, listGetter, keyOf, render) {
  /** key -> { nodes, set, dispose } */
  let mounted = new Map();

  effect(() => {
    // Spread, not the value itself: pass 2 indexes `items`, and a list a
    // program built with `append` is an iterable chain until something
    // asks it to be an array. Iterating it is what asks. Pass 1 walks the
    // whole list anyway, so this costs no order of growth.
    const items = [...(read(listGetter) ?? [])];
    const parent = end.parentNode;

    batch(() => {
      // Pass 1: compute the key sequence and retire what left the list.
      const keys = [];
      let n = 0;
      for (const item of items) keys.push(keyOf(item, n++));
      const live = new Set(keys);
      for (const [key, entry] of mounted) {
        if (!live.has(key)) {
          for (const node of entry.nodes) node.remove();
          entry.dispose();
          mounted.delete(key);
        }
      }

      // Pass 2: create, re-supply, and place.
      const next = new Map();
      let cursor = start.nextSibling;
      for (let i = 0; i < items.length; i += 1) {
        const item = items[i];
        const key = keys[i];
        if (next.has(key)) {
          throw new Error(
            `Duplicate key ${JSON.stringify(key)} in a list. Keys must be unique.`
          );
        }
        let entry = mounted.get(key);
        if (entry === undefined) {
          // `render` receives a GETTER, not a value: the row outlives any
          // one version of the item, so its bindings must read through the
          // graph. Reusing a node is then only a decision about DOM
          // identity — the row's *content* still flows reactively.
          const [get, set] = signal(item);
          // Own the row's bindings so removing it unsubscribes them.
          const [rendered, dispose] = owned(() => render(get));
          // A row may legally have several roots, so an entry holds a node
          // LIST. Capture it before insertion empties the fragment.
          const nodes =
            rendered.nodeType === 11 ? [...rendered.childNodes] : [rendered];
          entry = { nodes, set, dispose };
        } else {
          // The key survived; the value need not have. Re-supplying it is
          // what makes an update to a row that kept its key visible.
          entry.set(item);
        }
        next.set(key, entry);

        if (cursor !== entry.nodes[0]) {
          for (const node of entry.nodes) parent.insertBefore(node, cursor);
        } else {
          cursor = entry.nodes[entry.nodes.length - 1].nextSibling;
        }
      }

      mounted = next;
    });
  });
}

/**
 * Variant dispatch — `when value` over `Remote`, `Option`, or a `choice`.
 *
 * `arms` maps a variant name to a function receiving that variant's
 * fields positionally. Spec §14G.1.6 requires every arm to be present,
 * so a missing arm is a compiler bug rather than a runtime fallback.
 */
export function when(getter, arms) {
  const fragment = anchors();
  whenInto(fragment.firstChild, fragment.lastChild, getter, arms);
  return fragment;
}

/** Variant dispatch between two existing anchors. */
export function whenInto(start, end, getter, arms) {
  // The arm's payload lives in a signal, and each field is handed to the
  // arm as a getter. So a changed payload flows to the bindings that read
  // it, and only a changed TAG rebuilds the subtree.
  //
  // The earlier implementation was `dynamic(derived(...))`, which rebuilt
  // on any change. Since every list in the language sits inside a `when`
  // arm, one changed cell tore down and recreated the entire list.
  const [fields, setFields] = signal([]);
  let currentTag = null;
  let disposeArm = null;

  effect(() => {
    const value = read(getter);
    setFields(value.fields ?? []);
    if (value.tag === currentTag) return;

    const arm = arms[value.tag];
    if (arm === undefined) {
      throw new Error(
        `No arm for variant ${JSON.stringify(value.tag)}. The compiler should have rejected this.`
      );
    }
    currentTag = value.tag;
    // The outgoing arm's bindings read this `when`'s own `fields` signal,
    // which keeps being written, so leaving them subscribed would keep
    // running them against detached nodes for the life of the page.
    if (disposeArm !== null) disposeArm();
    clearBetween(start, end);
    const binders = (value.fields ?? []).map((_, index) => () => fields()[index]);
    const [rendered, dispose] = owned(() => arm(...binders));
    disposeArm = dispose;
    end.parentNode.insertBefore(rendered, end);
  });
}

/**
 * Conditional rendering between two existing anchors — `if cond`.
 *
 * Not a `whenInto` with two arms: there is no variant here and no `choice`
 * the program declared, so there is no tag to switch on. The branch is
 * rebuilt only when the condition's *truth* changes, for exactly the reason
 * `whenInto` rebuilds only on a tag change — a condition that reads a
 * signal which keeps changing without crossing the boundary would otherwise
 * tear down and recreate the whole subtree on every write.
 *
 * `otherwise` may be null, which renders nothing.
 */
export function ifInto(start, end, condition, render, otherwise) {
  // `null` rather than a boolean, so the first run always renders: neither
  // branch has been built yet, and `false` would look like "already
  // showing the else".
  let current = null;
  let disposeBranch = null;

  effect(() => {
    const taken = Boolean(read(condition));
    if (taken === current) return;
    current = taken;

    // The outgoing branch's bindings read signals that keep being written,
    // so leaving them subscribed would keep running them against detached
    // nodes for the life of the page.
    if (disposeBranch !== null) disposeBranch();
    disposeBranch = null;
    clearBetween(start, end);

    const branch = taken ? render : otherwise;
    if (branch === null || branch === undefined) return;
    const [rendered, dispose] = owned(() => branch());
    disposeBranch = dispose;
    end.parentNode.insertBefore(rendered, end);
  });
}

/**
 * The interim key function: identity is the slot a row occupies.
 *
 * Spec §14G.6a reconciles by identity when the element type is a record
 * declaring `unique`, and positionally otherwise. There are no `record`
 * declarations yet, so every list is positional today. When `unique`
 * lands this is the one argument at the one call site that changes.
 */
export function byPosition(item, index) {
  return index;
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

function clearBetween(start, end) {
  let node = start.nextSibling;
  while (node && node !== end) {
    const next = node.nextSibling;
    node.remove();
    node = next;
  }
}
