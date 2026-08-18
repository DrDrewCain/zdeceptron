// A minimal DOM, sufficient to exercise the renderer.
//
// The renderer is the half of the runtime that a signal test cannot reach,
// and it is where the interesting failures live: keyed reconciliation
// moving the wrong node, a text binding replacing rather than updating,
// an attribute effect that never detaches. Testing it needs a document.
//
// This is deliberately not a browser. It implements exactly the surface
// `dom.js` and `elements.js` touch, so that a test failing here means the
// runtime is wrong rather than the shim being incomplete. Anything the
// runtime starts using that is missing will throw rather than silently
// no-op — a shim that quietly returns undefined would make the tests lie.

let nodeSerial = 1;

// `dom.js` tests `child instanceof Node`, so the shim needs a real
// constructor rather than plain objects — otherwise every append silently
// falls through to the string branch and renders "[object Object]".
/** The `nodeType` constants the runtime actually branches on. */
const NODE_TYPE = { element: 1, text: 3, comment: 8, fragment: 11 };

class Node {
  // On the prototype, NOT in the object literal below: `Object.assign`
  // invokes a getter and copies its value, which would freeze
  // `nextSibling` at creation time. That silently broke `dynamic()` —
  // its `clearBetween` walk found nothing to remove, so `when` appended
  // each new branch beside the old one instead of replacing it.
  get nextSibling() {
    const siblings = this.parentNode ? this.parentNode.childNodes : null;
    if (!siblings) return null;
    const index = siblings.indexOf(this);
    return index >= 0 && index + 1 < siblings.length ? siblings[index + 1] : null;
  }

  // Live, for the same reason. Generated code walks a clone by
  // `firstChild`/`nextSibling` offsets computed at compile time, so a
  // stale value here would point every binding at the wrong node.
  get firstChild() {
    return this.childNodes.length > 0 ? this.childNodes[0] : null;
  }

  get lastChild() {
    return this.childNodes.length > 0 ? this.childNodes[this.childNodes.length - 1] : null;
  }

  get nodeType() {
    return NODE_TYPE[this.kind];
  }

  // `Dialog`'s binding asks this before it calls `showModal`, because a
  // real `showModal` throws `InvalidStateError` on a node that is not in
  // the document — and every binding this runtime attaches runs while the
  // tree is still a clone of a `<template>`.
  //
  // ⚠️ THERE IS NO DOCUMENT HERE, so this cannot be the browser's answer
  // and does not pretend to be. What it models is the distinction the
  // binding actually turns on: a node still inside the fragment the
  // template handed out is *not* placed, and a node whose ancestor chain
  // ends at an element — which is what `mount` and every `insertBefore`
  // produce — is. A freshly created element that was never appended is
  // its own root and reports false, as it would in a browser.
  get isConnected() {
    let node = this;
    while (node.parentNode) node = node.parentNode;
    return node !== this && node.kind === 'element';
  }
}

