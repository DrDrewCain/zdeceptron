// The js-framework-benchmark workload, written five ways against one DOM.
//
// The workload is the standard one: create 1,000 rows, replace them, update
// every 10th, select a row, swap two, remove one, clear, then create 10,000
// and append 1,000 more. Every arm below performs the SAME transitions over
// the SAME row objects and must end each step with the same DOM — the
// driver compares a digest across arms and fails if they diverge, so an arm
// cannot look fast by doing less.
//
// The arms:
//
//   zd-positional   what `zdc build` emits: one cloned template per row,
//                   bindings at the holes, reconciled by `eachInto` with
//                   the interim positional key function (spec §16.6).
//   zd-identity     the same emission with the key function `record …
//                   unique` will supply. One argument differs.
//   direct          what a naive code generator would have produced:
//                   nested `elements.js` calls, one runtime call per node
//                   (spec §16.1, the rejected design).
//   vanilla         hand-written JavaScript, node by node, one listener
//                   per row — what js-framework-benchmark's `vanillajs`
//                   entry does.
//   vanilla-tuned   hand-written and hand-tuned: one parsed template
//                   cloned per row, one delegated listener for the whole
//                   list, and direct DOM edits for each operation. It is
//                   told what changed; the reactive arms are not. This is
//                   the floor §14A.2 says we lose to.
//
// React and SolidJS are absent because CI has no network and §8 forbids a
// Node dependency, so they cannot be fetched or run. What is measured here
// is what can be measured honestly without them.

// --- the row shape --------------------------------------------------------
//
// ZDC-ROW-TEMPLATE — this is the compiler's own output for
// `crates/zdc-bench/bench/row.zd`. `tests/fidelity.rs` recompiles that file
// and fails the build if this string, the walk below, or the binding
// sequence below stops matching what the emitter produces.
const ROW_HTML = '<div class="zd-row"><span> </span><button type="button"> </button><button type="button">x</button></div>';

/** Where a row's handlers go. They are never fired; they are allocated. */
const sink = {
  select() {},
  remove() {},
};

// --- arm: ZDeceptron, as emitted -----------------------------------------
//
// A fresh `template` per arm, so each arm pays for parsing its own markup
// once rather than inheriting a parse another arm already did.

function makeZdRow() {
  const $t0 = template(ROW_HTML);
  return function zdRow(item) {
    const cls = () => item().cls;
    const rowId = () => item().id;
    const rowLabel = () => item().label;
    const $r = $t0();
    // ZDC-EMITTED-BEGIN
    const $n0 = $r.firstChild;
    const $n1 = $n0.firstChild;
    const $n2 = $n1.nextSibling;
    const $n3 = $n2.nextSibling;
    bindAttr($n0, 'class', () => 'zd-row ' + (cls)());
    bindText($n1.firstChild, rowId);
    bindText($n2.firstChild, rowLabel);
    on($n2, 'click', () => sink.select(item()));
    on($n3, 'click', () => sink.remove(item()));
    // ZDC-EMITTED-END
    return $r;
  };
}

// --- arm: direct emission -------------------------------------------------
//
// One runtime call per node, nested exactly as the view nests. This is the
// shape §16.1 rejected; it is here so the rejection is measured.

function directRow(item) {
  return Row({ class: () => item().cls }, [
    Text(() => item().id),
    Button(() => item().label, { onClick: () => sink.select(item()) }),
    Button('x', { onClick: () => sink.remove(item()) }),
  ]);
}

// --- arm: hand-written vanilla, node by node ------------------------------

function vanillaRow(row) {
  const div = document.createElement('div');
  div.setAttribute('class', 'zd-row ' + row.cls);

  const span = document.createElement('span');
  const idText = document.createTextNode(String(row.id));
  span.appendChild(idText);
  div.appendChild(span);

  const label = document.createElement('button');
  label.setAttribute('type', 'button');
  const labelText = document.createTextNode(row.label);
  label.appendChild(labelText);
  label.addEventListener('click', () => sink.select(row));
  div.appendChild(label);

  const remove = document.createElement('button');
  remove.setAttribute('type', 'button');
  remove.appendChild(document.createTextNode('x'));
  remove.addEventListener('click', () => sink.remove(row));
  div.appendChild(remove);

  return { node: div, idText, labelText };
}

// --- arm: hand-tuned vanilla ---------------------------------------------
//
// The same trick the compiler uses — parse once, clone per row — plus the
// two things a person does that a general framework cannot: one delegated
// listener for the whole list, and a direct edit per operation because the
// caller already knows what changed.

function makeTunedRow() {
  let proto = null;
  return function tunedRow(row) {
    if (proto === null) {
      const element = document.createElement('template');
      element.innerHTML = ROW_HTML;
      proto = element.content;
    }
    const fragment = proto.cloneNode(true);
    const div = fragment.firstChild;
    const idText = div.firstChild.firstChild;
    const labelText = div.firstChild.nextSibling.firstChild;
    idText.nodeValue = String(row.id);
    labelText.nodeValue = row.label;
    return { node: div, idText, labelText, fragment };
  };
}

