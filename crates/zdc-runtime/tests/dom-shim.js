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

let nextId = 1;

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
}

function baseNode(kind) {
  return Object.assign(new Node(), {
    __id: nextId++,
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
    Object.defineProperty(node, 'innerHTML', {
      get() {
        return serialize(this.content);
      },
      set(value) {
        this.content = parseHtml(String(value));
      },
    });
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
  return node;
}

const document = {
  createElement,
  createTextNode,
  createComment,
  createDocumentFragment,
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
