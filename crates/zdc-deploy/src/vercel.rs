//! Vercel Functions.
//!
//! The shortest shim of the four, because Vercel's `fetch` Web Standard
//! export is the same shape Cloudflare and Deno use, and the longest list
//! of things this tool will not generate for you — because Vercel's
//! first-party storage is much smaller than the spec assumes.
//!
//! **Vercel KV and Vercel Postgres no longer exist.** Postgres moved to
//! Neon; KV was sunset when Upstash joined the Marketplace. What remains
//! first-party is Blob and Global Config, and Global Config is a 1 MB
//! read-only config channel with up to ten seconds of global propagation —
//! not a store, and certainly not one with `incr`. So this module generates
//! no configuration for a product that no longer exists, and the store
//! binds to Upstash Redis over the Marketplace.

use crate::capability::{stream_budget, Atomicity, Described, LiveSync};
use crate::{code_lines, File, Options, Plan, Program, VercelRuntime};

const ENTRY: &str = include_str!("../js/vercel-entry.js");
const STORE: &str = include_str!("../js/vercel-store.js");

/// The entry, plus the per-function `config` export Vercel reads from the
/// module itself. Appended rather than templated so the shared part stays a
/// file you can read.
fn entry(options: &Options) -> String {
    let mut out = String::from(ENTRY);
    out.push('\n');
    match options.runtime {
        VercelRuntime::Fluid => out.push_str(&format!(
            "export const config = {{ maxDuration: {} }};\n",
            stream_budget(options).ceiling_seconds()
        )),
        // `maxDuration` cannot be set when the runtime is `edge`.
        VercelRuntime::Edge => out.push_str("export const config = { runtime: 'edge' };\n"),
    }
    out
}

pub fn capabilities(options: &Options) -> Described {
    let front = match options.runtime {
        VercelRuntime::Fluid => {
            "`api/index.js`, exporting `default { fetch(request) }` on the Node runtime with \
             Fluid compute. `vercel.json` rewrites `/_zd/*` to it; `public/` is served \
             statically."
        }
        VercelRuntime::Edge => {
            "`api/index.js`, exporting `default { fetch(request) }` on the Edge runtime. Note \
             that Vercel's own documentation now recommends migrating from Edge to Node."
        }
    };

    let mut ceilings = vec![
        match (options.runtime, options.plan) {
            (VercelRuntime::Fluid, Plan::Free) => {
                "Hobby `maxDuration` is 300 s by default *and* by maximum, and includes time \
                 spent streaming. On timeout: `504 FUNCTION_INVOCATION_TIMEOUT`."
            }
            (VercelRuntime::Fluid, Plan::Paid) => {
                "Pro `maxDuration` maximum is 800 s. The 1800 s extension is beta, is \
                 configured per function, and is not supported with Secure Compute or Static \
                 IPs, so it is not generated here."
            }
            (VercelRuntime::Edge, Plan::Free) | (VercelRuntime::Edge, Plan::Paid) => {
                "The Edge runtime must send a first byte within 25 s, and may then stream for \
                 up to 300 s."
            }
        }
        .to_string(),
        "Request and response body payloads cap at 4.5 MB.".to_string(),
        "Upstash Redis over REST: no blocking commands, and no `WATCH`/`UNWATCH`/`DISCARD`."
            .to_string(),
    ];
    match options.runtime {
        VercelRuntime::Fluid => ceilings.push(
            "Bundle: 250 MB uncompressed. Fluid's bytecode caching and pre-warming apply only \
             to production deployments, not preview or development."
                .to_string(),
        ),
        VercelRuntime::Edge => ceilings.push(
            "Bundle, **gzipped**: 1 MB on Hobby, 2 MB on Pro, 4 MB on Enterprise. This is the \
             binding constraint on Edge."
                .to_string(),
        ),
    }

    (
        front.to_string(),
        LiveSync::Poll {
            reason: "Vercel has no first-party pub/sub. Upstash's REST API does expose \
                     `SUBSCRIBE` over SSE, but consuming it means holding a second streamed \
                     connection inside the same `maxDuration` budget as the one being served, \
                     so the adapter re-reads the store instead",
        },
        Atomicity::Native {
            mechanism: "Redis `HINCRBYFLOAT` on one hash field per durable subkey",
        },
        "active CPU plus provisioned memory time, so idle connection time is not charged as \
         compute",
        ceilings,
        vec![
            "Install the store: `vercel install upstash`, then create a Redis database. \
             Nothing here provisions it."
                .to_string(),
            "Set `UPSTASH_REDIS_REST_URL` and `UPSTASH_REDIS_REST_TOKEN` in the project's \
             environment. The Upstash integration usually sets both for you."
                .to_string(),
            "Deploy with `vercel deploy` from the deployment directory. Nothing here has been \
             deployed, and this tool cannot deploy it."
                .to_string(),
            "**No `vercel.json` is generated for Vercel KV or Vercel Postgres.** Neither \
             product exists any more; anything that generates config for them is writing \
             against 2024 documentation."
                .to_string(),
        ],
        (code_lines(&entry(options)), code_lines(STORE)),
    )
}