// --- the arms -------------------------------------------------------------

function reactiveArm(renderRow, keyOf) {
  let write = null;
  return {
    mount(host) {
      const [list, set] = signal([]);
      write = set;
      const region = anchors();
      const start = region.firstChild;
      const end = region.lastChild;
      host.appendChild(region);
      eachInto(start, end, list, keyOf, renderRow);
    },
    // The reactive arms are handed the new list and nothing else. Working
    // out what changed is the framework's job, which is the whole point of
    // the comparison.
    apply(next) {
      write(next);
    },
  };
}

function vanillaArm() {
  let host = null;
  let rows = [];
  return {
    mount(h) {
      host = h;
    },
    apply(next, hint) {
      switch (hint.kind) {
        case 'create': {
          host.replaceChildren();
          rows = [];
          const fragment = document.createDocumentFragment();
          for (const row of next) {
            const record = vanillaRow(row);
            rows.push(record);
            fragment.appendChild(record.node);
          }
          host.appendChild(fragment);
          break;
        }
        case 'append': {
          const fragment = document.createDocumentFragment();
          for (const row of next.slice(next.length - hint.count)) {
            const record = vanillaRow(row);
            rows.push(record);
            fragment.appendChild(record.node);
          }
          host.appendChild(fragment);
          break;
        }
        case 'update': {
          for (const index of hint.indices) {
            rows[index].labelText.nodeValue = next[index].label;
          }
          break;
        }
        case 'select': {
          if (hint.previous >= 0) {
            rows[hint.previous].node.setAttribute('class', 'zd-row');
          }
          rows[hint.index].node.setAttribute('class', 'zd-row danger');
          break;
        }
        case 'swap': {
          const a = rows[hint.a].node;
          const b = rows[hint.b].node;
          const after = b.nextSibling;
          host.insertBefore(b, a);
          host.insertBefore(a, after);
          const record = rows[hint.a];
          rows[hint.a] = rows[hint.b];
          rows[hint.b] = record;
          break;
        }
        case 'remove': {
          rows[hint.index].node.remove();
          rows.splice(hint.index, 1);
          break;
        }
        case 'clear': {
          host.replaceChildren();
          rows = [];
          break;
        }
        default:
          throw new Error('unknown hint ' + hint.kind);
      }
    },
  };
}

function tunedArm() {
  const tunedRow = makeTunedRow();
  let host = null;
  let rows = [];
  return {
    mount(h) {
      host = h;
      // One listener for every row that will ever exist.
      host.addEventListener('click', () => {});
    },
    apply(next, hint) {
      switch (hint.kind) {
        case 'create': {
          host.replaceChildren();
          rows = [];
          const fragment = document.createDocumentFragment();
          for (const row of next) {
            const record = tunedRow(row);
            rows.push(record);
            fragment.appendChild(record.fragment);
          }
          host.appendChild(fragment);
          break;
        }
        case 'append': {
          const fragment = document.createDocumentFragment();
          for (const row of next.slice(next.length - hint.count)) {
            const record = tunedRow(row);
            rows.push(record);
            fragment.appendChild(record.fragment);
          }
          host.appendChild(fragment);
          break;
        }
        case 'update': {
          for (const index of hint.indices) {
            rows[index].labelText.nodeValue = next[index].label;
          }
          break;
        }
        case 'select': {
          if (hint.previous >= 0) {
            rows[hint.previous].node.setAttribute('class', 'zd-row');
          }
          rows[hint.index].node.setAttribute('class', 'zd-row danger');
          break;
        }
        case 'swap': {
          const a = rows[hint.a].node;
          const b = rows[hint.b].node;
          const after = b.nextSibling;
          host.insertBefore(b, a);
          host.insertBefore(a, after);
          const record = rows[hint.a];
          rows[hint.a] = rows[hint.b];
          rows[hint.b] = record;
          break;
        }
        case 'remove': {
          rows[hint.index].node.remove();
          rows.splice(hint.index, 1);
          break;
        }
        case 'clear': {
          host.replaceChildren();
          rows = [];
          break;
        }
        default:
          throw new Error('unknown hint ' + hint.kind);
      }
    },
  };
}

// --- the data -------------------------------------------------------------
//
// Built once and shared by every arm, so identity keying sees the same
// object identities the other arms see and no arm gets a different list.

let nextRowId = 1;

function build(count) {
  const out = [];
  for (let i = 0; i < count; i += 1) {
    out.push({ id: nextRowId, label: 'row ' + nextRowId, cls: '' });
    nextRowId += 1;
  }
  return out;
}

const FIRST_1K = build(1000);
const SECOND_1K = build(1000);
const TEN_K = build(10000);
const EXTRA_1K = build(1000);

function replaceAt(list, index, row) {
  const out = list.slice();
  out[index] = row;
  return out;
}

const UPDATE_INDICES = [];
for (let i = 0; i < SECOND_1K.length; i += 10) UPDATE_INDICES.push(i);

