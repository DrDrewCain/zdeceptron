// Variant dispatch and conditional rendering — `when value` and `if cond`.
//
// **Its own module, and that is a size decision rather than a tidiness
// one** — the same decision `list.js`, `foreign.js` and `markup.js` each
// record. A program can go its whole life without writing a `when` or an
// `if`: `hello.zd` does, `counter.zd` does, and so does the null program
// the size gate is measured on. Left in `dom.js` these bytes were
// downloaded by every page ever served, including the ones that can never
// reach them, which is a fixed cost paid for an optional feature.
//
// `Bundle::runtime` already computes a transitive import closure, so this
// is that existing mechanism applied once more rather than a new exemption
// from the gate: a null program must not reach this file, and a program
// with a `when` or an `if` must.
//
// It needs the reactivity core and two functions from `dom.js` —
// `anchors` for the unanchored wrapper and `clearBetween` for the teardown
// — so a program that links this already had both.

import { signal, effect, owned, onCleanup } from './signal.js';
import { anchors, clearBetween } from './dom.js';

/** A value that may be a signal getter or a constant. */
function read(value) {
  return typeof value === 'function' ? value() : value;
}

/**
 * Variant dispatch — `when value` over `Remote`, `Option`, or a `choice`.
 *
 * `arms` maps a variant name to a function receiving that variant's
 * fields positionally. Spec §14G.1.6 requires every arm to be present,
 * so a missing arm is a compiler bug rather than a runtime fallback.
 */
export function when(getter, arms) {
  const fragment = anchors();
  whenInto(fragment.firstChild, fragment.lastChild, getter, arms);
  return fragment;
}

/** Variant dispatch between two existing anchors. */
export function whenInto(start, end, getter, arms) {
  // The arm's payload lives in a signal, and each field is handed to the
  // arm as a getter. So a changed payload flows to the bindings that read
  // it, and only a changed TAG rebuilds the subtree.
  //
  // The earlier implementation was `dynamic(derived(...))`, which rebuilt
  // on any change. Since every list in the language sits inside a `when`
  // arm, one changed cell tore down and recreated the entire list.
  const [fields, setFields] = signal([]);
  let currentTag = null;
  let disposeArm = null;
  // What the build painted between these anchors, lifted out by `adopt.js`
  // before the walk ran and therefore detached (#208). If this dispatch
  // reaches a different arm than the build did, these nodes are dropped
  // rather than left in the page beside the arm the client renders.
  let served = start.$region;
  start.$region = undefined;

  onCleanup(() => disposeArm && disposeArm());

  effect(() => {
    const value = read(getter);
    setFields(value.fields ?? []);
    if (value.tag === currentTag) return;

    const arm = arms[value.tag];
    // $dev
    // Development only (#140). §14G.1.6 makes every arm present, so this
    // states a compiler invariant rather than handling a case: a release
    // build calling `arm(...)` on `undefined` throws too, and #139's
    // containment reports it. What is lost is the sentence, not the
    // failure.
    if (arm === undefined) {
      throw new Error(
        `No arm for variant ${JSON.stringify(value.tag)}. The compiler should have rejected this.`
      );
    }
    // $end
    currentTag = value.tag;
    // The outgoing arm's bindings read this `when`'s own `fields` signal,
    // which keeps being written, so leaving them subscribed would keep
    // running them against detached nodes for the life of the page.
    if (disposeArm !== null) disposeArm();
    clearBetween(start, end);
    // The anchor records which arm is between the anchors, so a served
    // document **says** what it holds. Without that the client would have
    // to assume its own answer was the build's, and the two differ
    // whenever a starting value does — a `remembered` cell read back from
    // the store is the everyday case. Adopting the wrong arm binds a walk
    // to markup it was not written for, which is the failure mode with no
    // compile-time signal that #208 names.
    // `'['` written out rather than named. Every runtime module a program
    // links is flattened into ONE scope by the prerender pass, so a top-level
    // `const` here collides with the one `adopt.js` declares — and a
    // collision there is a `SyntaxError` that turns the prerender off
    // silently, because the pass is best-effort by design.
    const mark = '[' + value.tag;
    const claimed = start.nodeValue === mark ? served : undefined;
    served = undefined;
    start.nodeValue = mark;
    const binders = (value.fields ?? []).map((_, index) => () => fields()[index]);
    const [rendered, dispose] = owned(() => arm(...binders, claimed));
    disposeArm = dispose;
    end.parentNode.insertBefore(rendered, end);
  });
}

/**
 * Conditional rendering between two existing anchors — `if cond`.
 *
 * Not a `whenInto` with two arms: there is no variant here and no `choice`
 * the program declared, so there is no tag to switch on. The branch is
 * rebuilt only when the condition's *truth* changes, for exactly the reason
 * `whenInto` rebuilds only on a tag change — a condition that reads a
 * signal which keeps changing without crossing the boundary would otherwise
 * tear down and recreate the whole subtree on every write.
 *
 * `otherwise` may be null, which renders nothing.
 */
export function ifInto(start, end, condition, render, otherwise) {
  // `null` rather than a boolean, so the first run always renders: neither
  // branch has been built yet, and `false` would look like "already
  // showing the else".
  let current = null;
  let disposeBranch = null;
  // The served branch, lifted out by `adopt.js` and therefore detached.
  // See `whenInto` above for why taking the other branch drops it rather
  // than leaving it in the page.
  let served = start.$region;
  start.$region = undefined;

  onCleanup(() => disposeBranch && disposeBranch());

  effect(() => {
    const taken = Boolean(read(condition));
    if (taken === current) return;
    current = taken;

    // The outgoing branch's bindings read signals that keep being written,
    // so leaving them subscribed would keep running them against detached
    // nodes for the life of the page.
    if (disposeBranch !== null) disposeBranch();
    disposeBranch = null;
    clearBetween(start, end);

    // Which branch the build took, in the anchor, for the reason `whenInto`
    // writes the arm's tag into it. `1` and `0` rather than the words, since
    // this is in the served bytes once per conditional.
    // Written out rather than named, for the reason `whenInto` gives.
    const mark = taken ? '[1' : '[0';
    const claimed = start.nodeValue === mark ? served : undefined;
    served = undefined;
    start.nodeValue = mark;

    const branch = taken ? render : otherwise;
    if (branch === null || branch === undefined) return;
    const [rendered, dispose] = owned(() => branch(claimed));
    disposeBranch = dispose;
    end.parentNode.insertBefore(rendered, end);
  });
}
