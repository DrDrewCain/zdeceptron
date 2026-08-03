// Tests for the renderer. Run: `cargo test -p zdc-runtime`
//
// These execute against a minimal DOM implemented in the test harness, not
// a browser — the point is to catch renderer bugs (a text binding that
// replaces instead of updating, keyed reconciliation moving the wrong
// node, an attribute effect that never detaches) without needing a
// browser or a JavaScript toolchain installed.
//
// `document`, `html`, `findTag`, `test` and `assert` come from the harness;
// `signal`, `derived`, `el`, `text`, `each`, `when`, `variant`, `Column`,
// `Text`, `Button`, `Input` come from the runtime evaluated in the same scope.

test('el renders a tag with static attributes', () => {
  const node = el('div', { id: 'x' }, ['hi']);
  assert.equal(html(node), '<div id="x">hi</div>');
});

test('a text binding updates in place rather than replacing the node', () => {
  const [get, set] = signal('one');
  const node = el('span', {}, [text(get)]);
  const before = node.childNodes[0];
  assert.equal(html(node), '<span>one</span>');

  set('two');
  assert.equal(html(node), '<span>two</span>');
  // Identity matters: replacing the node would lose selection and caret
  // position, which is one of the things a virtual DOM has to work to keep.
  assert.equal(node.childNodes[0].__id, before.__id, 'the text node must be reused');
});

test('a reactive attribute updates without touching its siblings', () => {
  const [title, setTitle] = signal('a');
  const node = el('div', { title, id: 'fixed' }, []);
  assert.equal(node.attributes.title, 'a');
  setTitle('b');
  assert.equal(node.attributes.title, 'b');
  assert.equal(node.attributes.id, 'fixed', 'a static attribute must not be rewritten');
});

test('a falsy attribute value removes the attribute', () => {
  const [on, setOn] = signal(true);
  const node = el('div', { hidden: on }, []);
  assert.equal(node.attributes.hidden, '');
  setOn(false);
  assert.equal(node.attributes.hidden, undefined, 'false must remove, not render "false"');
});

test('each renders a keyed list', () => {
  const [items] = signal([{ id: 'a' }, { id: 'b' }]);
  const host = el('div', {}, []);
  host.appendChild(each(items, (i) => i.id, (i) => el('p', {}, [i.id])));
  assert.equal(html(host), '<div><p>a</p><p>b</p></div>');
});

// The reason keys are required rather than optional. Without identity a
// reorder destroys and recreates nodes, losing focus, scroll position and
// the contents of any input inside a row — a correctness bug, not a
// performance one.
test('each preserves node identity across a reorder', () => {
  const [items, setItems] = signal([{ id: 'a' }, { id: 'b' }, { id: 'c' }]);
  const host = el('div', {}, []);
  host.appendChild(each(items, (i) => i.id, (i) => el('p', {}, [i.id])));

  const idOf = (label) => host.childNodes.find((n) => html(n) === `<p>${label}</p>`).__id;
  const aBefore = idOf('a');
  const cBefore = idOf('c');

  setItems([{ id: 'c' }, { id: 'a' }, { id: 'b' }]);
  assert.equal(html(host), '<div><p>c</p><p>a</p><p>b</p></div>');
  assert.equal(idOf('a'), aBefore, 'a reordered row must be moved, not rebuilt');
  assert.equal(idOf('c'), cBefore, 'a reordered row must be moved, not rebuilt');
});

test('each removes only what left the list', () => {
  const [items, setItems] = signal([{ id: 'a' }, { id: 'b' }, { id: 'c' }]);
  const host = el('div', {}, []);
  host.appendChild(each(items, (i) => i.id, (i) => el('p', {}, [i.id])));
  const bBefore = host.childNodes.find((n) => html(n) === '<p>b</p>').__id;

  setItems([{ id: 'b' }]);
  assert.equal(html(host), '<div><p>b</p></div>');
  assert.equal(
    host.childNodes.find((n) => html(n) === '<p>b</p>').__id,
    bBefore,
    'the surviving row must be the same node'
  );
});

test('each rejects duplicate keys instead of silently mis-rendering', () => {
  const [items] = signal([{ id: 'a' }, { id: 'a' }]);
  const host = el('div', {}, []);
  let threw = false;
  try {
    host.appendChild(each(items, (i) => i.id, (i) => el('p', {}, [i.id])));
  } catch (e) {
    threw = true;
    assert.ok(String(e.message).includes('Duplicate key'), 'the message must name the problem');
  }
  assert.ok(threw, 'duplicate keys must be an error, not a quiet mis-render');
});

test('when dispatches on the variant tag', () => {
  const [state, setState] = signal(variant('Loading'));
  const host = el('div', {}, []);
  host.appendChild(
    when(state, {
      Loading: () => el('p', {}, ['loading']),
      Ready: (value) => el('p', {}, [value]),
      Failed: (error) => el('p', {}, [error]),
    })
  );
  assert.equal(html(host), '<div><p>loading</p></div>');

  setState(variant('Ready', 'done'));
  assert.equal(html(host), '<div><p>done</p></div>');

  setState(variant('Failed', 'boom'));
  assert.equal(html(host), '<div><p>boom</p></div>');
});

test('an event handler runs and its writes are batched', () => {
  const [count, setCount] = signal(0);
  const [other, setOther] = signal(0);
  let renders = 0;
  const label = derived(() => {
    renders += 1;
    return `${count()}/${other()}`;
  });
  const host = el('div', {}, [text(label)]);
  assert.equal(html(host), '<div>0/0</div>');
  const rendersAfterMount = renders;

  const button = el('button', {
    onClick: () => {
      setCount(1);
      setOther(1);
    },
  }, []);
  button.fire('click');

  assert.equal(html(host), '<div>1/1</div>');
  assert.equal(renders, rendersAfterMount + 1, 'two writes in one handler must recompute once');
});

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
  const tree = Column({}, [Heading(() => 'Title'), Text(name)]);
  assert.equal(tree.tagName, 'div');
  assert.equal(findTag(tree, 'h2') !== null, true, 'Heading renders an h2');
  assert.equal(html(findTag(tree, 'span')), '<span>zd</span>');
});
