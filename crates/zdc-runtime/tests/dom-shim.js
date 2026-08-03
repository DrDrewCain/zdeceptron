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
  return node;
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
