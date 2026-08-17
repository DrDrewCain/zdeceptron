// `Scene` — the same drawing, rasterised four ways.
//
// # What this is
//
// `Svg` and `Scene` take the same five children: `Group`, `Path`,
// `Circle`, `Segment`, and each other. The difference is only where the
// pixels come from. `Svg` builds DOM nodes and lets the browser's own
// renderer keep them; `Scene` hands the compiler's draw list to one of
// the backends below and paints a `<canvas>`.
//
// So the program is written once. Which backend runs it is a property of
// the *deployment*, not of the drawing — the same relationship placements
// already have to data. A ring diagram written for `renderer is "svg"`
// becomes a hundred thousand paths on the GPU by editing one word, and
// nothing about the shapes changes.
//
// # Why four and not one
//
//   svg      — retained mode, the DOM keeps the nodes. Accessible, hit
//              testable, inspectable. Falls over around a thousand nodes
//              because every one of them is a DOM node with style.
//   canvas   — immediate mode. ~10k shapes at 60fps. The browser's own
//              path rasteriser, so curves and joins are exactly right.
//   webgl    — tessellated to triangles. ~50k+ at 60fps. Curves are
//              flattened first, so a flatness tolerance replaces exact
//              arcs; at screen scale nobody can see the difference.
//   webgpu   — the same triangles through a modern pipeline, with the
//              vertex buffer written once per repaint instead of per
//              draw call. 100k+ at the display's own rate.
//
// # The GPU is not assumed to exist
//
// `navigator.gpu` is absent on most browsers today, and present-but-
// unusable on some (no adapter, or `requestDevice` rejects on a blocked
// driver). Every one of those is checked before anything is drawn, and a
// failure means the next backend down, not a blank canvas. A program that
// *asks* for `"webgpu"` on a machine without one gets WebGL and one line
// in the console saying so — a drawing that does not appear is a worse
// answer than a drawing that appears by another route.
//
// # The repaint is an effect
//
// The compiler passes the draw list as a *thunk*, not an array. It is
// evaluated inside `effect`, so every signal the drawing reads is tracked
// and any write repaints — the whole scene, which is what a GPU renderer
// does anyway. There is no diffing and no retained scene graph, because
// for immediate-mode backends there is nothing a diff would save.

import { effect, onCleanup } from './signal.js';

const SVG_NS = 'http://www.w3.org/2000/svg';

/// Mount a scene onto `canvas`.
///
/// `options.viewBox` is the SVG spelling — `"minX minY width height"` —
/// so a drawing moves between `Svg` and `Scene` by changing the element
/// name and nothing else. It is parsed once, here, because neither the
/// coordinate space nor the backend can change after the canvas exists.
/// The canvas is sized by CSS and the transform is derived, so a scene
/// scales with its box without the program computing anything.
export function scene(canvas, options, draws) {
  const wanted = options.renderer || 'auto';
  const box = viewBox(options.viewBox);
  // The chosen backend arrives late when it is WebGPU — adapter and
  // device are both promises. Until it does, paint with Canvas 2D so the
  // first frame is never blank; the swap is invisible because both draw
  // the same list.
  let backend = canvas2d(canvas);
  let live = true;

  choose(wanted, canvas).then((chosen) => {
    if (!live || !chosen || chosen === backend) return;
    backend.destroy();
    backend = chosen;
    paint();
  });

  let pending = 0;
  let latest = null;

  function paint() {
    if (!live || latest === null) return;
    backend.draw(latest, box, size(canvas));
  }

  // Repaints are coalesced to the frame for the same reason the scroll
  // signal is: several writes in one turn are one picture, and painting
  // each of them means work the compositor throws away.
  function schedule(list) {
    latest = list;
    if (pending) return;
    pending = requestAnimationFrame(() => {
      pending = 0;
      paint();
    });
  }

  effect(() => schedule(flatten(draws())));

  const observer =
    typeof ResizeObserver === 'function' ? new ResizeObserver(() => paint()) : null;
  if (observer) observer.observe(canvas);

  onCleanup(() => {
    live = false;
    if (pending) cancelAnimationFrame(pending);
    if (observer) observer.disconnect();
    backend.destroy();
  });
}