pub fn files(program: &Program<'_>, options: &Options) -> Vec<File> {
    vec![
        File::new("api/index.js", entry(options)),
        File::new("_zd/store.js", STORE),
        File::new(
            "package.json",
            "{\n  \"private\": true,\n  \"type\": \"module\"\n}\n",
        ),
        File::new("vercel.json", vercel_json(program, options)),
    ]
}

fn vercel_json(program: &Program<'_>, options: &Options) -> String {
    // `maxDuration` is expressible in `vercel.json` only for the Node
    // runtime; on Edge it is rejected, and the runtime is chosen by the
    // function's own `config` export instead.
    let functions = match options.runtime {
        VercelRuntime::Fluid => format!(
            ",\n  \"functions\": {{\n    \"api/index.js\": {{\n      \"maxDuration\": {}\n    }}\n  }}",
            stream_budget(options).ceiling_seconds()
        ),
        VercelRuntime::Edge => String::new(),
    };
    format!(
        "{{\n\
         \x20 \"$schema\": \"https://openapi.vercel.sh/vercel.json\",\n\
         \x20 \"outputDirectory\": \"public\"{functions},{}\n\
         \x20 \"rewrites\": [\n\
         \x20   {{ \"source\": \"/_zd/(.*)\", \"destination\": \"/api/index\" }}\n\
         \x20 ]\n\
         }}\n",
        headers(program)
    )
}

/// The `headers` block, for the files whose names carry a content hash
/// (#137). Empty for a program with nothing hashed.
///
/// Vercel has no `_headers` file, so this is the one place the rule can be
/// written for this target. It is stated as exact `source` paths rather
/// than a pattern for two reasons: a hashed name is not a shape a glob can
/// describe without also matching a file that merely has dots in it, and
/// there is no second rule for these paths to be merged with.
fn headers(program: &Program<'_>) -> String {
    if program.immutable.is_empty() {
        return String::new();
    }
    let mut paths: Vec<&String> = program.immutable.iter().collect();
    paths.sort();
    paths.dedup();
    let rules: Vec<String> = paths
        .iter()
        .map(|path| {
            format!(
                "\x20   {{\n\
                 \x20     \"source\": \"/{}\",\n\
                 \x20     \"headers\": [{{ \"key\": \"Cache-Control\", \"value\": \"{}\" }}]\n\
                 \x20   }}",
                escape(path),
                zdc_codegen::cache::IMMUTABLE
            )
        })
        .collect();
    format!("\n  \"headers\": [\n{}\n  ],", rules.join(",\n"))
}

/// A JSON string body. These are file names the compiler chose — a stem, a
/// hash and an extension — so this only ever has to be correct.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}
