// Tests for document key listeners. Run: `cargo test -p zdc-runtime`
//
// A suite of its own for the reason `handler.test.js` is one: `boa` panics
// inside its own `Set` builtin once a context's total allocation crosses a
// threshold, and this file is about one decision anyway.
//
// `document`, `createElement`, `test` and `assert` come from the harness;
// `signal`, `owned` and `onKey` come from the runtime evaluated in the same
// scope.
//
// Three claims, and each is a separate case because each could break
// without the others noticing:
//
//   1. only the named key runs the handler;
//   2. no key runs it while an editable element has focus;
//   3. a discarded listener is *removed*, not merely inert.

// --- 1. only the key the program named -------------------------------------

test('a key handler runs for the key it named and no other', () => {
  const [count, setCount] = signal(0);
  const [, dispose] = owned(() => onKey('Escape', () => setCount(count() + 1)));

  document.fire('keydown', { key: 'Escape' });
  assert.equal(count(), 1, 'the named key ran it');

  document.fire('keydown', { key: 'a' });
  document.fire('keydown', { key: 'ArrowLeft' });
  document.fire('keydown', { key: 'escape' });
  assert.equal(count(), 1, 'no other key reached it, and the match is exact');

  dispose();
});

test('two handlers on two keys stay apart', () => {
  const [left, setLeft] = signal(0);
  const [right, setRight] = signal(0);
  const [, dispose] = owned(() => {
    onKey('ArrowLeft', () => setLeft(left() + 1));
    onKey('ArrowRight', () => setRight(right() + 1));
  });

  document.fire('keydown', { key: 'ArrowRight' });
  document.fire('keydown', { key: 'ArrowRight' });
  document.fire('keydown', { key: 'ArrowLeft' });
  assert.equal(left(), 1, 'left ran once');
  assert.equal(right(), 2, 'right ran twice');

  dispose();
});

// --- 2. the focus rule -----------------------------------------------------
//
// **This is the case the feature exists to make safe.** A document listener
// receives keystrokes aimed at every element on the page, including a field
// this program never declared. `on key "r"` must not be a way to read the
// `r` somebody is typing into a password box.

test('a keystroke aimed at a field does not reach a document handler', () => {
  const [seen, setSeen] = signal(0);
  const [, dispose] = owned(() => onKey('r', () => setSeen(seen() + 1)));

  document.fire('keydown', { key: 'r', target: createElement('input') });
  assert.equal(seen(), 0, 'an input is somebody typing');

  document.fire('keydown', { key: 'r', target: createElement('textarea') });
  assert.equal(seen(), 0, 'so is a textarea');

  document.fire('keydown', { key: 'r', target: createElement('select') });
  assert.equal(seen(), 0, 'so is a select');

  const editable = createElement('div');
  editable.isContentEditable = true;
  document.fire('keydown', { key: 'r', target: editable });
  assert.equal(seen(), 0, 'so is anything contenteditable');

  // The control. Without it this test is satisfied by a handler that never
  // runs at all, which would be a regression rather than a rule.
  document.fire('keydown', { key: 'r', target: createElement('div') });
  assert.equal(seen(), 1, 'a keystroke aimed at nothing editable reaches it');

  dispose();
});

test('a named key is suppressed inside a field too', () => {
  // Not an exception for `Escape`: a reader pressing Escape in a search box
  // means "clear this box", and the box is what should decide.
  const [closed, setClosed] = signal(0);
  const [, dispose] = owned(() => onKey('Escape', () => setClosed(closed() + 1)));

  document.fire('keydown', { key: 'Escape', target: createElement('input') });
  assert.equal(closed(), 0, 'the field has it');

  document.fire('keydown', { key: 'Escape', target: createElement('button') });
  assert.equal(closed(), 1, 'a button is not somewhere text goes');

  dispose();
});

// --- 3. disposal -----------------------------------------------------------
//
// A listener on a node dies when the node is removed, which is why
// `dom.js`'s `on` never detaches. A document listener has no such node. One
// left behind is a leak *and* a wrong answer: it keeps firing into a graph
// whose signals nothing renders any more.

test('a discarded handler is removed from the document, not merely inert', () => {
  const before = document.listenerCount('keydown');
  const [count, setCount] = signal(0);
  const [, dispose] = owned(() => onKey('Escape', () => setCount(count() + 1)));
  assert.equal(document.listenerCount('keydown'), before + 1, 'registered');

  document.fire('keydown', { key: 'Escape' });
  assert.equal(count(), 1, 'it ran while it was live');

  dispose();
  assert.equal(document.listenerCount('keydown'), before, 'and it was removed');

  document.fire('keydown', { key: 'Escape' });
  assert.equal(count(), 1, 'a discarded handler does not fire');
});

test('mounting and discarding many times leaves nothing behind', () => {
  // The shape #16.3.9 R1 was written against, applied to listeners: a
  // dialog opened and closed two hundred times must leave one document with
  // no listeners on it, not two hundred.
  const before = document.listenerCount('keydown');
  const [count, setCount] = signal(0);
  for (let i = 0; i < 200; i += 1) {
    const [, dispose] = owned(() => onKey('Escape', () => setCount(count() + 1)));
    dispose();
  }
  assert.equal(document.listenerCount('keydown'), before, 'none accumulated');

  document.fire('keydown', { key: 'Escape' });
  assert.equal(count(), 0, 'and none of the two hundred is still firing');
});

test('an inner scope is disposed with its outer one', () => {
  // What `ifInto` does: the branch closure is `owned` inside whatever scope
  // the region already opened, so disposing the region disposes the branch.
  const before = document.listenerCount('keydown');
  const [count, setCount] = signal(0);
  const [, disposeOuter] = owned(() => {
    onKey('Escape', () => setCount(count() + 1));
    owned(() => onKey('Enter', () => setCount(count() + 10)));
  });
  assert.equal(document.listenerCount('keydown'), before + 2, 'both registered');

  disposeOuter();
  assert.equal(document.listenerCount('keydown'), before, 'both removed');

  document.fire('keydown', { key: 'Escape' });
  document.fire('keydown', { key: 'Enter' });
  assert.equal(count(), 0, 'neither fires');
});