function viewBox(text) {
  const parts = String(text == null ? '' : text).trim().split(/[\s,]+/).map(Number);
  if (parts.length !== 4 || parts.some((n) => !Number.isFinite(n)) || parts[2] <= 0 || parts[3] <= 0) {
    return [0, 0, 100, 100];
  }
  return parts;
}

function size(canvas) {
  const ratio = Math.min(window.devicePixelRatio || 1, 2);
  const width = Math.max(1, Math.round(canvas.clientWidth * ratio));
  const height = Math.max(1, Math.round(canvas.clientHeight * ratio));
  if (canvas.width !== width) canvas.width = width;
  if (canvas.height !== height) canvas.height = height;
  return [width, height];
}

// --- choosing a backend -----------------------------------------------------

async function choose(wanted, canvas) {
  if (wanted === 'canvas') return null; // already mounted
  if (wanted === 'webgl') return webgl(canvas) || warn('webgl', null);
  if (wanted === 'webgpu') {
    const gpu = await webgpu(canvas);
    if (gpu) return gpu;
    return warn('webgpu', webgl(canvas));
  }
  // `auto`: the fastest thing this machine actually has.
  const gpu = await webgpu(canvas);
  return gpu || webgl(canvas);
}

let warned = false;

function warn(wanted, fallback) {
  if (!warned) {
    warned = true;
    // eslint-disable-next-line no-console
    console.warn(
      `zd: renderer is "${wanted}" but this browser has no usable ${wanted} context; ` +
        `drawing with ${fallback ? 'WebGL' : 'Canvas 2D'} instead.`
    );
  }
  return fallback;
}

/// Is there a GPU this page may actually use?
///
/// Three separate ways to say no, and every one of them happens in the
/// field: the API is missing (most browsers), the adapter is null (no
/// hardware, or a driver on the browser's blocklist), or the device
/// request rejects (the adapter went away between the two calls, which a
/// laptop switching graphics chips really does). None of them is an
/// error worth showing a reader — they mean "use the other backend".
export async function gpuDevice() {
  if (typeof navigator === 'undefined' || !navigator.gpu) return null;
  let adapter = null;
  try {
    adapter = await navigator.gpu.requestAdapter();
  } catch {
    return null;
  }
  if (!adapter) return null;
  try {
    return await adapter.requestDevice();
  } catch {
    return null;
  }
}

// --- the draw list ----------------------------------------------------------
//
// The compiler emits a tree, because `Group` nests and its paint is
// inherited. Everything below wants a flat list with the paint already
// resolved, so the tree is walked once per repaint and flattened. This is
// cheap — it is a few field reads per shape — and it means no backend
// has to know that `Group` exists.

const ROOT_PAINT = { fill: null, stroke: null, strokeWidth: 1, opacity: 1 };

function flatten(nodes, inherited = ROOT_PAINT, out = []) {
  for (const node of nodes) {
    const paint = {
      fill: node.fill !== undefined && node.fill !== null ? node.fill : inherited.fill,
      stroke: node.stroke !== undefined && node.stroke !== null ? node.stroke : inherited.stroke,
      strokeWidth:
        node.strokeWidth !== undefined && node.strokeWidth !== null
          ? node.strokeWidth
          : inherited.strokeWidth,
      opacity:
        node.opacity !== undefined && node.opacity !== null
          ? node.opacity * inherited.opacity
          : inherited.opacity,
    };
    if (node.op === 'group') {
      flatten(node.children || [], paint, out);
    } else {
      out.push({ ...node, ...paint });
    }
  }
  return out;
}

// --- Canvas 2D --------------------------------------------------------------
//
// The reference backend, and the one whose output the other two are
// trying to match. It is also the fallback for everything, so it may not
// itself fail — `getContext('2d')` is the one context every browser that
// runs this language at all is guaranteed to have.

