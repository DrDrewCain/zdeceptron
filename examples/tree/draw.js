// The JavaScript half of `tree.zd` — three.js behind a `foreign … gives view`.
//
// Not compiled, not generated, not part of the runtime. An ordinary ES
// module, which is what the FFI is for.
//
// The contract is the one `examples/gauge.js` documents in full:
//
//     mount(node, props) -> { update(props), destroy() }
//
// `props` has one property per `takes` argument of the declaration, in
// declaration order. Nothing goes back to ZDeceptron.
//
// ## Why `update` matters more here than in the gauge
//
// A WebGL context is not cheap and a browser will only give a page so many
// of them. Re-mounting on every slider move would leak contexts until the
// oldest were force-reclaimed and the canvas went blank — so `mount`
// acquires the renderer, the scene and the camera exactly once, and
// `update` rebuilds only the branch meshes.
//
// The frame loop lives here for the same reason. There is no per-frame
// signal in ZDeceptron and there should not be: a signal that changes
// sixty times a second is not state, it is an animation, and the language
// draws that line deliberately (#189 is the open issue for keyframes).
//
// ## Where three.js comes from
//
// Imported from a CDN as an ES module, because there is no bundler in this
// toolchain and that is the point. For a real deployment, vendor
// `three.module.js` into `assets/` and import it relatively — everything
// under `assets/` is copied into the bundle, so the program keeps working
// with no network at load time.
import * as THREE from 'https://unpkg.com/three@0.180.0/build/three.module.js';

const WIDTH = 720;
const HEIGHT = 420;

/** How far a child leans away from its parent, in radians. */
const LEAN = 0.62;

