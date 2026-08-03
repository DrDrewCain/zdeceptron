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

import { el, text } from './dom.js';

const BASE = {
  column: { display: 'flex', 'flex-direction': 'column', gap: '0.5rem' },
  row: { display: 'flex', 'flex-direction': 'row', gap: '0.5rem', 'align-items': 'center' },
};

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
      case 'label':
        break; // consumed by the element itself
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
  const p = props(args);
  p.style = { ...BASE.column, ...(p.style ?? {}) };
  return el('div', p, children);
}

export function Row(args = {}, children = []) {
  const p = props(args);
  p.style = { ...BASE.row, ...(p.style ?? {}) };
  return el('div', p, children);
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
  return el('label', { style: BASE.row }, [box, text(args.label)]);
}

export function Spinner(args = {}) {
  return el('span', { 'aria-busy': 'true', ...props(args) }, ['…']);
}

export function ErrorBar(args = {}) {
  return el(
    'div',
    {
      role: 'alert',
      style: { color: '#b3151c', border: '1px solid #b3151c', padding: '0.5rem' },
      ...props(args),
    },
    [text(args.message ?? '')]
  );
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
};