function canvas2d(canvas) {
  const ctx = canvas.getContext('2d');
  const cache = new Map();

  return {
    kind: 'canvas',
    draw(list, box, [width, height]) {
      if (!ctx) return;
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.clearRect(0, 0, width, height);
      const scale = Math.min(width / box[2], height / box[3]);
      ctx.setTransform(scale, 0, 0, scale, -box[0] * scale, -box[1] * scale);
      ctx.lineCap = 'round';
      ctx.lineJoin = 'round';
      for (const shape of list) {
        ctx.globalAlpha = shape.opacity;
        const path = path2d(shape, cache);
        if (!path) continue;
        if (paintable(shape.fill)) {
          ctx.fillStyle = resolve(shape.fill, canvas);
          ctx.fill(path);
        }
        if (paintable(shape.stroke) && shape.strokeWidth > 0) {
          ctx.strokeStyle = resolve(shape.stroke, canvas);
          ctx.lineWidth = shape.strokeWidth;
          ctx.stroke(path);
        }
      }
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.globalAlpha = 1;
    },
    destroy() {
      cache.clear();
    },
  };
}

// A `Path2D` per distinct `d`, kept across repaints. Parsing a path
// string is the expensive part of an immediate-mode frame, and a ring
// diagram redraws the same forty outlines every time a signal moves.
function path2d(shape, cache) {
  if (shape.op === 'path') {
    if (!shape.d) return null;
    let path = cache.get(shape.d);
    if (!path) {
      path = new Path2D(shape.d);
      cache.set(shape.d, path);
    }
    return path;
  }
  const path = new Path2D();
  if (shape.op === 'circle') {
    path.arc(shape.cx, shape.cy, Math.abs(shape.r), 0, Math.PI * 2);
  } else if (shape.op === 'line') {
    path.moveTo(shape.x1, shape.y1);
    path.lineTo(shape.x2, shape.y2);
  }
  return path;
}

function paintable(colour) {
  return colour !== null && colour !== undefined && colour !== 'none' && colour !== '';
}

// --- colour -----------------------------------------------------------------
//
// SVG paint is a CSS colour, and the GPU backends need four floats. The
// browser is the only thing that knows what `oklch(…)` or a custom
// property resolves to, so it is asked: a 1×1 context normalises any
// colour it accepts to `#rrggbb`, `#rrggbbaa` or `rgba(…)`, and
// `currentColor` is read off the canvas's own computed style, which is
// what the SVG backend would have done.

let probe = null;

function resolve(colour, canvas) {
  if (colour === 'currentColor') {
    return getComputedStyle(canvas).color;
  }
  return colour;
}

const rgbaCache = new Map();

function rgba(colour, canvas) {
  const text = resolve(colour, canvas);
  const hit = rgbaCache.get(text);
  if (hit) return hit;
  if (!probe) probe = document.createElement('canvas').getContext('2d');
  probe.fillStyle = '#000';
  probe.fillStyle = text;
  const normal = probe.fillStyle;
  let out = [0, 0, 0, 1];
  if (normal.startsWith('#')) {
    const hex = normal.slice(1);
    const wide = hex.length > 6;
    out = [
      parseInt(hex.slice(0, 2), 16) / 255,
      parseInt(hex.slice(2, 4), 16) / 255,
      parseInt(hex.slice(4, 6), 16) / 255,
      wide ? parseInt(hex.slice(6, 8), 16) / 255 : 1,
    ];
  } else {
    const parts = normal.match(/[\d.]+/g);
    if (parts) {
      out = [+parts[0] / 255, +parts[1] / 255, +parts[2] / 255, parts[3] === undefined ? 1 : +parts[3]];
    }
  }
  rgbaCache.set(text, out);
  return out;
}

// --- tessellation -----------------------------------------------------------
//
// The GPU backends draw triangles and nothing else, so every shape is
// reduced to some. Curves are flattened rather than evaluated, which is
// the trade the whole GPU path is: an arc becomes a polyline fine enough
// that the difference is below a pixel, and after that the geometry is
// the same for a circle, a bézier and a straight line.
//
// The *parser* is the browser's. Writing an SVG path parser is a
// well-known way to be subtly wrong about arc flags for years, and every
// browser ships a correct one behind `SVGPathElement`, whose
// `getPointAtLength` walks the path at any distance. Subpaths are split
// on `M`/`m` first, because a single sampled polyline would join the end
// of one subpath to the start of the next — a hole in a letter would be
// drawn as a line to it.

const flatCache = new Map();

