// The built-in view elements.
//
// These are the only elements a program can use until user-defined
// components land (spec §14D). Each is a thin mapping onto DOM structure
// plus a small amount of default styling, so that a program with no style
// declarations still renders as something a person would recognise.
//
// Input elements bind two-way, and only to `client`-placed signals — a
// keystroke must not silently become a network write (spec §14B.5). The
// compiler enforces the placement rule; the runtime just wires the event.

import { el, safeUrl, text } from './dom.js';

// Base styling is a CLASS NAME, not an inline style object (spec §16.2 R6).
// §6 already specifies that styles compile to static CSS with generated
// scoped class names and zero runtime cost; an inline style object costs one
// effect and one `setProperty` per declaration, which is seven of each for a
// `Column` and a `Row` that can never change. The declarations themselves
// live in `base.css`, which `zdc build` copies into `styles.css`.
const BASE = {
  column: 'zd-col',
  row: 'zd-row',
  error: 'zd-err',
};

/**
 * Put a base class in front of whatever class the program asked for.
 *
 * The program's `class` may be a getter, so the join has to stay reactive
 * rather than stringifying a function into the attribute.
 */
function withBase(p, base) {
  const given = p.class;
  if (given === undefined) {
    p.class = base;
  } else if (typeof given === 'function') {
    p.class = () => `${base} ${given()}`;
  } else {
    p.class = `${base} ${given}`;
  }
  return p;
}

/** Split ZDeceptron element arguments into DOM props. */
function props(args = {}) {
  const out = {};
  const style = {};
  for (const [name, value] of Object.entries(args)) {
    switch (name) {
      case 'padding':
        style.padding = typeof value === 'function' ? () => `${value()}px` : `${value}px`;
        break;
      case 'weight':
        style['font-weight'] = value;
        break;
      case 'hint':
        out.placeholder = value;
        break;
      // The ZDeceptron spelling of `src`. Filtered, not merely renamed:
      // an image source is a request the browser issues to whatever host
      // the value names (spec §16.3.5, corrected).
      case 'source':
        out.src = typeof value === 'function' ? () => safeUrl(value()) : safeUrl(value);
        break;
      case 'src':
      case 'href':
        out[name] = typeof value === 'function' ? () => safeUrl(value()) : safeUrl(value);
        break;
      case 'label':
      case 'message':
        break; // consumed by the element itself, never an attribute
      case 'class':
        out.class = value;
        break;
      default:
        out[name] = value;
    }
  }
  if (Object.keys(style).length > 0) out.style = style;
  return out;
}

export function Column(args = {}, children = []) {
  return el('div', withBase(props(args), BASE.column), children);
}

export function Row(args = {}, children = []) {
  return el('div', withBase(props(args), BASE.row), children);
}

export function Text(value, args = {}) {
  return el('span', props(args), [text(value)]);
}

export function Heading(value, args = {}) {
  return el('h2', props(args), [text(value)]);
}

export function Button(label, args = {}, children = []) {
  return el('button', { type: 'button', ...props(args) }, [text(label), ...children]);
}

/**
 * A text input bound two-way to a client signal.
 *
 * `binding` is the [read, write] pair the compiler emits for a `client`
 * signal. Passing a server or durable signal here is a compile error
 * (§14B.5), so the runtime can assume the write is local and synchronous.
 */
export function Input(binding, args = {}) {
  const [get, set] = binding;
  return el('input', {
    type: 'text',
    value: get,
    onInput: (e) => set(e.target.value),
    ...props(args),
  });
}

export function Checkbox(binding, args = {}) {
  const [get, set] = binding;
  const box = el('input', {
    type: 'checkbox',
    checked: get,
    onChange: (e) => set(e.target.checked),
  });
  if (args.label === undefined) return box;
  return el('label', { class: BASE.row }, [box, text(args.label)]);
}

/**
 * An image. `source` and `alt` are both required by the compiler: an image
 * with no alternative text is a hole in the page for a reader who cannot
 * see it.
 */
export function Image(args = {}) {
  return el('img', props(args));
}

/**
 * A real anchor with a real `href`, so a click is a document navigation
 * rather than a signal write (§14G.2 revision 1).
 */
export function Link(args = {}, children = []) {
  return el('a', props(args), children);
}

export function Spinner(args = {}) {
  return el('span', { 'aria-busy': 'true', ...props(args) }, ['…']);
}

export function ErrorBar(args = {}) {
  return el('div', withBase({ role: 'alert', ...props(args) }, BASE.error), [
    text(args.message ?? ''),
  ]);
}

export const BUILTINS = {
  Column,
  Row,
  Text,
  Heading,
  Button,
  Input,
  Checkbox,
  Spinner,
  ErrorBar,
  Image,
  Link,
};
