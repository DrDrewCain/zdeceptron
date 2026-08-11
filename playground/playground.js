// The playground, which is a text box, a compiler and a service worker.
//
// # What is going on here
//
// `zdc-wasm.wasm` is this compiler's front end and emitter, built for
// `wasm32-wasip1`. It is a filter: a program on standard input, one JSON
// document on standard output. `wasi.js` supplies the thirteen host calls
// that makes possible; nothing else is imported and no glue is generated,
// so the only build step between a checkout and this page is one
// `cargo build`.
//
// Running the result is the other half. `bundle-sw.js` serves the emitted
// bundle from memory, so the iframe loads `index.html` and its relative
// imports over a real origin and satisfies the emitted
// Content-Security-Policy verbatim. Its comment says why nothing simpler
// works.
//
// # Where the sources come from
//
// The examples are fetched from `examples/` in this repository rather than
// pasted in here. A playground with its own copies would drift from the
// programs CI actually compiles, and the first thing a reader does is
// compare the two. The one exception is `A secret in the view`, which is
// deliberately broken and therefore cannot live in `examples/` — CI builds
// everything in that directory.

import { run as runWasm } from './wasi.js';

/// Where the module might be, in the order it is looked for.
///
/// A copy beside this file wins, because that is what a deployment has —
/// the page and the `.wasm` in one directory, nothing above them. Serving
/// this repository from its root has no such copy, so the first probe 404s
/// and the console says why: an unexplained 404 next to a working page is
/// the kind of thing a reader chases for ten minutes.
const WASM_PATHS = [
  './zdc-wasm.wasm',
  '../target/wasm32-wasip1/release/zdc-wasm.wasm',
  '../target/wasm32-wasip1/debug/zdc-wasm.wasm',
];

/// The picker, in two groups: what this page can run, and what it can only
/// compile. The second group is not a list of failures — it is the point.
/// Every entry in it is a program `zdc build` handles and a browser alone
/// cannot, and the reason differs each time.
const EXAMPLES = [
  { group: 'Runs in this tab', items: [
    { file: 'hello.zd', title: 'hello — one signal, one view' },
    { file: 'counter.zd', title: 'counter — derived state, no dependency array' },
    { file: 'todo.zd', title: 'todo — a real client program' },
    { file: 'preferences.zd', title: 'preferences — `remembered`, survives a reload' },
    { file: 'poker.zd', title: 'poker — a hand evaluator, all in the browser' },
  ] },
  { group: 'Compiles here, runs elsewhere', items: [
    { file: 'guestbook.zd', title: 'guestbook — client, server, durable, secret' },
    { file: 'tally.zd', title: 'tally — a durable map, and its two endpoints' },
    { file: 'writing.zd', title: 'writing — `static`, computed at build time' },
    { file: 'blog.zd', title: 'blog — imports a module, and there is no filesystem' },
    { file: 'gauge.zd', title: 'gauge — a `foreign` module this page cannot fetch' },
    { source: LEAK(), title: 'a secret in the view — refused' },
  ] },
];

/// The one program written here rather than fetched.
function LEAK() {
  return [
    '# A secret, and a view that reads it. This is the refusal that makes',
    "# the language's point in one screen: `apiKey` never reaches the",
    '# browser, and that is a property the type system enforces rather',
    '# than a rule anyone remembers to follow.',
    '',
    'secret state apiKey is server Text from environment "GREETING_API_KEY"',
    '',
    'state name is client Text starting ""',
    '',
    'view',
    '    Column',
    '        Heading "Guestbook"',
    '        Input name, hint is "your name"',
    '',
    '        # Delete this line and the program compiles.',
    '        Text apiKey',
    '',
  ].join('\n');
}

const el = (id) => document.getElementById(id);
const source = el('source');
const status = el('status');

let compiler = null;
let controller = null;
let generation = 0;

// --- loading the compiler ------------------------------------------------