function baseNode(kind) {
  return Object.assign(new Node(), {
    __id: nodeSerial++,
    kind,
    parentNode: null,
    childNodes: [],

    appendChild(child) {
      return this.insertBefore(child, null);
    },

    insertBefore(child, reference) {
      if (child.kind === 'fragment') {
        // A fragment inserts its children and empties itself, as in a browser.
        for (const grandchild of [...child.childNodes]) {
          this.insertBefore(grandchild, reference);
        }
        child.childNodes.length = 0;
        return child;
      }
      if (child.parentNode) child.parentNode.removeChild(child);
      const index = reference === null ? this.childNodes.length : this.childNodes.indexOf(reference);
      if (index < 0) throw new Error('insertBefore: reference node is not a child');
      this.childNodes.splice(index, 0, child);
      child.parentNode = this;
      return child;
    },

    removeChild(child) {
      const index = this.childNodes.indexOf(child);
      if (index < 0) throw new Error('removeChild: node is not a child');
      this.childNodes.splice(index, 1);
      child.parentNode = null;
      return child;
    },

    remove() {
      if (this.parentNode) this.parentNode.removeChild(this);
    },

    replaceChildren(...nodes) {
      for (const child of [...this.childNodes]) this.removeChild(child);
      for (const node of nodes) this.appendChild(node);
    },

    // What `template()` hands out per instantiation. A shallow clone would
    // silently produce an empty region, so `deep` is honoured rather than
    // ignored.
    cloneNode(deep = false) {
      let copy;
      if (this.kind === 'element') {
        copy = createElement(this.tagName);
        for (const [name, value] of Object.entries(this.attributes)) {
          copy.setAttribute(name, value);
        }
      } else if (this.kind === 'text') {
        copy = createTextNode(this.nodeValue);
      } else if (this.kind === 'comment') {
        copy = createComment(this.nodeValue);
      } else {
        copy = createDocumentFragment();
      }
      if (deep) {
        for (const child of this.childNodes) copy.appendChild(child.cloneNode(true));
      }
      return copy;
    },

    // The §16.3.6 parity test's assertion: the tree `elements.js` builds
    // node by node must equal the tree the compiler's markup parses into.
    // Attribute ORDER is not part of the comparison, as in a browser.
    isEqualNode(other) {
      if (!other || this.kind !== other.kind) return false;
      if (this.kind === 'text' || this.kind === 'comment') {
        return this.nodeValue === other.nodeValue;
      }
      if (this.kind === 'element') {
        if (this.tagName !== other.tagName) return false;
        const names = Object.keys(this.attributes).sort();
        const otherNames = Object.keys(other.attributes).sort();
        if (names.length !== otherNames.length) return false;
        for (let i = 0; i < names.length; i += 1) {
          if (names[i] !== otherNames[i]) return false;
          if (this.attributes[names[i]] !== other.attributes[names[i]]) return false;
        }
      }
      if (this.childNodes.length !== other.childNodes.length) return false;
      for (let i = 0; i < this.childNodes.length; i += 1) {
        if (!this.childNodes[i].isEqualNode(other.childNodes[i])) return false;
      }
      return true;
    },
  });
}

// The two `innerHTML` accessors, defined once and shared by every node
// rather than rebuilt per element.
//
// They were built inside `createElement`, which allocated a descriptor and
// two closures for every node the suite made. Keyed reconciliation makes
// thousands, and the extra garbage was enough to bring a collection down
// inside a `Set` iteration — where this engine's finaliser panics on a
// borrow it already holds. Sharing them is the same behaviour with a
// fraction of the allocation: both read `this`, so neither needs a
// per-node binding.

// `template()` is the whole of the emitted render path: one static HTML
// string parsed once, cloned per instantiation. Without `content` and an
// `innerHTML` that really parses, the shim would make every generated
// program render nothing while reporting no error.
const TEMPLATE_INNER_HTML = {
  get() {
    return serialize(this.content);
  },
  set(value) {
    this.content = parseHtml(String(value));
  },
};

// `markup()` in `dom.js` assigns `innerHTML` on an ordinary element — the
// one place in the runtime that parses HTML. It must really parse here
// too, or a test asserting the rendered structure of a post would pass
// against a shim that stored a string and built no nodes.
const ELEMENT_INNER_HTML = {
  get() {
    return serialize(this);
  },
  set(value) {
    const parsed = parseHtml(String(value));
    for (const child of this.childNodes) child.parentNode = null;
    this.childNodes = [];
    for (const child of [...parsed.childNodes]) {
      child.parentNode = this;
      this.childNodes.push(child);
    }
  },
};

