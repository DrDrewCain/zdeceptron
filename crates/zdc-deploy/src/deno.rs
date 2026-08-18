//! Deno Deploy.
//!
//! The only backend of the four with a literal `watch()` — and the one
//! whose `watch()` has the limitation that shaped the store's physical
//! model: **it takes an explicit key list, not a prefix.** A durable `Map`
//! signal lives across as many keys as it has entries, so it cannot be
//! watched directly. The store keeps a version cell per signal, bumped
//! inside the same atomic commit as every write, and watches those — an
//! explicit list the compiler already knows, because it knows every durable
//! key in the program.
//!
//! The platform risk here is not a timeout. There is none documented, and
//! sending response bytes is itself what keeps the app alive. It is
//! eviction: an isolate can be shut down at any time, even while actively
//! receiving traffic, with a `SIGINT` and five seconds before `SIGKILL`.
//! Client reconnect is mandatory rather than advisable.

use crate::capability::{Atomicity, Described, LiveSync};
use crate::{code_lines, File, Options, Program};

const ENTRY: &str = include_str!("../js/deno-entry.js");
const STORE: &str = include_str!("../js/deno-store.js");

pub fn capabilities(_options: &Options) -> Described {
    (
        "`Deno.serve`, with `public/` read from the deployment itself".to_string(),
        LiveSync::Push {
            mechanism: "`Deno.Kv.watch()` on one version key per durable signal, bumped in the \
                        same atomic commit as every write",
        },
        Atomicity::CompareAndSet {
            mechanism: "a versionstamp check with a bounded retry. The native `sum` mutation \
                        works on `Deno.KvU64` — unsigned, 64-bit, wrapping — which can \
                        represent neither a decrement below zero nor ZDeceptron's `Whole` (an \
                        f64, §14A.3), so it is used only for the version cell",
        },
        "not documented. Deno publishes no pricing detail for a held-open stream",
        vec![
            "**An isolate can be evicted at any time, including mid-stream.** `SIGINT`, then \
             five seconds, then `SIGKILL`. Reconnect is not optional."
                .to_string(),
            "Deno KV: key 2048 bytes, value 64 KiB. An atomic operation is capped at 100 \
             checks, 1000 mutations, 800 KiB total."
                .to_string(),
            "`watch()` does not deliver every intermediate state: a key modified several times \
             quickly may produce one notification. The adapter re-reads on every notification, \
             so the value is current even when the count is not."
                .to_string(),
            "Deployment total size should not exceed 1 GB; memory is 512 MB.".to_string(),
            "The new platform runs in 2 regions; Deploy Classic had 6. Classic's shutdown date \
             has passed, so treat any Classic-era documentation — including the only published \
             KV latency table — as stale."
                .to_string(),
            "`Deno.Kv.enqueue()` and `listenQueue()` are **not supported** on the new Deno \
             Deploy. Nothing generated here uses them; do not add them."
                .to_string(),
        ],
        vec![
            "Create the app and deploy it with `deployctl deploy --entrypoint main.js` from \
             the deployment directory, or connect the repository. Nothing here has been \
             deployed, and this tool cannot deploy it."
                .to_string(),
            "Set each environment key in the app's Environment Variables, marked as a secret. \
             `deno.json` names none of them."
                .to_string(),
            "Existing Deno KV data is not migrated from Deploy Classic automatically.".to_string(),
        ],
        (code_lines(ENTRY), code_lines(STORE)),
    )
}

pub fn files(program: &Program<'_>, options: &Options) -> Vec<File> {
    let _ = options;
    vec![
        File::new("main.js", ENTRY),
        File::new("_zd/store.js", STORE),
        File::new("_zd/cache.js", cache_js(program)),
        File::new("deno.json", deno_json(program)),
    ]
}

/// `_zd/cache.js` — which paths may be cached for a year, for the entry
/// that serves them (#137).
///
/// Always generated, including for a program with nothing hashed: the
/// entry imports it, so an absent file is a deployment that does not
/// start. An empty list is a true statement anyway — everything
/// revalidates — and it is the statement the other targets leave to the
/// host's default.
fn cache_js(program: &Program<'_>) -> String {
    let mut paths: Vec<&String> = program.immutable.iter().collect();
    paths.sort();
    paths.dedup();
    let entries: Vec<String> = paths
        .iter()
        .map(|path| format!("  '/{}',\n", escape(path)))
        .collect();
    format!(
        "// zdc · generated, do not edit\n\
         // What the static half may be cached for, by path.\n\
         //\n\
         // A path on this list carries a content hash in its name, so its URL\n\
         // changes whenever its bytes do: a browser that never revisits it is\n\
         // never wrong. Everything else is revalidated on every request, which\n\
         // costs one round trip and returns 304 with no body — the right price\n\
         // for a file whose URL stays the same when its content does not.\n\
         \nconst immutable = new Set([\n{}]);\n\
         \nexport function cacheControl(path) {{\n\
         \x20 return immutable.has(path)\n\
         \x20   ? '{}'\n\
         \x20   : '{}';\n\
         }}\n",
        entries.concat(),
        zdc_codegen::cache::IMMUTABLE,
        zdc_codegen::cache::REVALIDATE,
    )
}

/// A JavaScript single-quoted string body. These are file names the
/// compiler chose, so this only ever has to be correct.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\'', "\\'")
}

fn deno_json(program: &Program<'_>) -> String {
    let _ = program;
    // `--unstable-kv` is still required for `Deno.openKv`, and the config
    // file's `unstable` array is how a deployment says so without a flag.
    String::from(
        "{\n\
         \x20 \"$schema\": \"https://deno.land/x/deno/cli/schemas/config-file.v1.json\",\n\
         \x20 \"unstable\": [\"kv\"],\n\
         \x20 \"tasks\": {\n\
         \x20   \"start\": \"deno run --allow-net --allow-env --allow-read --unstable-kv main.js\"\n\
         \x20 }\n\
         }\n",
    )
}