async function load() {
  let lastError = null;
  for (const path of WASM_PATHS) {
    try {
      const response = await fetch(path);
      if (!response.ok) {
        lastError = `${path} → ${response.status}`;
        continue;
      }
      // `compileStreaming` needs `application/wasm`, which `python3 -m
      // http.server` does send. The fallback is not defensive padding: a
      // static host that does not is a real thing to meet, and failing
      // there with "incorrect response MIME type" would send a reader
      // looking for a bug in the compiler.
      try {
        return { module: await WebAssembly.compileStreaming(response.clone()), path };
      } catch {
        return { module: await WebAssembly.compile(await response.arrayBuffer()), path };
      }
    } catch (error) {
      lastError = `${path} → ${error.message}`;
    }
  }
  throw new Error(lastError ?? 'no candidate path');
}

async function register() {
  if (!('serviceWorker' in navigator)) return null;
  const registration = await navigator.serviceWorker.register('./bundle-sw.js', { scope: './' });
  await navigator.serviceWorker.ready;
  // `ready` resolves on an active worker, which is not the same as one
  // controlling *this* page: on the very first load the page was fetched
  // before any worker existed. `clients.claim()` in the worker fixes that,
  // and this waits for it to land.
  if (!navigator.serviceWorker.controller) {
    await new Promise((resolve) => {
      navigator.serviceWorker.addEventListener('controllerchange', resolve, { once: true });
      setTimeout(resolve, 3000);
    });
  }
  return registration;
}

// --- compiling -----------------------------------------------------------

/// `focus` decides whether the panel may change under the reader.
///
/// True when a person asked for a compile — the button, the keyboard, a new
/// example — and false for the recompile that follows typing. Switching
/// tabs on every keystroke would take the source out from under someone
/// editing it.
async function compile({ focus = true } = {}) {
  if (!compiler) return;
  const started = performance.now();
  const { stdout, stderr, code } = await runWasm(compiler.module, source.value);
  const elapsed = Math.round(performance.now() - started);

  if (code !== 0 || stdout === '') {
    show('diagnostics');
    el('panel-diagnostics').innerHTML = '';
    el('panel-diagnostics').append(notice(
      'The compiler did not answer',
      `It exited with ${code} and wrote ${stdout.length} bytes. ` +
      (stderr ? `Standard error said: ${stderr}` : 'Standard error was empty.'),
      true,
    ));
    status.innerHTML = '<span class="bad">the compiler did not answer</span>';
    return;
  }

  const answer = JSON.parse(stdout);
  renderDiagnostics(answer);
  renderPlacement(answer.placement);
  renderBundle(answer.bundle);
  // A failed compile always takes the reader to the diagnostics, whether
  // they asked for the compile or only typed: a result panel showing the
  // last program that worked, beside source that does not, is the one
  // state this page must never sit in.
  await renderRun(answer, focus || !answer.ok);

  const reports = count(answer.diagnostics);
  status.innerHTML = answer.ok
    ? `<span class="ok">compiled</span> in ${elapsed} ms · ${answer.bundle.length} files`
    : `<span class="bad">refused</span> in ${elapsed} ms · ${reports} diagnostic${reports === 1 ? '' : 's'}`;
}