// `NumberInput` and `DateInput` bind through `valueAsNumber` in both
// directions (#45, #48), so a shim that did not derive it from `value`
// would make every such binding read `undefined` and write
// unconditionally — the suite would pass over a control that does
// nothing. Shared rather than built per node, for the allocation reason
// the two `innerHTML` descriptors above are.
//
// ⚠️ THIS IS NOT THE BROWSER'S ALGORITHM AND CANNOT BE. A real number
// field runs HTML's value sanitisation, which empties `value` while the
// reader is part way through `1.` or `-`; this keeps the text it was
// given. So the half-typed states belong to the browser suite
// (`zdc-cli/tests/browser.rs`), exactly as the parser's insertion modes
// do. What is faithful here is everything either side of them: a complete
// number, a complete `YYYY-MM-DD`, and the empty box.
const VALUE_AS_NUMBER = {
  get() {
    if (this.attributes.type === 'date') {
      const day = /^(\d{4})-(\d{2})-(\d{2})$/.exec(this.value);
      return day ? Date.UTC(Number(day[1]), Number(day[2]) - 1, Number(day[3])) : NaN;
    }
    return this.value.trim() === '' ? NaN : Number(this.value);
  },
  set(number) {
    if (Number.isNaN(number)) {
      this.value = '';
    } else if (this.attributes.type === 'date') {
      this.value = new Date(number).toISOString().slice(0, 10);
    } else {
      this.value = String(number);
    }
  },
  configurable: true,
};

// The open/closed state machine of a `<dialog>`, and nothing else about
// one (#53).
//
// ⚠️ THIS IS NOT A MODAL AND CANNOT BE. There is no top layer here, no
// backdrop, no `inert`, no focus and no close request, so the four
// properties `Dialog` exists for are all absent. They belong to the
// browser suite (`zdc-cli/tests/browser.rs`), exactly as the HTML
// parser's insertion modes and a number field's value sanitisation do.
//
// What is faithful is the part the emitted binding is written against,
// and it is faithful in the way that matters — **it throws where a
// browser throws**. `showModal()` on an already-open dialog is an
// `InvalidStateError`, and so is one on a node that is not in the
// document; a binding that got either wrong would fail here rather than
// silently doing nothing, which is this file's whole rule. `close()` on a
// closed dialog is the no-op HTML specifies, and a real close fires
// `close`, which is the event the write-back listens for.
//
// The `open` attribute is written and removed alongside the property so
// that a test can read the state out of a serialised tree.
function dialogState(node) {
  node.open = false;
  node.showModal = function () {
    if (this.open) throw new Error('showModal: the dialog is already open (InvalidStateError)');
    if (!this.isConnected) {
      throw new Error('showModal: the dialog is not in the document (InvalidStateError)');
    }
    this.open = true;
    this.setAttribute('open', '');
  };
  node.close = function () {
    if (!this.open) return;
    this.open = false;
    this.removeAttribute('open');
    this.fire('close');
  };
}

function createElement(tag) {
  const node = baseNode('element');
  node.tagName = tag;
  node.attributes = {};
  node.listeners = {};
  // `dom.js` routes `value` and `checked` to properties rather than
  // attributes, guarded by `'value' in node`. Form controls must therefore
  // have them present, and other elements must not.
  if (tag === 'input' || tag === 'textarea' || tag === 'select') {
    node.value = '';
    node.checked = false;
  }
  if (tag === 'input') {
    Object.defineProperty(node, 'valueAsNumber', VALUE_AS_NUMBER);
  }
  if (tag === 'dialog') dialogState(node);
  node.style = {
    properties: {},
    setProperty(name, value) {
      this.properties[name] = value;
    },
  };
  node.setAttribute = function (name, value) {
    this.attributes[name] = String(value);
  };
  node.removeAttribute = function (name) {
    delete this.attributes[name];
  };
  node.addEventListener = function (event, handler) {
    (this.listeners[event] ??= []).push(handler);
  };
  // Test-only: deliver an event without a full event system.
  node.fire = function (event, payload = {}) {
    for (const handler of this.listeners[event] ?? []) {
      handler({ target: this, ...payload });
    }
  };
  // `template()` is the whole of the emitted render path: one static HTML
  // string parsed once, cloned per instantiation. Without `content` and an
  // `innerHTML` that really parses, the shim would make every generated
  // program render nothing while reporting no error.
  if (tag === 'template') {
    node.content = createDocumentFragment();
    Object.defineProperty(node, 'innerHTML', TEMPLATE_INNER_HTML);
  } else {
    Object.defineProperty(node, 'innerHTML', ELEMENT_INNER_HTML);
  }
  return node;
}

