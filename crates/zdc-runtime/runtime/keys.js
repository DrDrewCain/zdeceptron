// Document key listeners. Run: `cargo test -p zdc-runtime`
//
// Its own module rather than four lines in `dom.js`, for the reason
// `list.js`, `markup.js` and `foreign.js` are their own modules: a program
// that writes no `on key` must not download this (spec §16.3.1). It imports
// `signal.js` and nothing else — it needs a listener and a focus question,
// not a node to render into, so a program whose only DOM work is a shortcut
// does not link the renderer.
//
// # What this file is actually for
//
// `document.addEventListener('keydown', …)` is a strictly larger capability
// than a listener on one element: it receives keystrokes aimed at *every*
// element on the page, including a field the program never declared. A
// password manager's iframe, a `Prose` block holding markup somebody else
// wrote, a third-party embed — the program did not put those characters on
// the screen and has no business reading them.
//
// So the capability is narrowed here, where it is created, rather than
// labelled after the fact:
//
//   1. **The program named its key.** `onKey` compares and returns; a key
//      the program did not name reaches nothing. The compiler emits the
//      literal, so the set of observable keys is written in the source.
//   2. **No editable element had focus.** `isEditable` is what makes (1)
//      safe for a printable key — `on key "r"` cannot see the `r` in a
//      password, because while that password field has focus this listener
//      stands down.
//
// Together: a handler learns only that the key it named itself was pressed
// while nobody was typing into anything. There is no payload, so there is
// nothing else to learn, and there is no argument about what to label.
//
// # The listener is removed
//
// `dom.js`'s `on` never detaches, and it is right not to: the node it is
// attached to is what gets removed. A document listener has no such node.
// One left behind is a leak *and* a correctness bug — it keeps firing into
// a graph whose signals nothing renders any more. `onCleanup` is registered
// against whatever `owned` scope is open, which is the branch closure
// `ifInto`, `whenInto` and `eachInto` already build and already dispose.

import { batch, onCleanup } from './signal.js';

/**
 * Whether a keystroke aimed at `target` is somebody typing into something.
 *
 * Conservative by construction: the question asked is "could this element
 * receive text", and anything unrecognised is treated as if it could not,
 * only because an unrecognised element cannot receive text either — every
 * text-receiving element in HTML is one of these three cases.
 *
 * `type` is not consulted. A `<input type="checkbox">` receives no
 * characters, so suppressing there costs a shortcut and protects nothing —
 * but reading `type` to allow it would be a list of the input types that
 * are safe, and that list fails open the day a new one is added.
 */
function isEditable(target) {
  if (!target || typeof target !== 'object') return false;
  if (target.isContentEditable) return true;
  const tag = typeof target.tagName === 'string' ? target.tagName.toLowerCase() : '';
  return tag === 'input' || tag === 'textarea' || tag === 'select';
}

/**
 * Run `handler` when `key` is pressed and nobody is typing.
 *
 * `key` is compared against `KeyboardEvent.key` exactly, which is why the
 * compiler checks the literal against a closed table: `"Esc"` is a listener
 * that never fires, and a browser reports that as silence.
 *
 * Batched and contained for the reasons `dom.js`'s `on` is: several writes
 * in one handler repaint once, and a throwing handler must not take the
 * page's other listeners with it.
 */
export function onKey(key, handler) {
  const listener = (event) => {
    if (event.key !== key) return;
    if (isEditable(event.target)) return;
    try {
      // No argument, and that is not an oversight. The grammar has no
      // binder, so nothing emitted can read one — passing the event anyway
      // would leave a live channel one lowering change away from being
      // reachable. The event does not leave this function.
      batch(() => handler());
    } catch (failure) {
      reportError(failure);
    }
  };
  document.addEventListener('keydown', listener);
  onCleanup(() => document.removeEventListener('keydown', listener));
}
