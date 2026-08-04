// Hand-written stand-in for the output of `zdc build examples/counter.zd`.
//
// Source:
//
//   state count   is client Whole starting 0
//   state doubled is client Whole from count * 2
//
//   view
//       Column
//           Heading "Counter"
//           Text count
//           Text doubled
//           Row
//               Button "minus one"  on click: subtract 1 from count
//               Button "plus one"   on click: add 1 to count
//               Button "reset"      on click: set count to 0
//
// The point of this example is `doubled`. There is no dependency array.
// `derived` discovers that it reads `count` by running it and watching the
// read — so extracting `count() * 2` into a helper function could not break
// the reactivity, which is the failure mode Svelte documented when moving
// to runes.

import { signal, derived } from '../signal.js';
import { mount } from '../dom.js';
import { Column, Row, Heading, Text, Button } from '../elements.js';

// `starting` — a mutable source signal.
const [count, setCount] = signal(0);

// `from` — derived, recomputed when what it read changes.
const doubled = derived(() => count() * 2);

export function main(container) {
  return mount(
    Column(undefined, {}, [
      Heading(() => 'Counter'),
      Text(count),
      Text(doubled),
      Row(undefined, {}, [
        Button(() => 'minus one', { onClick: () => setCount(count() - 1) }),
        Button(() => 'plus one', { onClick: () => setCount(count() + 1) }),
        Button(() => 'reset', { onClick: () => setCount(0) }),
      ]),
    ]),
    container
  );
}