function polylines(shape) {
  if (shape.op === 'circle') {
    const r = Math.abs(shape.r);
    const steps = Math.max(12, Math.min(256, Math.ceil(r * 4)));
    const points = [];
    for (let i = 0; i < steps; i += 1) {
      const a = (i / steps) * Math.PI * 2;
      points.push(shape.cx + Math.cos(a) * r, shape.cy + Math.sin(a) * r);
    }
    return [{ points, closed: true }];
  }
  if (shape.op === 'line') {
    return [{ points: [shape.x1, shape.y1, shape.x2, shape.y2], closed: false }];
  }
  if (shape.op !== 'path' || !shape.d) return [];
  const hit = flatCache.get(shape.d);
  if (hit) return hit;
  const out = [];
  for (const sub of subpaths(shape.d)) {
    const element = document.createElementNS(SVG_NS, 'path');
    element.setAttribute('d', sub);
    let length = 0;
    try {
      length = element.getTotalLength();
    } catch {
      continue;
    }
    if (!(length > 0)) continue;
    const steps = Math.max(2, Math.min(1024, Math.ceil(length / 0.4)));
    const points = [];
    for (let i = 0; i <= steps; i += 1) {
      const p = element.getPointAtLength((i / steps) * length);
      points.push(p.x, p.y);
    }
    out.push({ points, closed: /[zZ]\s*$/.test(sub.trim()) });
  }
  flatCache.set(shape.d, out);
  return out;
}

function subpaths(d) {
  const parts = [];
  let current = '';
  for (let i = 0; i < d.length; i += 1) {
    const c = d[i];
    if ((c === 'M' || c === 'm') && current.trim()) {
      parts.push(current);
      current = '';
    }
    current += c;
  }
  if (current.trim()) parts.push(current);
  return parts;
}

/// A closed polyline as triangles, by ear clipping.
///
/// Not the fastest triangulator there is, and it is the right one here:
/// it is forty lines, it needs no preprocessing, and it handles the
/// concave outlines a flattened path produces. Self-intersecting paths
/// come out wrong, which is the same thing `fill-rule` would have had to
/// settle and no caller in this language has yet asked for.
function earClip(points) {
  const n = points.length / 2;
  if (n < 3) return [];
  const index = [];
  for (let i = 0; i < n; i += 1) index.push(i);
  // Wind counter-clockwise so the ear test has one sign to check.
  if (area(points, index) < 0) index.reverse();
  const out = [];
  let guard = index.length * 3;
  while (index.length > 3 && guard > 0) {
    guard -= 1;
    let clipped = false;
    for (let i = 0; i < index.length; i += 1) {
      const a = index[(i + index.length - 1) % index.length];
      const b = index[i];
      const c = index[(i + 1) % index.length];
      if (!isEar(points, index, a, b, c)) continue;
      out.push(points[a * 2], points[a * 2 + 1], points[b * 2], points[b * 2 + 1], points[c * 2], points[c * 2 + 1]);
      index.splice(i, 1);
      clipped = true;
      break;
    }
    if (!clipped) break;
  }
  if (index.length === 3) {
    for (const i of index) out.push(points[i * 2], points[i * 2 + 1]);
  }
  return out;
}

function area(points, index) {
  let sum = 0;
  for (let i = 0; i < index.length; i += 1) {
    const a = index[i];
    const b = index[(i + 1) % index.length];
    sum += points[a * 2] * points[b * 2 + 1] - points[b * 2] * points[a * 2 + 1];
  }
  return sum / 2;
}