const UPDATED_1K = SECOND_1K.slice();
for (const index of UPDATE_INDICES) {
  UPDATED_1K[index] = {
    id: SECOND_1K[index].id,
    label: SECOND_1K[index].label + ' !!!',
    cls: SECOND_1K[index].cls,
  };
}

const SELECTED_1K = replaceAt(UPDATED_1K, 500, {
  id: UPDATED_1K[500].id,
  label: UPDATED_1K[500].label,
  cls: 'danger',
});

const SWAPPED_1K = SELECTED_1K.slice();
{
  const a = SWAPPED_1K[1];
  SWAPPED_1K[1] = SWAPPED_1K[998];
  SWAPPED_1K[998] = a;
}

const REMOVED_1K = SWAPPED_1K.slice();
REMOVED_1K.splice(4, 1);

const ELEVEN_K = TEN_K.concat(EXTRA_1K);

const STEPS = [
  { name: 'create 1,000 rows', next: FIRST_1K, hint: { kind: 'create' } },
  { name: 'replace 1,000 rows', next: SECOND_1K, hint: { kind: 'create' } },
  {
    name: 'update every 10th row',
    next: UPDATED_1K,
    hint: { kind: 'update', indices: UPDATE_INDICES },
  },
  {
    name: 'select a row',
    next: SELECTED_1K,
    hint: { kind: 'select', index: 500, previous: -1 },
  },
  { name: 'swap two rows', next: SWAPPED_1K, hint: { kind: 'swap', a: 1, b: 998 } },
  { name: 'remove a row', next: REMOVED_1K, hint: { kind: 'remove', index: 4 } },
  { name: 'clear 999 rows', next: [], hint: { kind: 'clear' } },
  { name: 'create 10,000 rows', next: TEN_K, hint: { kind: 'create' } },
  {
    name: 'append 1,000 to 10,000',
    next: ELEVEN_K,
    hint: { kind: 'append', count: 1000 },
  },
  { name: 'clear 11,000 rows', next: [], hint: { kind: 'clear' } },
];

// --- comparing the arms ---------------------------------------------------

/**
 * A structural digest of the rendered DOM.
 *
 * Comments are skipped: the reactive arms carry two anchor comments the
 * vanilla arms have no reason to create, and that difference is reported
 * in the node counts rather than smuggled into a correctness comparison.
 * `class` is compared as the unordered token set a browser treats it as,
 * so `"zd-row "` and `"zd-row"` are the same class list — which they are.
 */
function digest(node) {
  let hash = 0x811c9dc5;
  const push = (text) => {
    for (let i = 0; i < text.length; i += 1) {
      hash ^= text.charCodeAt(i);
      hash = Math.imul(hash, 0x01000193) >>> 0;
    }
  };
  const visit = (n) => {
    if (n.kind === 'comment') return;
    if (n.kind === 'text') {
      push('#');
      push(n.nodeValue);
      return;
    }
    if (n.kind === 'element') {
      push('<');
      push(n.tagName);
      for (const name of Object.keys(n.attributes).sort()) {
        push(name);
        push('=');
        push(
          name === 'class'
            ? n.attributes[name].split(/\s+/).filter(Boolean).sort().join(' ')
            : n.attributes[name]
        );
      }
    }
    for (const child of n.childNodes) visit(child);
    if (n.kind === 'element') push('>');
  };
  visit(node);
  return hash;
}

function elementCount(node) {
  let total = 0;
  for (const child of node.childNodes) if (child.kind === 'element') total += 1;
  return total;
}

// --- the driver -----------------------------------------------------------

const REPORT = [];

function record(arm, step, host) {
  const counts = snapshot();
  const fields = [
    'rows=' + elementCount(host),
    'digest=' + digest(host),
    'crossings=' + totalCrossings(counts.crossings),
  ];
  for (const key of COUNTERS) {
    if (counts.crossings[key] !== 0) fields.push('cross.' + key + '=' + counts.crossings[key]);
    if (counts.work[key] !== 0) fields.push('work.' + key + '=' + counts.work[key]);
  }
  for (const key of REACTIVE) {
    if (counts.reactive[key] !== 0) fields.push('reactive.' + key + '=' + counts.reactive[key]);
  }
  REPORT.push('RESULT\t' + arm + '\t' + step + '\t' + fields.join(','));
}

function runArm(name, arm) {
  const host = document.createElement('div');

  // Mounting and the first row are measured on their own: parsing the
  // template is a one-time cost paid at the first clone, and charging it
  // to `create 1,000 rows` would misattribute it.
  resetCounts();
  arm.mount(host);
  arm.apply([FIRST_1K[0]], { kind: 'create' });
  record(name, 'mount and render one row', host);

  arm.apply([], { kind: 'clear' });

  for (const step of STEPS) {
    resetCounts();
    arm.apply(step.next, step.hint);
    record(name, step.name, host);
  }
}

runArm('zd-positional', reactiveArm(makeZdRow(), byPosition));
runArm('zd-identity', reactiveArm(makeZdRow(), (item) => item.id));
runArm('direct', reactiveArm(directRow, byPosition));
runArm('vanilla', vanillaArm());
runArm('vanilla-tuned', tunedArm());

REPORT.join('\n');