// --- the HTML parser ------------------------------------------------------
//
// Only what the compiler's nine built-in elements can produce: start tags
// with quoted or bare attribute values, end tags, void elements, comments,
// and text with the five escapes the emitter writes. Anything it does not
// understand throws, because a parser that silently skipped a construct
// would move every subsequent `nextSibling` offset by one and point every
// binding after it at the wrong node — the exact failure §16.10 names as
// having no compile-time signal.

const VOID_ELEMENTS = new Set([
  'area', 'base', 'br', 'col', 'embed', 'hr', 'img',
  'input', 'link', 'meta', 'param', 'source', 'track', 'wbr',
]);

function decodeEntities(text) {
  return text
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&amp;/g, '&');
}

/** Parse a start tag, returning its name, attributes, and end offset. */
function parseStartTag(source, start) {
  let i = start + 1;
  let name = '';
  while (i < source.length && /[A-Za-z0-9-]/.test(source[i])) name += source[i++];
  if (name === '') throw new Error(`template HTML: expected a tag name at ${start}`);

  const attributes = {};
  for (;;) {
    while (i < source.length && /\s/.test(source[i])) i += 1;
    if (i >= source.length) throw new Error('template HTML: unterminated start tag');
    if (source[i] === '/') {
      i += 1;
      continue;
    }
    if (source[i] === '>') {
      i += 1;
      break;
    }

    let attribute = '';
    while (i < source.length && !/[\s=>/]/.test(source[i])) attribute += source[i++];
    if (attribute === '') throw new Error(`template HTML: expected an attribute name at ${i}`);

    let value = '';
    while (i < source.length && /\s/.test(source[i])) i += 1;
    if (source[i] === '=') {
      i += 1;
      while (i < source.length && /\s/.test(source[i])) i += 1;
      const quote = source[i];
      if (quote === '"' || quote === "'") {
        i += 1;
        const close = source.indexOf(quote, i);
        if (close < 0) throw new Error('template HTML: unterminated attribute value');
        value = decodeEntities(source.slice(i, close));
        i = close + 1;
      } else {
        while (i < source.length && !/[\s>]/.test(source[i])) value += source[i++];
        value = decodeEntities(value);
      }
    }
    attributes[attribute] = value;
  }
  return { name, attributes, end: i };
}

function parseHtml(source) {
  const root = createDocumentFragment();
  const stack = [root];
  const top = () => stack[stack.length - 1];
  let i = 0;

  const addText = (raw) => {
    if (raw.length > 0) top().appendChild(createTextNode(decodeEntities(raw)));
  };

  while (i < source.length) {
    const lt = source.indexOf('<', i);
    if (lt < 0) {
      addText(source.slice(i));
      break;
    }
    addText(source.slice(i, lt));

    if (source.startsWith('<!--', lt)) {
      const close = source.indexOf('-->', lt + 4);
      if (close < 0) throw new Error('template HTML: unterminated comment');
      top().appendChild(createComment(source.slice(lt + 4, close)));
      i = close + 3;
      continue;
    }

    if (source.startsWith('</', lt)) {
      const close = source.indexOf('>', lt);
      if (close < 0) throw new Error('template HTML: unterminated end tag');
      if (stack.length === 1) throw new Error('template HTML: end tag with no open element');
      stack.pop();
      i = close + 1;
      continue;
    }

    const tag = parseStartTag(source, lt);
    const element = createElement(tag.name);
    for (const [name, value] of Object.entries(tag.attributes)) {
      element.setAttribute(name, value);
    }
    top().appendChild(element);
    if (!VOID_ELEMENTS.has(tag.name)) stack.push(element);
    i = tag.end;
  }

  if (stack.length !== 1) throw new Error('template HTML: unclosed element');
  return root;
}

