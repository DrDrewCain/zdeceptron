// The JavaScript half of `gauge.zd` — what a `foreign … gives view` is.
//
// This file is not compiled, not generated, and not part of the runtime.
// It is an ordinary ES module, exactly as `three`, `chart.js` or
// `maplibre-gl` would be, and it is here so the example is a whole thing
// rather than a declaration pointing at nothing.
//
// The contract is three lines long:
//
//   mount(node, props) -> { update(props), destroy() }
//
// `node` is a `<div>` the page already contains and this module now owns.
// `props` is a plain object with one property per `takes` argument of the
// declaration, in declaration order. Nothing is returned to ZDeceptron —
// the handle is consumed by the runtime and never becomes a value the
// program can read, which is what keeps the FFI from laundering anything
// back out.
//
// **Why `update` and not re-invocation.** A canvas has a 2D context, and
// a real library would have a WebGL context, a texture cache, a physics
// loop or an animation in flight. Rebuilding those on every signal write
// is not slow, it is wrong: the animation restarts, the cache is cold,
// and on some drivers the contexts are simply exhausted. So the runtime
// calls `mount` once and `update` thereafter, and this module keeps
// everything expensive across updates.

/** How wide the canvas is, in device-independent pixels. */
const WIDTH = 240;
const HEIGHT = 48;

export function mount(node, props) {
  const canvas = document.createElement('canvas');
  canvas.width = WIDTH;
  canvas.height = HEIGHT;
  node.appendChild(canvas);

  // The expensive things, acquired once. `update` never re-acquires them,
  // which is the whole point of the handle being what it is.
  const context = canvas.getContext('2d');

  let current = props;
  // Eased towards the target rather than snapped, so that a write is
  // visibly *animated* — which is also what makes a re-mount on every
  // write obvious if it ever regresses: the animation would restart.
  let drawn = props.level;
  let frame = null;

  function paint() {
    const target = clamp(current.level);
    drawn += (target - drawn) * 0.2;
    if (Math.abs(target - drawn) < 0.1) drawn = target;

    context.clearRect(0, 0, WIDTH, HEIGHT);
    context.fillStyle = '#e6e6e6';
    context.fillRect(0, 16, WIDTH, 16);
    context.fillStyle = drawn > 80 ? '#b23b3b' : '#3b7fb2';
    context.fillRect(0, 16, (WIDTH * drawn) / 100, 16);
    context.fillStyle = '#333';
    context.font = '12px system-ui, sans-serif';
    context.fillText(`${current.label}: ${Math.round(drawn)}%`, 0, 12);

    frame = drawn === clamp(current.level) ? null : requestAnimationFrame(paint);
  }

  paint();

  return {
    update(next) {
      current = next;
      // Only schedule if nothing is already scheduled: two writes in one
      // batch must not queue two animation loops.
      if (frame === null) frame = requestAnimationFrame(paint);
    },
    destroy() {
      // The reason `destroy` exists. Without it this frame keeps being
      // requested after the node has left the document — invisible in the
      // output, invisible in the DOM, and a leak for the life of the page.
      if (frame !== null) cancelAnimationFrame(frame);
      frame = null;
      node.replaceChildren();
    },
  };
}

function clamp(level) {
  return Math.max(0, Math.min(100, level));
}
