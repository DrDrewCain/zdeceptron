// The lifecycle of a `foreign … gives view` — spec §14E.1, §14E.3.
//
// **Its own module, and that is a size decision rather than a tidiness
// one.** A DOM-owning foreign is the one construct here that a program
// can go its whole life without writing, and §16.3.1 promises a bundle
// ships nothing it does not use. Left in `dom.js` these bytes were
// downloaded by every page ever served, including one with no FFI in it,
// which is a fixed cost paid for an optional feature. `Bundle::runtime`
// already computes a transitive import closure — `rpc.js`, `store.js` and
// `wire.js` are linked only when the split finds a crossing or a durable
// key — so this is that existing mechanism applied once more, and not a
// new exemption from the size gate. `zdc-bench` charges this file to the
// programs that link it and to no others, and a test pins both halves of
// that: a null program must not reach it, and a program with a `gives
// view` foreign must.
//
// Nothing here touches the DOM. The node is handed in — the template
// already carries it — so this module needs the reactivity core and
// nothing else, which is why it does not import `dom.js` and why linking
// it costs a program `signal.js` it already had.

import { effect, onCleanup } from './signal.js';

/**
 * Hand an element to a `foreign … gives view` (§14E.1, §14E.3).
 *
 * `node` is a `<div>` the template already carries, so a foreign is a
 * static-markup hole bound like an attribute rather than an anchor pair
 * like `each`, keeping it inside §16.2 R2's cloning model. `props` is a
 * thunk giving a plain object, one property per `takes` argument in
 * order, read inside an effect.
 *
 * Reactivity is `update`, never re-invocation: re-running `create` would
 * rebuild whatever the module owns — a WebGL context, an animation — on
 * every write, the failure this form prevents. Nothing crosses back, and
 * the handle's shape is asserted, not verified (§14E.4).
 */
export function foreign(node, create, props) {
  let handle = null;
  let disposed = false;

  effect(() => {
    // Above the guard, so the edge exists on a run that bails.
    const next = props();
    // Disposal cannot retract a run the flush has already queued:
    // `clearSources` unsubscribes for the future, and a pending run is
    // still in the drain list. Without this, removing an `each` row in
    // the same batch as a write that row read calls `update` on a
    // destroyed handle — a fault with no visible symptom.
    if (disposed) return;
    if (handle === null) {
      handle = create(node, next);
      return;
    }
    handle.update(next);
  });

  // Registered *after* the effect, and the order is load-bearing rather
  // than incidental. `owned` disposes in registration order, and `effect`
  // registers its own unsubscribe when it is created — so putting this
  // second means the binding is unsubscribed before `destroy` runs, and
  // the module is never handed an update it cannot survive. Reversing
  // these two lines reintroduces exactly the fault the `disposed` flag
  // above is the second line of defence against.
  onCleanup(() => {
    disposed = true;
    if (handle !== null) handle.destroy();
  });
}