function createTextNode(value) {
  const node = baseNode('text');
  node.nodeValue = String(value);
  return node;
}

function createComment(value) {
  const node = baseNode('comment');
  node.nodeValue = String(value);
  return node;
}

function createDocumentFragment() {
  const node = baseNode('fragment');
  node.append = function (...children) {
    for (const child of children) this.appendChild(child);
  };
  // A fragment serialises to its children and nothing else, which is what
  // makes it the right container for a prerender: an element would put
  // one more `<div>` around the page than the client builds, and the
  // emitted walk indexes from the container's first child — so every
  // binding would attach one level out.
  //
  // `serializeForParse` and not `serialize`, because this markup is going
  // to be *parsed back* rather than compared as a string.
  Object.defineProperty(node, 'innerHTML', {
    get() {
      return serializeForParse(this);
    },
  });
  return node;
}

// The document's own listener table, for `keys.js`.
//
// A pair rather than one function, because what `keys.js` claims is that a
// discarded listener *stops firing*, and a shim whose `removeEventListener`
// is a no-op would agree with a runtime that never called it. `fire` walks
// the registered list, so a listener that was removed is a listener that is
// not there.
const documentListeners = {};

const document = {
  createElement,
  // The namespace is recorded and otherwise ignored. This shim models the
  // *tree* — what `isEqualNode` in the parity suite compares — and a real
  // namespace would need a real HTML parser to be worth anything. The one
  // claim that turns on it, that an `each` row of `<path>` is an
  // `SVGPathElement`, is settled in `zdc-cli/tests/browser.rs` by a real
  // browser, because nothing else can settle it.
  createElementNS(namespace, tag) {
    const node = createElement(tag);
    node.namespaceURI = namespace;
    return node;
  },
  createTextNode,
  createComment,
  createDocumentFragment,
  addEventListener(event, handler) {
    (documentListeners[event] ??= []).push(handler);
  },
  removeEventListener(event, handler) {
    const registered = documentListeners[event];
    if (!registered) return;
    const at = registered.indexOf(handler);
    if (at !== -1) registered.splice(at, 1);
  },
  /** How many listeners are registered — the leak check. */
  listenerCount(event) {
    return (documentListeners[event] ?? []).length;
  },
  /** Test-only: deliver `payload` to every registered listener. */
  fire(event, payload = {}) {
    for (const handler of (documentListeners[event] ?? []).slice()) {
      handler({ type: event, target: null, ...payload });
    }
  },
};

/** Serialise a subtree so assertions can be written against a string. */
function html(node) {
  if (node.kind === 'text') return node.nodeValue;
  if (node.kind === 'comment') return '';
  const inner = node.childNodes.map(html).join('');
  if (node.kind === 'fragment') return inner;
  const attrs = Object.entries(node.attributes)
    .map(([k, v]) => (v === '' ? ` ${k}` : ` ${k}="${v}"`))
    .join('');
  return `<${node.tagName}${attrs}>${inner}</${node.tagName}>`;
}

/**
 * Serialise a subtree for a parity assertion, holding back nothing.
 *
 * `html` above drops comments and form-control state, which is right for
 * readable assertions and wrong for proving two render strategies agree:
 * a missing anchor pair or an unwritten `input.value` would compare equal.
 * This one shows both.
 */