/// How many reports the compiler wrote. Each begins at column zero with a
/// level word, and every following line of an `ariadne` report is indented
/// — so counting those openings counts reports without the compiler having
/// to send a number the renderer could disagree with.
function count(text) {
  if (!text) return 0;
  return text.split('\n').filter((line) => /^(Error|Warning|Note):|^\[/.test(line)).length;
}

// --- the four panels -----------------------------------------------------

function renderDiagnostics(answer) {
  const panel = el('panel-diagnostics');
  panel.innerHTML = '';
  const badge = el('diagnostic-count');
  const reports = count(answer.diagnostics);
  badge.hidden = reports === 0;
  badge.textContent = String(reports);
  badge.className = `count${answer.ok ? '' : ' bad'}`;

  if (!answer.diagnostics) {
    panel.append(notice(
      'Nothing to report',
      'The program compiled. Diagnostics here are drawn by `ariadne`, exactly as `zdc check` draws them in a terminal.',
    ));
    return;
  }
  const pre = document.createElement('pre');
  pre.className = 'report';
  pre.textContent = answer.diagnostics.replace(/\n+$/, '');
  panel.append(pre);
}

function renderPlacement(placement) {
  const panel = el('panel-placement');
  panel.innerHTML = '';
  const pad = document.createElement('div');
  pad.className = 'pad';
  panel.append(pad);

  if (!placement) {
    pad.append(notice(
      'No split to show',
      'The compiler stopped before it settled where anything lives. Read the diagnostics first — the placement of a program that does not resolve is not a fact yet.',
      true,
    ));
    return;
  }

  // The table and its legend are one section. Appended separately they
  // would be two, and `.group + .group`'s spacing would put the next
  // heading hard against the last sentence of the legend.
  const state = document.createDocumentFragment();
  state.append(signalTable(placement));
  if (placement.legend.length > 0) {
    const legend = document.createElement('ul');
    legend.className = 'legend';
    for (const entry of placement.legend) {
      const item = document.createElement('li');
      item.append(chip(entry.placement));
      item.append(' ');
      // The sentence comes from `zdc-doc`'s `prose`, which is also what
      // the language server hovers with. Its emphasis is Markdown, and
      // this is the whole of the Markdown that has to survive.
      const span = document.createElement('span');
      span.innerHTML = entry.sentence
        .replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' })[c])
        .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
        .replace(/`(.+?)`/g, '<code>$1</code>');
      item.append(span);
      legend.append(item);
    }
    state.append(legend);
  }
  pad.append(group('Where the state lives', state));

  pad.append(group('Where the network is', endpointTable(placement)));
  pad.append(group('What a deployment would have to provide', provisionTable(placement)));
}

function signalTable(placement) {
  if (placement.signals.length === 0) return sentence('This program declares no state.');
  const table = document.createElement('table');
  table.innerHTML = '<thead><tr><th>Signal</th><th>Type</th><th>Lives</th>'
    + '<th>Read from the browser as</th><th>Line</th></tr></thead>';
  const body = document.createElement('tbody');
  for (const signal of placement.signals) {
    const row = document.createElement('tr');
    row.append(cell(signal.name, 'name'));
    row.append(code(signal.type));
    const lives = document.createElement('td');
    lives.append(chip(signal.placement));
    if (signal.secret) {
      lives.append(' ');
      lives.append(chip('secret'));
    }
    row.append(lives);
    row.append(marked(signal.read));
    row.append(cell(String(signal.line), 'line'));
    body.append(row);
  }
  table.append(body);
  return table;
}

function endpointTable(placement) {
  if (placement.endpoints.length === 0) {
    return sentence('Nothing here crosses a placement boundary, so the compiler derived no endpoints. '
      + 'This program is one bundle and makes no calls of its own.');
  }
  const wrap = document.createDocumentFragment();
  wrap.append(sentence(`The compiler derived ${placement.endpoints.length} endpoint`
    + `${placement.endpoints.length === 1 ? '' : 's'} from the placements above. `
    + 'Nobody wrote them: an edge from one placement to another is the transport.'));
  const table = document.createElement('table');
  table.innerHTML = '<thead><tr><th>Endpoint</th><th>Emitted to</th><th>What it is</th><th>Takes</th></tr></thead>';
  const body = document.createElement('tbody');
  for (const endpoint of placement.endpoints) {
    const row = document.createElement('tr');
    row.append(cell(endpoint.name, 'name'));
    row.append(code(endpoint.file));
    row.append(marked(endpoint.what));
    row.append(code(endpoint.takes.length > 0 ? endpoint.takes.join(', ') : '—'));
    body.append(row);
  }
  table.append(body);
  wrap.append(table);
  return wrap;
}

function provisionTable(placement) {
  const parts = [];
  if (placement.durable.length > 0) {
    parts.push(`a store holding ${placement.durable.map((key) => `\`${key}\``).join(', ')}`);
  }
  if (placement.environment.length > 0) {
    parts.push(`${placement.environment.map((key) => `\`${key}\``).join(', ')} in the environment`);
  }
  if (parts.length === 0) {
    return sentence('Nothing. Every byte this program needs is in the bundle.');
  }
  return marked(`${parts.join(', and ')}. Neither is something you configure — both are read off the placements.`, 'div');
}

function renderBundle(files) {
  const panel = el('panel-bundle');
  panel.innerHTML = '';
  const badge = el('file-count');
  badge.hidden = files.length === 0;
  badge.textContent = String(files.length);
  badge.className = 'count';

  if (files.length === 0) {
    panel.append(notice(
      'No bundle',
      'Nothing was emitted, so there is nothing here. The diagnostics say why.',
      true,
    ));
    return;
  }

  const shell = document.createElement('div');
  shell.className = 'files';
  const nav = document.createElement('nav');
  const view = document.createElement('pre');
  shell.append(nav, view);
  panel.append(shell);

  const select = (index) => {
    view.textContent = files[index].source;
    for (const [at, button] of [...nav.children].entries()) {
      button.setAttribute('aria-current', String(at === index));
    }
  };
  for (const [index, file] of files.entries()) {
    const button = document.createElement('button');
    button.type = 'button';
    button.textContent = file.path;
    button.addEventListener('click', () => select(index));
    nav.append(button);
  }
  select(0);
}

async function renderRun(answer, focus) {
  const panel = el('panel-run');
  panel.innerHTML = '';

  if (!answer.run.can) {
    panel.append(notice(
      answer.ok ? 'Compiled, and not run here' : 'Not run',
      answer.run.why,
      true,
    ));
    if (answer.ok) {
      panel.append(notice(
        'What to look at instead',
        'The **Placement** tab has the split, and the **Bundle** tab has every file — including the endpoints, which nobody wrote.',
      ));
    }
    if (focus) show(answer.ok ? 'placement' : 'diagnostics');
    return;
  }

  if (!controller) {
    panel.append(notice(
      'No service worker',
      'This page serves the emitted bundle from a service worker, so the run satisfies the same Content-Security-Policy a deployment ships. '
      + 'Without one there is nowhere to serve it from. Load this page over `http://localhost` rather than `file://`.',
      true,
    ));
    if (focus) show('run');
    return;
  }

  generation += 1;
  const files = Object.fromEntries(answer.bundle.map((file) => [file.path, file.source]));
  await handOver(generation, files);

  const frame = document.createElement('iframe');
  frame.title = 'the compiled program, running';
  frame.src = new URL(`run/${generation}/index.html`, window.location.href).href;
  panel.append(frame);
  if (focus) show('run');
}

