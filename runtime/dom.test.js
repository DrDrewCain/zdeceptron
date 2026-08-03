// Tests for the renderer. Run: `cargo test -p zdc-runtime`
//
// These execute against a minimal DOM implemented in the test harness, not
// a browser — the point is to catch renderer bugs (a text binding that
// replaces instead of updating, keyed reconciliation moving the wrong
// node, an attribute effect that never detaches) without needing a
// browser or a JavaScript toolchain installed.
//
// `document`, `html`, `serialize`, `findTag`, `test` and `assert` come from
// the harness; `signal`, `derived`, `el`, `text`, `each`, `when`, `variant`,
// `template`, `bindText`, `Column`, `Text`, `Button`, `Input` come from the
// runtime evaluated in the same scope.

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
  host.appendChild(each(items, (i) => i.id, (i) => el('p', {}, [text(() => i().id)])));
  assert.equal(html(host), '<div><p>a</p><p>b</p></div>');
});

// The reason keys are required rather than optional. Without identity a
// reorder destroys and recreates nodes, losing focus, scroll position and
// the contents of any input inside a row — a correctness bug, not a
// performance one.
test('each preserves node identity across a reorder', () => {
  const [items, setItems] = signal([{ id: 'a' }, { id: 'b' }, { id: 'c' }]);
  const host = el('div', {}, []);
  host.appendChild(each(items, (i) => i.id, (i) => el('p', {}, [text(() => i().id)])));

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
  host.appendChild(each(items, (i) => i.id, (i) => el('p', {}, [text(() => i().id)])));
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
    host.appendChild(each(items, (i) => i.id, (i) => el('p', {}, [text(() => i().id)])));
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

// Found by an independent review of the code generator design, not by the
// tests above — which covered reorder and removal but never a row whose
// value changed while its key stayed the same. That is the single most
// common list update there is.
test('each updates a row whose value changed but whose key did not', () => {
  const [items, setItems] = signal([{ id: 'a', label: 'first' }]);
  const host = el('div', {}, []);
  host.appendChild(each(items, (i) => i.id, (i) => el('p', {}, [text(() => i().label)])));
  assert.equal(html(host), '<div><p>first</p></div>');

  setItems([{ id: 'a', label: 'second' }]);
  assert.equal(html(host), '<div><p>second</p></div>', 'a surviving key must still re-render its row');
});

// R3. `props()` consumes `label` but not `message`, so the value lands in
// the attribute loop and paints a bogus `message="..."` onto the div.
test('ErrorBar renders its message as text, not as an attribute', () => {
  const node = ErrorBar({ message: 'boom' });
  assert.equal(html(node).includes('boom'), true, 'the message must be visible');
  assert.equal(node.attributes.message, undefined, 'message must not become an attribute');
});

// R4. `mounted` is documented `key -> { node, dispose }` but nothing ever
// populates or calls `dispose`, so a removed row stays subscribed and
// keeps re-running its bindings for the life of the page.
test('a removed list row stops re-running its bindings', () => {
  const [items, setItems] = signal([{ id: 'a', n: 0 }, { id: 'b', n: 0 }]);
  let bindingRuns = 0;
  const host = el('div', {}, []);
  host.appendChild(
    each(items, (i) => i.id, (i) => el('p', {}, [text(() => { bindingRuns += 1; return i().n; })]))
  );
  const afterMount = bindingRuns;

  setItems([{ id: 'a', n: 1 }]);            // b is removed, a is updated
  const afterRemoval = bindingRuns;
  assert.equal(afterRemoval, afterMount + 1, 'only the surviving row should re-render');

  setItems([{ id: 'a', n: 2 }]);
  assert.equal(bindingRuns, afterRemoval + 1, 'the removed row must not still be subscribed');
});

// R5. `when` was `dynamic(derived(...))`, so any change to the payload
// tore the whole arm down and rebuilt it — including a 1000-row list
// inside it, for one changed cell.
test('when rebuilds an arm only when the variant tag changes', () => {
  const [state, setState] = signal(variant('Ready', 'first'));
  const host = el('div', {}, []);
  host.appendChild(
    when(state, {
      Loading: () => el('p', {}, ['loading']),
      Ready: (value) => el('p', {}, [text(value)]),
      Failed: (error) => el('p', {}, [text(error)]),
    })
  );
  assert.equal(html(host), '<div><p>first</p></div>');
  const nodeBefore = host.childNodes.find((n) => n.kind === 'element').__id;

  setState(variant('Ready', 'second'));
  assert.equal(html(host), '<div><p>second</p></div>');
  assert.equal(
    host.childNodes.find((n) => n.kind === 'element').__id,
    nodeBefore,
    'the same tag must reuse its subtree, not rebuild it'
  );

  setState(variant('Loading'));
  assert.equal(html(host), '<div><p>loading</p></div>');
});

// R5 again, from the other side: an arm that is torn down must also be
// unsubscribed. Its bindings read this `when`'s own `fields` signal, which
// keeps being written, so leaving them alive keeps re-running them against
// nodes that are no longer in the document.
test('when unsubscribes the arm it tears down', () => {
  const [state, setState] = signal(variant('Ready', 'first'));
  let readyRuns = 0;
  const host = el('div', {}, []);
  host.appendChild(
    when(state, {
      Loading: () => el('p', {}, ['loading']),
      Ready: (value) => el('p', {}, [text(() => { readyRuns += 1; return value(); })]),
      Failed: (error) => el('p', {}, [text(error)]),
    })
  );
  setState(variant('Loading'));
  const afterSwitch = readyRuns;

  setState(variant('Loading'));   // no tag change, but `fields` is rewritten
  assert.equal(readyRuns, afterSwitch, 'the torn-down arm must not still be subscribed');
});

// --- the template surface (R2) --------------------------------------------

test('template parses once and clones a fragment per call', () => {
  const make = template('<div class="zd-col"><h2>Title</h2><span> </span></div>');
  const first = make();
  const second = make();

  assert.equal(html(first), '<div class="zd-col"><h2>Title</h2><span> </span></div>');
  assert.equal(html(second), html(first));
  assert.ok(first !== second, 'each call must hand out its own tree');
  assert.equal(
    first.firstChild.__id === second.firstChild.__id,
    false,
    'the clones must not share nodes'
  );
});

// Finding 8 in §16.9: `content.firstChild` silently drops every root but
// the first, and multi-root regions are legal at all three cut points.
test('template keeps every root of a multi-root region', () => {
  const make = template('<span>a</span><span>b</span>');
  const fragment = make();
  assert.equal(fragment.childNodes.length, 2, 'both roots must survive');
  assert.equal(html(fragment), '<span>a</span><span>b</span>');
});

test('template parses comments as the anchors a hole needs', () => {
  const fragment = template('<div><!--#--><!--#--></div>')();
  const div = fragment.firstChild;
  assert.equal(div.childNodes.length, 2);
  assert.equal(div.firstChild.kind, 'comment');
  assert.equal(serialize(fragment), '<div><!--#--><!--#--></div>');
});

test('bindText writes a cloned text node and updates in place', () => {
  const [name, setName] = signal('world');
  const fragment = template('<span> </span>')();
  const span = fragment.firstChild;
  bindText(span.firstChild, name);

  assert.equal(html(fragment), '<span>world</span>');
  const before = span.firstChild.__id;
  setName('Ada');
  assert.equal(html(fragment), '<span>Ada</span>');
  assert.equal(span.firstChild.__id, before, 'the text node must be reused');
});

// R7. Without the comparison, a list re-supply dirties layout for every row
// rather than for the rows that changed.
test('bindText does not write when the value is unchanged', () => {
  const [item, setItem] = signal({ label: 'same', other: 1 });
  const span = el('span', {}, []);
  const node = document.createTextNode('');
  span.appendChild(node);
  bindText(node, () => item().label);

  let writes = 0;
  let stored = node.nodeValue;
  Object.defineProperty(node, 'nodeValue', {
    get() { return stored; },
    set(value) { writes += 1; stored = value; },
  });

  setItem({ label: 'same', other: 2 });
  assert.equal(writes, 0, 'an unchanged value must not be written');
  setItem({ label: 'changed', other: 2 });
  assert.equal(writes, 1, 'a changed value must be written exactly once');
});

test('bindAttr routes value to the property and removes on false', () => {
  const [value, setValue] = signal('typed');
  const [flag, setFlag] = signal(true);
  const fragment = template('<input type="text">')();
  const input = fragment.firstChild;

  bindAttr(input, 'value', value);
  bindAttr(input, 'hidden', flag);
  assert.equal(input.value, 'typed', 'value is a property, not an attribute');
  assert.equal(input.attributes.hidden, '');

  setValue('again');
  setFlag(false);
  assert.equal(input.value, 'again');
  assert.equal(input.attributes.hidden, undefined, 'false must remove the attribute');
});

test('bindStyle writes one CSS property', () => {
  const [pad, setPad] = signal(8);
  const fragment = template('<div></div>')();
  const div = fragment.firstChild;
  bindStyle(div, 'padding', () => `${pad()}px`);

  assert.equal(div.style.properties.padding, '8px');
  setPad(12);
  assert.equal(div.style.properties.padding, '12px');
});

// §16.3.7: the emitter never writes a `batch(...)` wrapper, because `on`
// already is one. Two writes in one handler must therefore recompute once.
test('on batches a handler the way el does', () => {
  const [a, setA] = signal(0);
  const [b, setB] = signal(0);
  let runs = 0;
  const total = derived(() => { runs += 1; return a() + b(); });
  const host = el('div', {}, [text(total)]);
  const afterMount = runs;

  const button = template('<button type="button"></button>')().firstChild;
  on(button, 'click', () => { setA(1); setB(1); });
  button.fire('click');

  assert.equal(html(host), '<div>2</div>');
  assert.equal(runs, afterMount + 1, 'two writes in one handler must recompute once');
});

test('anchors makes an empty region without cloning a template', () => {
  const region = anchors();
  assert.equal(region.childNodes.length, 2);
  assert.equal(region.firstChild.kind, 'comment');
  assert.equal(region.lastChild.kind, 'comment');
  assert.ok(region.firstChild !== region.lastChild, 'a region needs two distinct anchors');
});

test('eachInto fills a region between anchors in a cloned template', () => {
  const [items, setItems] = signal(['a', 'b']);
  const fragment = template('<div><!--#--><!--#--></div>')();
  const div = fragment.firstChild;

  eachInto(div.firstChild, div.lastChild, items, byPosition, (item) => {
    const row = template('<p> </p>')();
    bindText(row.firstChild.firstChild, item);
    return row;
  });

  assert.equal(html(div), '<div><p>a</p><p>b</p></div>');
  setItems(['a', 'b', 'c']);
  assert.equal(html(div), '<div><p>a</p><p>b</p><p>c</p></div>');
  setItems(['z']);
  assert.equal(html(div), '<div><p>z</p></div>');
});

// Positional keying is the interim (§16.6), and it is only honest because
// a surviving key re-supplies its item. Under index keys every operation is
// an update, so this is the one that has to work.
test('eachInto under positional keys updates, removes and prepends correctly', () => {
  const [items, setItems] = signal(['a', 'b', 'c']);
  const region = anchors();
  const host = el('div', {}, []);
  host.appendChild(region);
  eachInto(host.firstChild, host.lastChild, items, byPosition, (item) =>
    el('p', {}, [text(item)])
  );
  assert.equal(html(host), '<div><p>a</p><p>b</p><p>c</p></div>');

  setItems(['a', 'B', 'c']);
  assert.equal(html(host), '<div><p>a</p><p>B</p><p>c</p></div>', 'update');

  setItems(['c', 'b', 'a']);
  assert.equal(html(host), '<div><p>c</p><p>b</p><p>a</p></div>', 'swap');

  setItems(['a', 'c']);
  assert.equal(html(host), '<div><p>a</p><p>c</p></div>', 'remove');

  setItems(['x', 'a', 'c']);
  assert.equal(html(host), '<div><p>x</p><p>a</p><p>c</p></div>', 'prepend');
});

// The case identity keying crashes on. `each tag in tags` over ["a","a"] is
// an ordinary program, and it must render.
test('eachInto under positional keys accepts repeated values', () => {
  const [items, setItems] = signal(['a', 'a']);
  const region = anchors();
  const host = el('div', {}, []);
  host.appendChild(region);
  eachInto(host.firstChild, host.lastChild, items, byPosition, (item) =>
    el('p', {}, [text(item)])
  );
  assert.equal(html(host), '<div><p>a</p><p>a</p></div>');

  setItems(['a', 'b']);
  assert.equal(html(host), '<div><p>a</p><p>b</p></div>');
});

// A row with several roots is legal — `HirEachNode.body` is a node list —
// and it is why `template` returns a fragment and an entry holds a list.
test('eachInto places every root of a multi-root row', () => {
  const [items, setItems] = signal(['a', 'b']);
  const region = anchors();
  const host = el('div', {}, []);
  host.appendChild(region);
  eachInto(host.firstChild, host.lastChild, items, byPosition, (item) => {
    const row = template('<p> </p><span> </span>')();
    bindText(row.firstChild.firstChild, item);
    bindText(row.lastChild.firstChild, () => item().toUpperCase());
    return row;
  });
  assert.equal(html(host), '<div><p>a</p><span>A</span><p>b</p><span>B</span></div>');

  setItems(['b']);
  assert.equal(html(host), '<div><p>b</p><span>B</span></div>', 'both roots must retire together');
});

test('whenInto dispatches inside a cloned template', () => {
  const [state, setState] = signal(variant('Loading'));
  const fragment = template('<div><!--#--><!--#--></div>')();
  const div = fragment.firstChild;

  whenInto(div.firstChild, div.lastChild, state, {
    Loading: () => template('<span aria-busy="true">…</span>')(),
    Ready: (value) => {
      const arm = template('<span> </span>')();
      bindText(arm.firstChild.firstChild, value);
      return arm;
    },
    Failed: (error) => {
      const arm = template('<div role="alert" class="zd-err"><span> </span></div>')();
      bindText(arm.firstChild.firstChild.firstChild, () => error().message);
      return arm;
    },
  });

  assert.equal(html(div), '<div><span aria-busy="true">…</span></div>');
  setState(variant('Ready', 'done'));
  assert.equal(html(div), '<div><span>done</span></div>');
  setState(variant('Failed', { message: 'boom' }));
  assert.equal(html(div), '<div><div role="alert" class="zd-err"><span>boom</span></div></div>');
});

test('ifInto shows a branch and takes it away again', () => {
  const [open, setOpen] = signal(false);
  const fragment = template('<div><!--#--><!--#--></div>')();
  const div = fragment.firstChild;

  ifInto(div.firstChild, div.lastChild, open, () => template('<p>shown</p>')(), null);
  assert.equal(html(div), '<div></div>', 'a false condition renders nothing');

  setOpen(true);
  assert.equal(html(div), '<div><p>shown</p></div>');
  setOpen(false);
  assert.equal(html(div), '<div></div>', 'closing must remove the branch');
});

test('ifInto swaps between two branches', () => {
  const [open, setOpen] = signal(true);
  const fragment = template('<div><!--#--><!--#--></div>')();
  const div = fragment.firstChild;

  ifInto(
    div.firstChild,
    div.lastChild,
    open,
    () => template('<p>yes</p>')(),
    () => template('<p>no</p>')()
  );

  assert.equal(html(div), '<div><p>yes</p></div>');
  setOpen(false);
  assert.equal(html(div), '<div><p>no</p></div>');
});

// The reason `ifInto` tracks the condition's truth rather than rebuilding
// on every run: a branch that is rebuilt loses whatever the bindings
// inside it were showing, and a condition can be written from a signal
// that changes far more often than the answer does.
test('ifInto rebuilds only when the condition changes truth', () => {
  const [count, setCount] = signal(1);
  const fragment = template('<div><!--#--><!--#--></div>')();
  const div = fragment.firstChild;
  let built = 0;

  ifInto(
    div.firstChild,
    div.lastChild,
    () => count() > 0,
    () => {
      built += 1;
      const branch = template('<span> </span>')();
      bindText(branch.firstChild.firstChild, count);
      return branch;
    },
    null
  );

  assert.equal(built, 1);
  assert.equal(html(div), '<div><span>1</span></div>');

  setCount(2);
  assert.equal(built, 1, 'still true, so the branch stands');
  assert.equal(html(div), '<div><span>2</span></div>', 'and its bindings still update');

  setCount(0);
  assert.equal(built, 1);
  assert.equal(html(div), '<div></div>');
});

test('dynamicInto replaces a region between existing anchors', () => {
  const [which, setWhich] = signal('one');
  const fragment = template('<div><!--#--><!--#--></div>')();
  const div = fragment.firstChild;
  dynamicInto(div.firstChild, div.lastChild, () => el('p', {}, [which()]));

  assert.equal(html(div), '<div><p>one</p></div>');
  setWhich('two');
  assert.equal(html(div), '<div><p>two</p></div>');
});

test('byPosition keys a row by its slot', () => {
  assert.equal(byPosition('a', 0), 0);
  assert.equal(byPosition('a', 3), 3);
});

// R6. Base styling is a class, so it is a byte-for-byte comparable string
// rather than a CSSOM serialisation — and it costs no effect at all.
test('Column and Row carry a base class rather than an inline style', () => {
  const column = Column({}, []);
  assert.equal(column.attributes.class, 'zd-col');
  assert.equal(Object.keys(column.style.properties).length, 0, 'no inline style at all');
  assert.equal(Row({}, []).attributes.class, 'zd-row');
  assert.equal(ErrorBar({ message: 'boom' }).attributes.class, 'zd-err');
});

test('a program class is appended to the base class, not replaced', () => {
  assert.equal(Column({ class: 'wide' }, []).attributes.class, 'zd-col wide');

  const [extra, setExtra] = signal('a');
  const reactive = Row({ class: extra }, []);
  assert.equal(reactive.attributes.class, 'zd-row a');
  setExtra('b');
  assert.equal(reactive.attributes.class, 'zd-row b', 'a reactive class must stay reactive');
});
