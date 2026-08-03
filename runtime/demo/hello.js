// Hand-written stand-in for the output of `zdc build examples/hello.zd`.
//
// The code generator does not exist yet. This file is what it must emit,
// written by hand so the runtime can be verified first — if the runtime is
// wrong, no amount of correct codegen will render anything.
//
// Source:
//
//   state name is client Text starting "world"
//
//   view
//       Column
//           Heading "Hello, ZDeceptron"
//           Input name, hint is "your name"
//           Text name

import { signal } from '../signal.js';
import { mount } from '../dom.js';
import { Column, Heading, Input, Text } from '../elements.js';

// `state name is client Text starting "world"`
// A client source signal compiles to a [read, write] pair.
const name = signal('world');

export function main(container) {
  return mount(
    Column({}, [
      Heading(() => 'Hello, ZDeceptron'),
      // `Input name` binds two-way; the compiler has already proved `name`
      // is client-placed, so the write is local and synchronous.
      Input(name, { hint: 'your name' }),
      // Reading a client signal from the view yields T, not Remote of T —
      // no boundary is crossed, so no `when` is required.
      Text(name[0]),
    ]),
    container
  );
}
