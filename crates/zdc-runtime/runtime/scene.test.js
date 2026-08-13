// Tests for the scene rasteriser's geometry. No DOM and no GPU.
//
// Run: `cargo test -p zdc-runtime`
//
// # What is testable here and what is not
//
// Three of the four backends cannot be exercised in this engine: they
// need a canvas, a GL context or an adapter, none of which exist inside a
// pure-Rust JavaScript interpreter. What *can* be tested is everything
// they share, and that is where the bugs are — a backend is a hundred
// lines of API calls around one buffer, while the tessellator is the part
// that can be silently, plausibly wrong.
//
// So: paint inheritance, view box parsing, ear clipping, stroke ribbons,
// and the circle flattener. Every one of them is pure arithmetic, and
// every one of them decides what a reader sees.

test('a view box is four numbers, however they are separated', () => {
  assert.equal(viewBox('0 0 100 50').join(','), '0,0,100,50');
  assert.equal(viewBox('-10,-10, 20 , 20').join(','), '-10,-10,20,20');
});

// A malformed box is not an error a reader can act on, and a scene that
// refuses to draw teaches nobody anything. The fallback is the unit box
// every other drawing tool defaults to.
test('a malformed view box falls back rather than throwing', () => {
  assert.equal(viewBox('').join(','), '0,0,100,100');
  assert.equal(viewBox('1 2 3').join(','), '0,0,100,100');
  assert.equal(viewBox('0 0 0 10').join(','), '0,0,100,100');
  assert.equal(viewBox(null).join(','), '0,0,100,100');
});

test('a group hands its paint down to shapes that do not state one', () => {
  const out = flatten([
    {
      op: 'group',
      stroke: '#f00',
      strokeWidth: 3,
      children: [{ op: 'circle', cx: 0, cy: 0, r: 1 }],
    },
  ]);
  assert.equal(out.length, 1);
  assert.equal(out[0].stroke, '#f00');
  assert.equal(out[0].strokeWidth, 3);
});

test('a shape overrides the group it is in', () => {
  const out = flatten([
    {
      op: 'group',
      stroke: '#f00',
      children: [{ op: 'circle', cx: 0, cy: 0, r: 1, stroke: '#0f0' }],
    },
  ]);
  assert.equal(out[0].stroke, '#0f0');
});

// Opacity is the one inherited property that *composes* rather than
// replaces, which is what SVG does and what anybody nesting a faded group
// inside a faded group expects. Getting this wrong looks right until two
// levels of nesting exist.
test('opacity multiplies through nesting', () => {
  const out = flatten([
    {
      op: 'group',
      opacity: 0.5,
      children: [{ op: 'group', opacity: 0.5, children: [{ op: 'circle', cx: 0, cy: 0, r: 1 }] }],
    },
  ]);
  assert.equal(out[0].opacity, 0.25);
});

test('a group contributes no shape of its own', () => {
  const out = flatten([{ op: 'group', children: [] }]);
  assert.equal(out.length, 0);
});

test('a circle flattens to a closed polygon at its own radius', () => {
  const lines = polylines({ op: 'circle', cx: 10, cy: 20, r: 5 });
  assert.equal(lines.length, 1);
  assert.equal(lines[0].closed, true);
  const points = lines[0].points;
  assert.equal(points.length % 2, 0);
  // Every vertex is on the circle, which is the only property that
  // matters and the one an off-by-one in the angle step would break.
  for (let i = 0; i < points.length; i += 2) {
    const d = Math.hypot(points[i] - 10, points[i + 1] - 20);
    assert.equal(Math.abs(d - 5) < 1e-9, true);
  }
});

test('a segment flattens to its two endpoints, open', () => {
  const lines = polylines({ op: 'line', x1: 0, y1: 1, x2: 2, y2: 3 });
  assert.equal(lines[0].closed, false);
  assert.equal(lines[0].points.join(','), '0,1,2,3');
});

// Subpaths are split before flattening because a single sampled polyline
// would join the end of one to the start of the next: the counter of an
// `o` would be drawn as a line across it.
test('a path with two subpaths splits into two', () => {
  assert.equal(subpaths('M0 0 L1 1 M2 2 L3 3').length, 2);
  assert.equal(subpaths('M0 0 L1 1').length, 1);
  assert.equal(subpaths('M0 0 L1 1 m1 1 l1 1').length, 2);
});

test('a square ear clips to two triangles', () => {
  const triangles = earClip([0, 0, 10, 0, 10, 10, 0, 10]);
  assert.equal(triangles.length, 12);
});

// The winding is normalised before clipping, so the same square wound the
// other way gives the same answer. Without that the ear test rejects
// every candidate and the fill comes out empty.
test('winding does not change the result', () => {
  const clockwise = earClip([0, 0, 0, 10, 10, 10, 10, 0]);
  assert.equal(clockwise.length, 12);
});

test('a concave outline still clips', () => {
  // An L, which has one reflex vertex — the case a fan triangulation
  // gets wrong and an ear clipper is chosen for.
  const triangles = earClip([0, 0, 10, 0, 10, 4, 4, 4, 4, 10, 0, 10]);
  assert.equal(triangles.length, 24);
});

test('fewer than three points is no triangle at all', () => {
  assert.equal(earClip([0, 0, 1, 1]).length, 0);
  assert.equal(earClip([]).length, 0);
});

test('a stroked segment is two triangles the width apart', () => {
  const triangles = ribbon([0, 0, 10, 0], false, 2);
  assert.equal(triangles.length, 12);
  // The ribbon straddles the line, so the extreme y values are +/- half.
  let low = Infinity;
  let high = -Infinity;
  for (let i = 1; i < triangles.length; i += 2) {
    low = Math.min(low, triangles[i]);
    high = Math.max(high, triangles[i]);
  }
  assert.equal(low, -1);
  assert.equal(high, 1);
});

// A closed polyline strokes the segment back to the start as well, which
// is what keeps a circle from having a gap at three o'clock.
test('a closed polyline strokes one more segment than an open one', () => {
  const open = ribbon([0, 0, 10, 0, 10, 10], false, 1).length;
  const closed = ribbon([0, 0, 10, 0, 10, 10], true, 1).length;
  assert.equal(closed - open, 12);
});

test('a zero-length segment contributes nothing rather than NaN', () => {
  const triangles = ribbon([5, 5, 5, 5], false, 2);
  assert.equal(triangles.length, 0);
});
