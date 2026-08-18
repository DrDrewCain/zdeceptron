// Taking over a document the build already painted — §16.10's third
// emission mode, issue #208.
//
// # The two modes that already existed, and why neither is this one
//
// A region is emitted as a static HTML string, cloned per instance, and
// walked to compile-time offsets. That is mode one, and it works because
// the emitter produced the very tree it is walking. Mode two is the same
// walk against a container the emitter mounted a clone into.
//
// Mode three walks a tree **the browser's HTML parser built from bytes a
// Rust serialiser wrote**. The shapes agree — the served markup came from
// the same templates — with one exception that is fatal on its own:
//
// > A region is a pair of anchor comments that a template clone leaves
// > ADJACENT, and a prerendered document has the region's rendered content
// > sitting between them.
//
// So `$n.nextSibling`, which the emitted walk uses for a region's closing
// anchor, finds the first served row instead. The first attempt at
// adoption did exactly that and nothing threw: `examples/writing.zd`
// served 55 elements and the client built 52 more over them, and a blog
// listing rendered 14 posts as 28 cards.
//
// # What this module does instead
//
// One pass, before any walk runs: **lift every served region out from
// between its anchors**. The anchors carry `[` and `]` (see `Edge` in
// `view.rs`), so the end of a region is a thing a reader of the served
// bytes can find rather than a thing it has to assume, and lifting the
// content leaves the two anchors adjacent — which is the shape the walk
// was written for. Nothing in the walk, in `eachInto`, in `whenInto` or in
// `ifInto` has to learn about hydration; each region's content is simply
// waiting on its own opening anchor when its binder asks for it.
//
// # Why a mismatch cannot duplicate anything
//
// This is the property that makes the mode safe to ship, and it is a
// consequence of lifting rather than something checked afterwards. Once a
// region's served nodes are **detached**, the only way any of them reaches
// the page again is a binder deciding to put it there. A list served four
// rows and given three items drops one. A conditional served the other
// branch drops the lot and builds. Both lose the build's work — which is
// the thing adoption was saving — and neither can leave a served node in
// the document beside one the client built, because nothing re-inserts it.
//
// The failure mode is therefore "as slow as no prerender at all", never
// "the page holds its contents twice".

/**
 * The document's root, over whatever the build painted into `container`.
 *
 * `build` is called only when the build painted nothing — the prerender is
 * best-effort by design (`prerender.rs` argues why), so a container that is
 * empty is an ordinary case and not an error.
 *
 * Emitted only for a root that has holes in it. A region with no anchors
 * has nothing to lift, so its root is two lines of emitted code that name
 * no module: `a_null_program_links_two_runtime_files` is what keeps that
 * distinction honest, and it is the reason this file is not in every
 * bundle.
 */
export function adopt(container, build) {
  if (container.firstChild === null) container.replaceChildren(build());
  else claimRegions(container);
  return container;
}

/** An anchor comment's data. `nodeType === 8` is a comment. */
const OPEN = '[';
const CLOSE = ']';

/**
 * Lift every region in `parent` out from between its anchors, recursively.
 *
 * Each opening anchor is left holding its region's nodes in a detached
 * fragment on `$region`, and is left **adjacent to its closing anchor** —
 * so a walk emitted against the template finds the same nodes in the same
 * places it would have in a clone.
 *
 * Depth-counted, because a region's content contains whole regions: a row
 * of a list may hold a list of its own, and the first `]` after an `[` is
 * not necessarily the one that closes it.
 *
 * Recursive into the lifted fragment as well as into elements, so that a
 * row's own holes are already lifted by the time the row is adopted. That
 * is what lets a list count its served rows by counting nodes: after this
 * pass a region's fragment holds exactly the template's roots per
 * instance, with everything nested waiting on an anchor of its own.
 *
 * The opening anchor's data may be longer than `[`. `whenInto` and
 * `ifInto` write which branch they rendered into it, so a served document
 * says what it holds rather than leaving the client to assume its own
 * answer was the build's — hence the first character rather than the whole
 * value.
 */
function claimRegions(parent) {
  for (let child = parent.firstChild; child !== null; ) {
    if (child.nodeType === 1) {
      claimRegions(child);
      child = child.nextSibling;
      continue;
    }
    if (child.nodeType !== 8 || child.nodeValue[0] !== OPEN) {
      child = child.nextSibling;
      continue;
    }

    const region = document.createDocumentFragment();
    let depth = 0;
    let node = child.nextSibling;
    while (node !== null) {
      // Read before the append, which moves `node` out of this list.
      const next = node.nextSibling;
      if (node.nodeType === 8) {
        if (node.nodeValue[0] === OPEN) depth += 1;
        else if (node.nodeValue === CLOSE) {
          if (depth === 0) break;
          depth -= 1;
        }
      }
      region.appendChild(node);
      node = next;
    }
    child.$region = region;
    claimRegions(region);

    // `node` is this region's closing anchor, now next to its opening one
    // — or `null` if the served markup had no closing anchor at all, which
    // is a serialiser and parser that disagreed. Nothing is bound to the
    // missing region, so the page renders whatever the binder builds.
    child = node === null ? null : node.nextSibling;
  }
}