function serialize(node) {
  if (node.kind === 'text') return node.nodeValue;
  if (node.kind === 'comment') return `<!--${node.nodeValue}-->`;
  const inner = node.childNodes.map(serialize).join('');
  if (node.kind === 'fragment') return inner;

  const attrs = Object.entries(node.attributes)
    .map(([k, v]) => (v === '' ? ` ${k}` : ` ${k}="${v}"`))
    .join('');
  let state = '';
  if ('value' in node) state += ` .value="${node.value}"`;
  if ('checked' in node && node.checked) state += ' .checked';
  return `<${node.tagName}${attrs}${state}>${inner}</${node.tagName}>`;
}

/**
 * Serialise a subtree so that parsing it back gives the *same node
 * structure*, which is what a prerender needs and `serialize` does not
 * promise.
 *
 * One difference, and it is the whole reason this exists. A binding whose
 * value is empty leaves an empty text node, `serialize` writes nothing
 * for it, and the HTML parser makes no text node at all — so the walk the
 * emitted module does lands on `null` and the module throws before it has
 * bound anything. `dom.js`'s `text_child` already knows this: the
 * template it emits carries a deliberate single space for exactly this
 * reason, and this writes the same space for the same reason.
 *
 * The space is never seen. A binding's effect runs synchronously while
 * the module evaluates, which is inside the task that loaded it and
 * therefore before the browser's next paint — the same argument the
 * template's own space rests on.
 */
function serializeForParse(node) {
  if (node.kind === 'text') return node.nodeValue === '' ? ' ' : escapeText(node.nodeValue);
  if (node.kind === 'comment') return `<!--${node.nodeValue}-->`;
  const inner = node.childNodes.map(serializeForParse).join('');
  if (node.kind === 'fragment') return inner;
  const attrs = Object.entries(node.attributes)
    .map(([k, v]) => ` ${k}="${escapeAttribute(v)}"`)
    .join('');
  if (VOID_TAGS.has(node.tagName)) return `<${node.tagName}${attrs}>`;
  return `<${node.tagName}${attrs}>${inner}</${node.tagName}>`;
}

/** Tags the HTML parser closes for you, and which must not be written
 *  with an end tag or the parser puts what follows inside them. */
const VOID_TAGS = new Set([
  'area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input',
  'link', 'meta', 'source', 'track', 'wbr',
]);

function escapeText(value) {
  return String(value).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function escapeAttribute(value) {
  return String(value)
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}

/** Every element in a subtree, in document order. */
function walk(node, out = []) {
  if (node.kind === 'element') out.push(node);
  for (const child of node.childNodes) walk(child, out);
  return out;
}

/** The first element whose tag matches, or null. */
function findTag(node, tagName) {
  return walk(node).find((n) => n.tagName === tagName) ?? null;
}

// --- the microtask queue --------------------------------------------------
//
// `Dialog` defers an opening that arrives while its node is still
// detached, because `showModal()` throws on one that is not in the
// document and every binding runs before the tree is inserted. A browser
// drains these at the end of the current task; the engine this shim runs
// in has no such queue at all, and a `queueMicrotask` that ran its
// callback immediately would be the opposite of what the deferral is for
// — it would call `showModal()` at exactly the moment that throws.
//
// So the drain is explicit, and a test says when the task ends. That is
// the honest shape: "after the synchronous work that mounted the tree" is
// what a browser means too, and here the test is the one that knows when
// that was.
const microtasks = [];

function queueMicrotask(callback) {
  microtasks.push(callback);
}

/** Test-only: end the current task, as a browser does. */
function flushMicrotasks() {
  // Drained rather than iterated: a callback may queue another.
  while (microtasks.length > 0) microtasks.shift()();
}

// --- the uncaught-error channel -------------------------------------------
//
// `dom.js` reports a throwing handler through `reportError` (#139), which
// in a browser fires `window.onerror` and the `error` event. There is no
// such channel here, so this one records: a test can then assert what a
// page would have been told, which is the part of the decision that would
// otherwise only be checkable in a browser.
//
// `var` rather than `const`, so a test can replace it for one case and put
// it back — which is what the browser lets a page do too.
var reported = [];
var reportError = function (error) {
  reported.push(error);
};
