// Tests for the element library. Run: `cargo test -p zdc-runtime`
//
// `elements.js` is the reference implementation of the built-in view
// vocabulary: it builds each element node by node, and the compiler's own
// shape table is checked against it by
// `crates/zdc-codegen/tests/element_parity.rs`. What that parity test
// cannot see is behaviour after construction, which is what is here.
//
// A suite of its own rather than part of `dom.test.js`, and the reason is
// measured. `boa` aborts the process with a Rust-level `BorrowMutError`
// inside its own `Set` builtin once a context's total allocation crosses a
// threshold, which BENCHMARKS.md already records as making signal fan-out
// unmeasurable here. `dom.test.js` and `elements.js` in one context sat on
// that threshold, so a vocabulary that grows would have kept walking into
// it. Two suites in two contexts is also the honest split: this one tests
// `elements.js` and that one tests `dom.js`.
//
// `document`, `html`, `serialize`, `findTag`, `test` and `assert` come from
// the harness; `signal`, `el`, `text` and the element constructors come
// from the runtime evaluated in the same scope.

test('Input binds two-way to a client signal', () => {
  const name = signal('world');
  const node = Input(name, { hint: 'your name' });
  assert.equal(node.attributes.placeholder, 'your name');
  assert.equal(node.value, 'world');

  // What typing does.
  node.fire('input', { target: { value: 'typed' } });
  assert.equal(name[0](), 'typed', 'typing must write back into the signal');

  // What a write from elsewhere does.
  name[1]('external');
  assert.equal(node.value, 'external', 'a signal write must reach the input');
});

test('the built-in elements render recognisable structure', () => {
  const [name] = signal('zd');
  const tree = Column(undefined, {}, [Heading(() => 'Title'), Text(name)]);
  assert.equal(tree.tagName, 'div');
  // A heading's level is its nesting depth, and this one is not nested, so
  // it is the document's first heading. The compiler chooses the tag; this
  // reference implementation has no enclosing context and renders the top.
  assert.equal(findTag(tree, 'h1') !== null, true, 'Heading renders an h1');
  assert.equal(html(findTag(tree, 'span')), '<span>zd</span>');
});

test('the semantic elements render the tag they name', () => {
  const page = Main({}, [
    Navigation({}, [Link(() => '/work', {}, [Text(() => 'work')])]),
    Article({}, [Paragraph(() => 'a sentence'), List({}, [Item(() => 'one')])]),
  ]);
  assert.equal(page.tagName, 'main');
  assert.equal(findTag(page, 'nav') !== null, true, 'Navigation renders a nav');
  assert.equal(html(findTag(page, 'a')), '<a href="/work"><span>work</span></a>');
  assert.equal(html(findTag(page, 'p')), '<p>a sentence</p>');
  assert.equal(html(findTag(page, 'ul')), '<ul><li>one</li></ul>');
});

test('a link that would run script goes nowhere instead', () => {
  const attack = Link(() => 'javascript:alert(1)', {}, []);
  assert.equal(attack.attributes.href, '', 'a script URL is filtered out');
  // A colon inside a path is not a scheme, so an ordinary URL survives.
  assert.equal(Link(() => '/a:b').attributes.href, '/a:b');
  assert.equal(Link(() => 'https://example.com').attributes.href, 'https://example.com');
});

test('ErrorBar renders its message as text, not as an attribute', () => {
  const node = ErrorBar({ message: 'boom' });
  assert.equal(html(node).includes('boom'), true, 'the message must be visible');
  assert.equal(node.attributes.message, undefined, 'message must not become an attribute');
});

test('Column and Row carry a base class rather than an inline style', () => {
  const column = Column(undefined, {}, []);
  assert.equal(column.attributes.class, 'zd-col');
  assert.equal(Object.keys(column.style.properties).length, 0, 'no inline style at all');
  assert.equal(Row(undefined, {}, []).attributes.class, 'zd-row');
  assert.equal(ErrorBar({ message: 'boom' }).attributes.class, 'zd-err');
});

test('a program class is appended to the base class, not replaced', () => {
  assert.equal(Column(undefined, { class: 'wide' }, []).attributes.class, 'zd-col wide');

  const [extra, setExtra] = signal('a');
  const reactive = Row(undefined, { class: extra }, []);
  assert.equal(reactive.attributes.class, 'zd-row a');
  setExtra('b');
  assert.equal(reactive.attributes.class, 'zd-row b', 'a reactive class must stay reactive');
});

test('Dialog opens and closes from its signal, and a dismissal writes back', () => {
  const showing = signal(false);
  const dialog = Dialog(showing, { label: 'Confirm deletion' }, [Text(() => 'Delete it?')]);
  // A node that is not in the document cannot be shown: a real
  // `showModal()` throws, and so does the shim's. So the dialog is placed
  // first, exactly as `mount` places the compiler's.
  const page = document.createElement('div');
  page.appendChild(dialog);

  assert.equal(dialog.open, false, 'a dialog whose signal is false starts closed');
  showing[1](true);
  assert.equal(dialog.open, true, 'writing the signal must show the modal');

  // What Escape ends in. The signal has to learn about it, or the program
  // and the DOM disagree and the next opening never happens.
  dialog.close();
  assert.equal(showing[0](), false, 'a dismissal must write the signal back');
  assert.equal(dialog.open, false, 'and the binding must not undo the dismissal');

  // A close request on a dialog that is already closed is a no-op, so
  // there is no second `close` event and nothing to write back.
  dialog.close();
  assert.equal(showing[0](), false, 'a no-op close must change nothing');

  showing[1](true);
  assert.equal(dialog.open, true, 'the signal must still open it after a dismissal');
  showing[1](false);
  assert.equal(dialog.open, false, 'and writing false must close it');
});

test('a Dialog that is already showing opens once it is in the document', () => {
  // Every binding the compiler emits runs while the tree is still a clone
  // of a `<template>`, and `showModal()` throws on a detached node. So an
  // opening that arrives too early is deferred rather than dropped.
  const showing = signal(true);
  const dialog = Dialog(showing, { label: 'Welcome' }, []);
  assert.equal(dialog.open, false, 'nothing can be shown before it is placed');

  const page = document.createElement('div');
  page.appendChild(dialog);
  flushMicrotasks();
  assert.equal(dialog.open, true, 'the deferred opening runs when the task ends');
});