/// Give the worker the bundle and wait for it to say so.
///
/// Navigating in the same turn as the `postMessage` is a race, and losing
/// it means the iframe asks for `index.html` before the worker has one —
/// which shows up as an empty frame with a 404 in a panel nobody has open.
function handOver(id, files) {
  return new Promise((resolve) => {
    const listener = (event) => {
      if (event.data && event.data.type === 'ready' && event.data.id === id) {
        navigator.serviceWorker.removeEventListener('message', listener);
        resolve();
      }
    };
    navigator.serviceWorker.addEventListener('message', listener);
    // The worker controlling *this* page, if there is one. `active` is the
    // fallback for the first load, where a worker exists and has not taken
    // control yet.
    const worker = navigator.serviceWorker.controller ?? controller.active;
    worker.postMessage({ type: 'bundle', id, files });
    setTimeout(resolve, 2000);
  });
}

// --- small builders ------------------------------------------------------

function group(heading, content) {
  const section = document.createElement('section');
  section.className = 'group';
  const title = document.createElement('h2');
  title.className = 'section';
  title.textContent = heading;
  section.append(title, content);
  return section;
}

function sentence(text) {
  const p = document.createElement('p');
  p.textContent = text;
  return p;
}

function cell(text, className) {
  const td = document.createElement('td');
  td.textContent = text;
  if (className) td.className = className;
  return td;
}

function code(text) {
  const td = document.createElement('td');
  const tag = document.createElement('code');
  tag.textContent = text;
  td.append(tag);
  return td;
}

/// A cell whose text came from the compiler and carries backticks.
///
/// The escape runs **first** and the two replacements only ever insert
/// `<strong>` and `<code>`, so nothing a program can be named survives as
/// markup: an identifier containing `<` arrives as `&lt;`, and neither
/// backticks nor `**` are spellable in one. That ordering is the whole of
/// the safety argument, which is why it is not left implicit.
function marked(text, tag = 'td') {
  const node = document.createElement(tag);
  node.innerHTML = text
    .replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' })[c])
    .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
    .replace(/`(.+?)`/g, '<code>$1</code>');
  return node;
}

