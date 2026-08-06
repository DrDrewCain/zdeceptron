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
//
// THE DIRECTORY OF THE VOCABULARY IS THE EXPORT LIST, and there is no
// object holding one property per element. There was: `BUILTINS`, which
// nothing in the runtime or the compiler read, whose one consumer was a
// test asserting it existed. It was removed rather than kept, because it
// had a measured cost and no benefit. `boa`, the engine both parity
// suites run this file in, aborts the *process* with a Rust-level
// `BorrowMutError` inside its own `Set` builtin once a context crosses an
// allocation threshold — the defect BENCHMARKS.md records as making
// signal fan-out unmeasurable here — and this file sat on that threshold.
// Building the object on demand instead of at load bought about a dozen
// elements and then stopped working too, because the function itself is
// an object holding a reference per element.
//
// Nothing is lost. `element_parity.rs` calls each name in this file
// directly, once per built-in, so an element the compiler knows and this
// file does not export fails there with the name in the message.

import { el, safeUrl, text } from './dom.js';
import { markup } from './markup.js';

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
  prose: 'zd-prose',
  preformatted: 'zd-pre',
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
      case 'exact':
        out.datetime = value;
        break;
      // What the letters stand for. It is `title` in the DOM, and the
      // compiler requires it, because an `abbr` with no expansion is an
      // acronym with nothing behind it.
      case 'expansion':
        out.title = value;
        break;
      // Which control this label names, by its `id`. `for` is a reserved
      // word in two of the three languages this pipeline touches, and it
      // reads as a preposition rather than as a claim.
      case 'controls':
        out.for = value;
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

/**
 * The two layout containers.
 *
 * Both take an optional leading text slot, ratified in §4.4: `Row item.name`
 * is one text node followed by the row's children, exactly as `Button`
 * already is. A row with nothing to say of its own passes `undefined`,
 * which is what a source program with no leading argument compiles to.
 */
export function Column(value, args = {}, children = []) {
  return el(
    'div',
    withBase(props(args), BASE.column),
    value === undefined ? children : [text(value), ...children],
  );
}

export function Row(value, args = {}, children = []) {
  return el(
    'div',
    withBase(props(args), BASE.row),
    value === undefined ? children : [text(value), ...children],
  );
}

export function Text(value, args = {}) {
  return el('span', props(args), [text(value)]);
}

/**
 * A heading, at the level its nesting says.
 *
 * The compiler chooses the tag from how many sectioning elements enclose
 * the heading, so `h1` is what a heading at the top of a document is. This
 * reference implementation has no enclosing context to consult, so it
 * renders the top level, which is the case the parity test compares.
 */
export function Heading(value, args = {}) {
  return el('h1', props(args), [text(value)]);
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

/**
 * A multi-line field, bound the way `Input` is.
 *
 * A `textarea` holds its value as a property rather than as an attribute,
 * which `setAttribute` in `dom.js` already knows; nothing here is special
 * about the binding except the tag.
 */
export function TextArea(binding, args = {}) {
  const [get, set] = binding;
  return el('textarea', {
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

export function Spinner(args = {}) {
  return el('span', { 'aria-busy': 'true', ...props(args) }, ['…']);
}

export function ErrorBar(args = {}) {
  return el('div', withBase({ role: 'alert', ...props(args) }, BASE.error), [
    text(args.message ?? ''),
  ]);
}

// --- structure, text, lists and media --------------------------------------
//
// These carry no base class and no baked-in attribute: they are the
// language's semantic vocabulary, and what they mean is the tag itself.
// Each is written out rather than generated from a table, because the whole
// value of this file is being an *independent* statement of the DOM shape
// that `element_parity.rs` checks the compiler's table against. A table
// here would be the compiler's table again, in JavaScript.

/** A container: everything it shows is nested inside it. */
function group(tag) {
  return (args = {}, children = []) => el(tag, props(args), children);
}

/** An element whose leading argument is one text node, before children. */
function shown(tag) {
  return (value, args = {}, children = []) =>
    el(tag, props(args), value === undefined ? children : [text(value), ...children]);
}

/** An element with no children at all. */
function empty(tag) {
  return (args = {}) => el(tag, props(args));
}

export const Main = group('main');
export const Section = group('section');
export const Article = group('article');
export const Aside = group('aside');
export const Navigation = group('nav');
export const Header = group('header');
export const Footer = group('footer');
export const Address = group('address');
export const Quote = group('blockquote');
export const List = group('ul');
export const NumberedList = group('ol');
export const Terms = group('dl');
export const Figure = group('figure');
export const Fieldset = group('fieldset');
export const Details = group('details');

export const Paragraph = shown('p');
export const Emphasis = shown('em');
export const Strong = shown('strong');
export const Code = shown('code');
export const CodeBlock = shown('pre');
export const Key = shown('kbd');
export const Time = shown('time');
export const Small = shown('small');
export const Mark = shown('mark');
export const Abbreviation = shown('abbr');
export const Label = shown('label');
export const Legend = shown('legend');
export const Summary = shown('summary');
export const Superscript = shown('sup');
export const Subscript = shown('sub');
export const Item = shown('li');
export const Term = shown('dt');
export const Description = shown('dd');
export const Caption = shown('figcaption');

/**
 * A rendered document: markup, parsed as markup.
 *
 * The one built-in whose content is parsed rather than assigned as a text
 * node. It is safe for the reason `dom.js`'s `markup` is safe and for no
 * other: its argument's type is `Markup`, the compiler admits nothing else
 * there, and the only producer of a `Markup` is `build markdown`, which
 * escapes raw HTML and rewrites script-bearing URLs before it returns.
 */
export function Prose(value, args = {}) {
  const p = props(args);
  withBase(p, BASE.prose);
  const node = el('div', p);
  markup(node, typeof value === 'function' ? value() : value);
  return node;
}

export const Divider = empty('hr');
export const Break = empty('br');
export const Canvas = empty('canvas');

/**
 * Preserved whitespace that is not code.
 *
 * A `pre`, as `CodeBlock` is, and told apart by its class: `zd-pre` takes
 * the document's own typeface and lets long lines wrap, which is what a
 * poem or an address block wants and what a listing must not have.
 */
export function Preformatted(value, args = {}, children = []) {
  return el(
    'pre',
    withBase(props(args), BASE.preformatted),
    value === undefined ? children : [text(value), ...children],
  );
}

/** An image. `source` and `alt` are required by the compiler, not here. */
export function Image(args = {}) {
  return el('img', props(args));
}

/**
 * A hyperlink, and routing's one element (spec §14G.2 revision 1).
 *
 * The leading argument is where it goes — §14G.2 writes `Link Home` with
 * the destination first and the content nested under it — and it is
 * filtered, because `setAttribute('href', 'javascript:…')` is script
 * execution that no amount of HTML escaping would have caught.
 *
 * A real anchor with a real `href`, because that is the whole argument:
 * clicking one is a browser navigation, so every navigation is crawlable,
 * works with a middle click, and needs no runtime at all. When the
 * destination is one of the program's routes the compiler has already
 * rendered the URL; nothing here parses a path or matches a pattern.
 */
export function Link(destination, args = {}, children = []) {
  const href =
    typeof destination === 'function' ? () => safeUrl(destination()) : safeUrl(destination);
  return el('a', { href, ...props(args) }, children);
}
