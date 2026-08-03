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

import { signal, derived, effect, batch } from './signal.js';

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

/**
 * A region whose content is replaced when its getter changes.
 *
 * Anchored between two comment nodes so the region's extent is known
 * without wrapping it in an element the program did not ask for.
 */
export function dynamic(getter) {
  const fragment = document.createDocumentFragment();
  const start = document.createComment('');
  const end = document.createComment('');
  fragment.append(start, end);

  effect(() => {
    const value = read(getter);
    clearBetween(start, end);
    const rendered = value instanceof Node ? value : document.createTextNode(String(value ?? ''));
    end.parentNode.insertBefore(rendered, end);
  });

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
  const fragment = document.createDocumentFragment();
  const start = document.createComment('');
  const end = document.createComment('');
  fragment.append(start, end);

  /** key -> { node, read, write } */
  let mounted = new Map();

  effect(() => {
    const items = read(listGetter) ?? [];
    const next = new Map();
    const parent = end.parentNode;

    // Build the next set, reusing nodes whose key is unchanged.
    let cursor = start.nextSibling;
    for (const item of items) {
      const key = keyOf(item);
      if (next.has(key)) {
        throw new Error(
          `Duplicate key ${JSON.stringify(key)} in a list. Keys must be unique.`
        );
      }
      let entry = mounted.get(key);
      if (entry === undefined) {
        // Each row owns a signal holding its item, and `render` receives
        // that signal rather than the value. Reusing a node is then only a
        // decision about DOM identity — the row's *content* still flows
        // through the same reactive path as everything else.
        const [readItem, writeItem] = signal(item);
        entry = { read: readItem, write: writeItem, node: render(readItem) };
      } else {
        // A surviving key must still see its new value. Without this a row
        // whose value changed but whose key did not shows stale content
        // forever — the most common list update there is.
        entry.write(item);
      }
      next.set(key, entry);

      // Move into place only when it is not already there.
      if (cursor !== entry.node) {
        parent.insertBefore(entry.node, cursor);
      } else {
        cursor = cursor.nextSibling;
      }
    }

    // Remove what survived from the previous pass but is not in the next.
    for (const [key, entry] of mounted) {
      if (!next.has(key)) entry.node.remove();
    }

    mounted = next;
  });

  return fragment;
}

/**
 * Variant dispatch — `when value` over `Remote`, `Option`, or a `choice`.
 *
 * `arms` maps a variant name to a function receiving that variant's
 * fields positionally. Spec §14G.1.6 requires every arm to be present,
 * so a missing arm is a compiler bug rather than a runtime fallback.
 */
export function when(getter, arms) {
  return dynamic(
    derived(() => {
      const value = read(getter);
      const arm = arms[value.tag];
      if (arm === undefined) {
        throw new Error(
          `No arm for variant ${JSON.stringify(value.tag)}. The compiler should have rejected this.`
        );
      }
      return arm(...(value.fields ?? []));
    })
  );
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