function cross(ax, ay, bx, by, cx, cy) {
  return (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
}

function isEar(points, index, a, b, c) {
  const ax = points[a * 2];
  const ay = points[a * 2 + 1];
  const bx = points[b * 2];
  const by = points[b * 2 + 1];
  const cx = points[c * 2];
  const cy = points[c * 2 + 1];
  if (cross(ax, ay, bx, by, cx, cy) <= 0) return false;
  for (const i of index) {
    if (i === a || i === b || i === c) continue;
    const px = points[i * 2];
    const py = points[i * 2 + 1];
    if (
      cross(ax, ay, bx, by, px, py) >= 0 &&
      cross(bx, by, cx, cy, px, py) >= 0 &&
      cross(cx, cy, ax, ay, px, py) >= 0
    ) {
      return false;
    }
  }
  return true;
}

/// A polyline as a stroked ribbon.
///
/// One quad per segment plus a round-ish join, which is what `lineJoin`
/// is on the 2D backend. Miter joins would need the adjacent segments'
/// normals and a limit, and at the widths a diagram uses the difference
/// is invisible.
function ribbon(points, closed, width) {
  const out = [];
  const half = width / 2;
  const n = points.length / 2;
  const last = closed ? n : n - 1;
  for (let i = 0; i < last; i += 1) {
    const ax = points[i * 2];
    const ay = points[i * 2 + 1];
    const bx = points[((i + 1) % n) * 2];
    const by = points[((i + 1) % n) * 2 + 1];
    const dx = bx - ax;
    const dy = by - ay;
    const length = Math.hypot(dx, dy);
    if (length === 0) continue;
    const nx = (-dy / length) * half;
    const ny = (dx / length) * half;
    out.push(
      ax + nx, ay + ny, bx + nx, by + ny, bx - nx, by - ny,
      ax + nx, ay + ny, bx - nx, by - ny, ax - nx, ay - ny
    );
  }
  return out;
}

/// Every shape in the list as one interleaved `[x, y, r, g, b, a]` buffer.
///
/// One buffer for the whole scene, which is the point: a hundred thousand
/// paths become one upload and one draw call, and the per-shape cost the
/// DOM and Canvas backends pay disappears.
function mesh(list, canvas) {
  const data = [];
  for (const shape of list) {
    const lines = polylines(shape);
    if (!lines.length) continue;
    if (paintable(shape.fill)) {
      const colour = rgba(shape.fill, canvas);
      for (const line of lines) {
        push(data, earClip(line.points), colour, shape.opacity);
      }
    }
    if (paintable(shape.stroke) && shape.strokeWidth > 0) {
      const colour = rgba(shape.stroke, canvas);
      for (const line of lines) {
        push(data, ribbon(line.points, line.closed, shape.strokeWidth), colour, shape.opacity);
      }
    }
  }
  return new Float32Array(data);
}

function push(data, triangles, colour, opacity) {
  for (let i = 0; i < triangles.length; i += 2) {
    data.push(triangles[i], triangles[i + 1], colour[0], colour[1], colour[2], colour[3] * opacity);
  }
}

// --- WebGL ------------------------------------------------------------------

const GL_VERTEX = `
attribute vec2 position;
attribute vec4 colour;
uniform vec4 box;
varying vec4 tint;
void main() {
  vec2 unit = (position - box.xy) / box.zw;
  gl_Position = vec4(unit.x * 2.0 - 1.0, 1.0 - unit.y * 2.0, 0.0, 1.0);
  tint = colour;
}`;

const GL_FRAGMENT = `
precision mediump float;
varying vec4 tint;
void main() { gl_FragColor = vec4(tint.rgb * tint.a, tint.a); }`;

function webgl(canvas) {
  const gl =
    canvas.getContext('webgl2', { antialias: true, alpha: true }) ||
    canvas.getContext('webgl', { antialias: true, alpha: true });
  if (!gl) return null;

  const program = gl.createProgram();
  for (const [type, source] of [
    [gl.VERTEX_SHADER, GL_VERTEX],
    [gl.FRAGMENT_SHADER, GL_FRAGMENT],
  ]) {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) return null;
    gl.attachShader(program, shader);
  }
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) return null;

  const buffer = gl.createBuffer();
  const position = gl.getAttribLocation(program, 'position');
  const colour = gl.getAttribLocation(program, 'colour');
  const box = gl.getUniformLocation(program, 'box');

  return {
    kind: 'webgl',
    draw(list, view, [width, height]) {
      const data = mesh(list, canvas);
      gl.viewport(0, 0, width, height);
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      if (!data.length) return;
      gl.enable(gl.BLEND);
      // Premultiplied, because the fragment shader multiplies: it is the
      // blend that composites overlapping translucent strokes the way the
      // 2D context does rather than darkening where they cross.
      gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
      gl.useProgram(program);
      gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
      gl.bufferData(gl.ARRAY_BUFFER, data, gl.DYNAMIC_DRAW);
      gl.enableVertexAttribArray(position);
      gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 24, 0);
      gl.enableVertexAttribArray(colour);
      gl.vertexAttribPointer(colour, 4, gl.FLOAT, false, 24, 8);
      gl.uniform4f(box, view[0], view[1], view[2], view[3]);
      gl.drawArrays(gl.TRIANGLES, 0, data.length / 6);
    },
    destroy() {
      gl.deleteBuffer(buffer);
      gl.deleteProgram(program);
    },
  };
}