function chip(word) {
  const span = document.createElement('span');
  span.className = `chip ${word}`;
  span.textContent = word;
  return span;
}

function notice(heading, text, refuse = false) {
  const box = document.createElement('div');
  box.className = refuse ? 'notice refuse' : 'notice';
  const title = document.createElement('h3');
  title.textContent = heading;
  box.append(title, marked(text, 'p'));
  return box;
}

function show(name) {
  for (const tab of document.querySelectorAll('#tabs .tab')) {
    const chosen = tab.dataset.panel === name;
    tab.setAttribute('aria-selected', String(chosen));
    el(`panel-${tab.dataset.panel}`).hidden = !chosen;
  }
}

// --- wiring --------------------------------------------------------------

function fillPicker() {
  const picker = el('examples');
  for (const { group: name, items } of EXAMPLES) {
    const optgroup = document.createElement('optgroup');
    optgroup.label = name;
    for (const item of items) {
      const option = document.createElement('option');
      option.value = item.file ?? '';
      option.textContent = item.title;
      if (item.source) option.dataset.source = item.source;
      optgroup.append(option);
    }
    picker.append(optgroup);
  }
  picker.addEventListener('change', () => choose(picker.selectedOptions[0]));
}

async function choose(option) {
  if (option.dataset.source !== undefined) {
    source.value = option.dataset.source;
    el('source-label').textContent = 'playground.zd';
  } else {
    const response = await fetch(`../examples/${option.value}`);
    if (!response.ok) {
      source.value = `# ../examples/${option.value} was not served (${response.status}).\n`
        + '# Start the server at the repository root:\n'
        + '#     python3 -m http.server 8000\n'
        + '# then open http://localhost:8000/playground/\n';
    } else {
      source.value = await response.text();
    }
    el('source-label').textContent = option.value;
  }
  await compile();
}

document.addEventListener('keydown', (event) => {
  if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
    event.preventDefault();
    compile();
  }
});
el('compile').addEventListener('click', () => compile());
for (const tab of document.querySelectorAll('#tabs .tab')) {
  tab.addEventListener('click', () => show(tab.dataset.panel));
}

// A compile is a few milliseconds, so typing recompiles. The delay is for
// the half-typed line rather than for the compiler.
let pending = null;
source.addEventListener('input', () => {
  clearTimeout(pending);
  pending = setTimeout(() => compile({ focus: false }), 400);
});

fillPicker();

// Exposed for the browser test, which needs to know the compiler is up
// before it types anything. A test that polls the DOM for a spinner is a
// test that passes on a fast machine.
window.zdcPlayground = { ready: false, compile };

(async () => {
  try {
    controller = await register();
  } catch (error) {
    // Not fatal. Everything except *running* the program still works, and
    // `renderRun` says so where a reader will see it.
    console.warn(`the bundle service worker did not register: ${error.message}`);
  }
  try {
    compiler = await load();
  } catch (error) {
    status.innerHTML = '<span class="bad">the compiler was not served</span>';
    el('panel-run').append(notice(
      'zdc-wasm.wasm was not found',
      'Build it and serve this directory from the repository root:\n\n'
      + '`cargo build --release --target wasm32-wasip1 -p zdc-wasm`\n\n'
      + '`python3 -m http.server 8000`\n\n'
      + `Then open \`http://localhost:8000/playground/\`. Tried: ${error.message}`,
      true,
    ));
    return;
  }
  el('build').textContent = compiler.path.replace(/^\.\.?\//, '');
  if (compiler.path !== WASM_PATHS[0]) {
    console.info(
      `zdc-wasm: loaded ${compiler.path}. The 404 above is this page looking for `
      + `${WASM_PATHS[0]} first, which is where a deployed copy sits.`,
    );
  }
  await choose(el('examples').selectedOptions[0]);
  window.zdcPlayground.ready = true;
})();
