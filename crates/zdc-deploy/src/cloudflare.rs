//! Cloudflare Workers: a module worker, a SQLite-backed Durable Object,
//! and static assets.
//!
//! The strongest of the four targets, and the only one where nothing has to
//! be given up. There is no documented hard duration limit on an
//! HTTP-triggered Worker, billing is on CPU time rather than wall clock so
//! a parked stream is close to free, and a Durable Object is simultaneously
//! the atomic store and the push channel — the one place in this crate
//! where `watch` is not bolted on beside the store.

use crate::capability::{Atomicity, Described, LiveSync};
use crate::{code_lines, File, Options, Program, Target, COMPATIBILITY_DATE};

const ENTRY: &str = include_str!("../js/cloudflare-entry.js");
const STORE: &str = include_str!("../js/cloudflare-store.js");

pub fn capabilities(_options: &Options) -> Described {
    (
        "a module worker's `fetch(request, env)`, with `env.ASSETS` serving `public/`".to_string(),
        LiveSync::Push {
            mechanism: "the Durable Object holds the subscribers and broadcasts on write; no \
                        polling and no second system",
        },
        Atomicity::Serialised {
            mechanism: "every Durable Object storage method is implicitly transactional, and \
                        input gates stop requests interleaving across an await",
        },
        "CPU time. Workers pricing states no charge and no limit for duration, so an SSE \
         connection waiting on I/O costs essentially nothing",
        vec![
            "About 1,000 requests per second per Durable Object (soft). This adapter uses one \
             object for the whole store; shard by signal key before approaching it."
                .to_string(),
            "10 GB per SQLite-backed object (1 GB on Free); key and value combined at most 2 MB."
                .to_string(),
            "A Worker must parse and execute its global scope within 1 second. Exceeding it is \
             a **deploy-time rejection** (error 10021), not a runtime error."
                .to_string(),
            "Bundle: 3 MB compressed on Free, 10 MB on Paid, 64 MB uncompressed.".to_string(),
            "CPU time per request: 30 s default on Paid, configurable to 5 minutes; 10 ms on \
             Free."
                .to_string(),
        ],
        vec![
            "Run `wrangler deploy` from the deployment directory. Nothing here has been \
             deployed, and this tool cannot deploy it."
                .to_string(),
            "Create the Durable Object namespace by deploying: the generated `[[migrations]]` \
             entry declares `ZdStore` as a new SQLite class on the first deploy."
                .to_string(),
            "Set each secret with `wrangler secret put`. `wrangler.toml` names them and holds \
             none of them."
                .to_string(),
        ],
        (code_lines(ENTRY), code_lines(STORE)),
    )
}

pub fn files(program: &Program<'_>, options: &Options) -> Vec<File> {
    let mut out = vec![
        File::new("worker.js", ENTRY),
        File::new("_zd/store.js", STORE),
        File::new("wrangler.toml", wrangler(program, options)),
    ];
    // Cache headers, in the file Workers' own static-assets handling reads
    // (#137). It sits *inside* the assets directory rather than beside
    // `wrangler.toml`, because that is where `env.ASSETS` looks — and
    // `wrangler.toml` has no `[headers]` table to put this in, so a rule
    // written there would be a rule nothing applies.
    if let Some(headers) = zdc_codegen::cache::headers(program.immutable) {
        out.push(File::new(
            format!("{}/_headers", Target::Cloudflare.browser_root()),
            headers,
        ));
    }
    out
}

fn wrangler(program: &Program<'_>, options: &Options) -> String {
    let mut out = String::from("# zdc · generated, do not edit\n");
    if !program.environment.is_empty() {
        out.push_str(
            "#\n# This program reads the environment keys listed at the end of this file. They \
             are\n# not set here and must not be: run `wrangler secret put <KEY>` for each.\n",
        );
    }
    out.push_str(&format!(
        "\nname = \"{}\"\nmain = \"worker.js\"\ncompatibility_date = \"{COMPATIBILITY_DATE}\"\n",
        slug(&options.app)
    ));
    out.push_str(
        "\n# The browser half of the bundle. `env.ASSETS.fetch` serves it for every path the\n\
         # router does not claim.\n[assets]\ndirectory = \"./public\"\nbinding = \"ASSETS\"\n",
    );
    out.push_str("\n[[durable_objects.bindings]]\nname = \"ZD_STORE\"\nclass_name = \"ZdStore\"\n");
    out.push_str(
        "\n# SQLite-backed, because new key-value-backed namespaces are no longer created.\n\
         [[migrations]]\ntag = \"v1\"\nnew_sqlite_classes = [\"ZdStore\"]\n",
    );
    if !program.environment.is_empty() {
        out.push_str("\n# Secrets, by name only:\n");
        for key in program.environment {
            out.push_str(&format!("#   wrangler secret put {key}\n"));
        }
    }
    out
}

/// A worker name: lowercase, and only what the platform accepts.
pub(crate) fn slug(app: &str) -> String {
    let mut out: String = app
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while out.starts_with('-') {
        out.remove(0);
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("zd-app");
    }
    out
}
