// Tests for the `foreign … gives view` lifecycle. Run: `cargo test -p zdc-runtime`
//
// `crates/zdc-codegen/tests/foreign_view.rs` drives this module the way a
// program does: it compiles a `.zd` source, runs the emitted bundle, and
// watches create/update/destroy arrive in order. That is the right place
// for the lifecycle, because the lifecycle is a property of the emission
// and the runtime together.
//
// What it cannot reach is the *shape check* at every corner (#239). A
// compiled program imports one export, so a test written through the
// compiler can only vary what that one export happens to be — and the
// interesting cases are a matrix: not callable at all, callable but a
// class, callable but handing back a handle missing one method or the
// other, handing back a primitive. Enumerating that against the emitter
// would be twelve compilations to test one `if`.
//
// So this suite calls `foreign(...)` directly, exactly as the emitted code
// does, and asks what the message said.
//
// `document`, `test` and `assert` come from the harness; `foreign`,
// `owned` and `signal` come from the runtime evaluated in the same scope.

/// Mount `create` under the declaration name `gauge` and return the
/// message it threw, or `''` if it mounted.
///
/// `owned` because `foreign` registers an `onCleanup`, which needs an
/// owner — the emitted code always has one, since a view is rendered
/// inside a root.
function mounting(create) {
  const node = document.createElement('div');
  try {
    owned(() => foreign(node, create, () => ({ level: 1 }), 'gauge'));
  } catch (e) {
    return String(e && e.message ? e.message : e);
  }
  return '';
}

/// A handle that meets the contract, so the refusals below are refusals of
/// what they name rather than of everything.
function conforming() {
  return { update() {}, destroy() {} };
}

test('a conforming module mounts and is not refused', () => {
  assert.equal(mounting(() => conforming()), '');
});

test('every non-callable import is refused, naming the declaration', () => {
  const imported = [
    [5, 'a number'],
    ['mount', 'a string'],
    [{ mount: () => conforming() }, 'an object'],
    [undefined, 'undefined'],
    [null, 'null'],
  ];
  for (const [value, described] of imported) {
    const message = mounting(value);
    assert.ok(message.includes('`gauge`'), 'unnamed declaration for ' + described + ': ' + message);
    assert.ok(message.includes(described), 'wrong description: ' + message);
    assert.ok(
      message.includes('mount(node, props) -> { update(props), destroy() }'),
      'the contract is not stated: ' + message
    );
  }
  assert.equal(imported.length, 5, 'the fixture list shrank without the assertion moving');
});

// A class passes `typeof x === 'function'`, so it is the one shape a
// callability check misses — and it is what every visual library exports.
test('a class is refused before it is called', () => {
  const message = mounting(class Scene {});
  assert.ok(message.includes('`gauge`'), 'unnamed declaration: ' + message);
  assert.ok(message.includes('a class'), 'the message does not say it is a class: ' + message);
  assert.ok(message.includes('`new`'), 'the message does not say why: ' + message);
});

// The distinction is ECMAScript's own — a class constructor's `prototype`
// is non-writable and an ordinary function's is writable — so an ordinary
// factory must not be mistaken for one.
test('an ordinary function is not mistaken for a class', () => {
  assert.equal(
    mounting(function mount() {
      return conforming();
    }),
    ''
  );
  assert.equal(mounting(() => conforming()), '');
});

test('a handle missing either method is refused, naming which', () => {
  const cases = [
    [() => ({ destroy() {} }), 'no `update`'],
    [() => ({ update() {} }), 'no `destroy`'],
    [() => ({}), 'neither `update` nor `destroy`'],
    [() => ({ update: 1, destroy: 2 }), 'neither `update` nor `destroy`'],
  ];
  for (const [create, expected] of cases) {
    const message = mounting(create);
    assert.ok(message.includes('`gauge`'), 'unnamed declaration: ' + message);
    assert.ok(message.includes(expected), 'expected ' + expected + ', got: ' + message);
  }
  assert.equal(cases.length, 4, 'the fixture list shrank without the assertion moving');
});

test('a mount that returns something other than a handle is refused', () => {
  for (const [create, described] of [
    [() => undefined, 'undefined'],
    [() => null, 'null'],
    [() => 7, 'a number'],
  ]) {
    const message = mounting(create);
    assert.ok(message.includes('`gauge`'), 'unnamed declaration: ' + message);
    assert.ok(message.includes(described), 'expected ' + described + ', got: ' + message);
  }
});

// The check is at mount and nowhere else, so a write reaches `update`
// without re-inspecting anything — and `update` is still called with the
// props the write produced, which is what would break if the check had
// been put in the effect's path rather than beside the create.
test('a write after a checked mount still reaches update', () => {
  const node = document.createElement('div');
  const [level, setLevel] = signal(1);
  const seen = [];
  owned(() =>
    foreign(
      node,
      () => ({
        update(next) {
          seen.push(next.level);
        },
        destroy() {},
      }),
      () => ({ level: level() }),
      'gauge'
    )
  );
  setLevel(2);
  setLevel(3);
  assert.equal(seen.join(','), '2,3');
});