// --- WebGPU -----------------------------------------------------------------

const WGSL = `
struct Uniforms { box: vec4<f32> };
@group(0) @binding(0) var<uniform> u: Uniforms;

struct Out {
  @builtin(position) clip: vec4<f32>,
  @location(0) tint: vec4<f32>,
};

@vertex
fn vs(@location(0) position: vec2<f32>, @location(1) colour: vec4<f32>) -> Out {
  var out: Out;
  let unit = (position - u.box.xy) / u.box.zw;
  out.clip = vec4<f32>(unit.x * 2.0 - 1.0, 1.0 - unit.y * 2.0, 0.0, 1.0);
  out.tint = colour;
  return out;
}

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
  return vec4<f32>(in.tint.rgb * in.tint.a, in.tint.a);
}`;

async function webgpu(canvas) {
  const device = await gpuDevice();
  if (!device) return null;
  const context = canvas.getContext('webgpu');
  if (!context) return null;

  const format = navigator.gpu.getPreferredCanvasFormat();
  context.configure({ device, format, alphaMode: 'premultiplied' });

  const module = device.createShaderModule({ code: WGSL });
  const pipeline = device.createRenderPipeline({
    layout: 'auto',
    vertex: {
      module,
      entryPoint: 'vs',
      buffers: [
        {
          arrayStride: 24,
          attributes: [
            { shaderLocation: 0, offset: 0, format: 'float32x2' },
            { shaderLocation: 1, offset: 8, format: 'float32x4' },
          ],
        },
      ],
    },
    fragment: {
      module,
      entryPoint: 'fs',
      targets: [
        {
          format,
          blend: {
            color: { srcFactor: 'one', dstFactor: 'one-minus-src-alpha' },
            alpha: { srcFactor: 'one', dstFactor: 'one-minus-src-alpha' },
          },
        },
      ],
    },
    primitive: { topology: 'triangle-list' },
  });

  const uniform = device.createBuffer({ size: 16, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
  const bind = device.createBindGroup({
    layout: pipeline.getBindGroupLayout(0),
    entries: [{ binding: 0, resource: { buffer: uniform } }],
  });

  // Grown, never shrunk: a scene whose shape count wobbles frame to
  // frame would otherwise allocate and destroy a buffer per repaint,
  // which is the one allocation a 120fps path cannot afford.
  let vertices = null;
  let capacity = 0;
  let lost = false;
  device.lost.then(() => {
    lost = true;
  });

  return {
    kind: 'webgpu',
    draw(list, view, [width, height]) {
      if (lost) return;
      const data = mesh(list, canvas);
      if (data.byteLength > capacity) {
        if (vertices) vertices.destroy();
        capacity = Math.max(data.byteLength, capacity * 2, 4096);
        vertices = device.createBuffer({
          size: capacity,
          usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
        });
      }
      if (data.length) device.queue.writeBuffer(vertices, 0, data);
      device.queue.writeBuffer(uniform, 0, new Float32Array(view));

      const encoder = device.createCommandEncoder();
      const pass = encoder.beginRenderPass({
        colorAttachments: [
          {
            view: context.getCurrentTexture().createView(),
            clearValue: { r: 0, g: 0, b: 0, a: 0 },
            loadOp: 'clear',
            storeOp: 'store',
          },
        ],
      });
      if (data.length) {
        pass.setPipeline(pipeline);
        pass.setBindGroup(0, bind);
        pass.setVertexBuffer(0, vertices);
        pass.draw(data.length / 6);
      }
      pass.end();
      device.queue.submit([encoder.finish()]);
      void width;
      void height;
    },
    destroy() {
      if (vertices) vertices.destroy();
      uniform.destroy();
    },
  };
}