export function mount(node, props) {
  const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  renderer.setSize(WIDTH, HEIGHT, false);
  renderer.domElement.style.width = '100%';
  renderer.domElement.style.height = 'auto';
  renderer.domElement.style.display = 'block';
  node.appendChild(renderer.domElement);

  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(42, WIDTH / HEIGHT, 0.1, 100);
  camera.position.set(0, 2.6, 7.2);
  camera.lookAt(0, 2.4, 0);

  scene.add(new THREE.HemisphereLight(0xffffff, 0x33402f, 1.15));
  const sun = new THREE.DirectionalLight(0xffffff, 0.85);
  sun.position.set(4, 8, 5);
  scene.add(sun);

  // Acquired once and reused for every branch of every rebuild. A cylinder
  // of unit height along +Y, so a branch is that geometry scaled and
  // oriented — one buffer for thousands of meshes.
  const trunkGeometry = new THREE.CylinderGeometry(0.06, 0.09, 1, 6);
  const leafGeometry = new THREE.SphereGeometry(0.11, 7, 5);
  const barkMaterial = new THREE.MeshStandardMaterial({ color: 0x6b4b3a, roughness: 0.95 });
  const leafMaterial = new THREE.MeshStandardMaterial({ color: 0x5fa855, roughness: 0.7 });

  // The group that turns. Rotating this rather than the camera keeps the
  // lighting fixed relative to the viewer, so the far side genuinely
  // darkens as it goes round.
  let tree = new THREE.Group();
  scene.add(tree);

  let current = props;
  let disposed = false;

  /**
   * Rebuild the branch meshes from four parallel lists.
   *
   * Every branch is placed relative to its parent's tip, and a parent
   * always appears earlier in the list than its children — `tree.zd`
   * grows one level at a time, so that ordering is a property of the
   * structure and not an assumption made here. That is what lets this be
   * one pass with no recursion and no lookup.
   */
  function build(p) {
    tree.clear();

    const parents = Array.from(p.parents ?? []);
    const depths = Array.from(p.depths ?? []);
    const turns = Array.from(p.turns ?? []);
    const lengths = Array.from(p.lengths ?? []);
    const spread = Math.max(1, Number(p.spread) || 1);
    const n = parents.length;
    if (n === 0) return;

    // Where each branch ends, and which way it was pointing when it got
    // there. Index i is filled before any child of i is read, per above.
    const tipX = new Float32Array(n);
    const tipY = new Float32Array(n);
    const tipZ = new Float32Array(n);
    const dirX = new Float32Array(n);
    const dirY = new Float32Array(n);
    const dirZ = new Float32Array(n);

    const up = new THREE.Vector3(0, 1, 0);
    const direction = new THREE.Vector3();
    const parentDir = new THREE.Vector3();
    const axis = new THREE.Vector3();
    const quaternion = new THREE.Quaternion();
    const maxDepth = depths.length ? Math.max(...depths) : 0;

    for (let i = 0; i < n; i += 1) {
      const length = (lengths[i] ?? 0) / 1000;
      const depth = depths[i] ?? 0;

      let startX = 0, startY = 0, startZ = 0;
      if (depth === 0) {
        direction.set(0, 1, 0);
      } else {
        const parent = parents[i] ?? 0;
        startX = tipX[parent]; startY = tipY[parent]; startZ = tipZ[parent];
        parentDir.set(dirX[parent], dirY[parent], dirZ[parent]);

        // Lean away from the parent, then spin that lean around the
        // parent's own direction by `turn / spread` of a full circle. Two
        // rotations rather than spherical coordinates, so the branch stays
        // attached to whatever the parent happened to be doing.
        axis.set(parentDir.z, 0, -parentDir.x);
        if (axis.lengthSq() < 1e-8) axis.set(1, 0, 0);
        axis.normalize();

        direction.copy(parentDir)
          .applyQuaternion(quaternion.setFromAxisAngle(axis, LEAN))
          .applyQuaternion(
            quaternion.setFromAxisAngle(parentDir, (2 * Math.PI * (turns[i] ?? 0)) / spread),
          )
          .normalize();
      }

      tipX[i] = startX + direction.x * length;
      tipY[i] = startY + direction.y * length;
      tipZ[i] = startZ + direction.z * length;
      dirX[i] = direction.x; dirY[i] = direction.y; dirZ[i] = direction.z;

      const branch = new THREE.Mesh(trunkGeometry, barkMaterial);
      branch.scale.set(Math.pow(0.78, depth), length, Math.pow(0.78, depth));
      branch.position.set(
        startX + direction.x * length * 0.5,
        startY + direction.y * length * 0.5,
        startZ + direction.z * length * 0.5,
      );
      branch.quaternion.setFromUnitVectors(up, direction);
      tree.add(branch);

      if (depth === maxDepth && maxDepth > 0) {
        const leaf = new THREE.Mesh(leafGeometry, leafMaterial);
        leaf.position.set(tipX[i], tipY[i], tipZ[i]);
        tree.add(leaf);
      }
    }
  }

  build(current);

  // The circular motion. `speed` is turns per hundred seconds, so the
  // slider moves in numbers a person can read rather than in radians.
  let last = performance.now();
  let frame = 0;
  function tick(now) {
    if (disposed) return;
    const elapsed = (now - last) / 1000;
    last = now;
    tree.rotation.y += elapsed * ((Number(current.speed) || 0) / 100) * Math.PI * 2;
    renderer.render(scene, camera);
    frame = requestAnimationFrame(tick);
  }
  frame = requestAnimationFrame(tick);

  return {
    update(next) {
      // Only the structure is worth rebuilding for. A speed change is read
      // by the frame loop on its next tick, so it costs nothing.
      const changed =
        next.spread !== current.spread ||
        Array.from(next.parents ?? []).length !== Array.from(current.parents ?? []).length;
      current = next;
      if (changed) build(current);
    },
    destroy() {
      disposed = true;
      cancelAnimationFrame(frame);
      trunkGeometry.dispose();
      leafGeometry.dispose();
      barkMaterial.dispose();
      leafMaterial.dispose();
      // Frees the WebGL context rather than waiting for the collector,
      // which is the difference between navigating away ten times and
      // running the browser out of contexts.
      renderer.dispose();
      node.replaceChildren();
    },
  };
}
