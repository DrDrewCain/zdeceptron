#![forbid(unsafe_code)]

//! Deploy adapters: everything a serverless platform needs, generated from
//! an already-compiled bundle.
//!
//! # What is portable, and what is not
//!
//! Spec §8 says an emitted function uses only the WinterTC common API and
//! therefore "one artifact runs on AWS Lambda, Cloudflare Workers, Deno
//! Deploy, Vercel Functions, and Azure Functions". That is half true, and
//! this crate exists because of the other half.
//!
//! **ECMA-429** (*Minimum common web API*, 1st edition, December 2025)
//! standardises the *interior* of a handler: `Response`, `ReadableStream`
//! and `TextEncoder` are all it takes to emit a `text/event-stream`, and no
//! platform API is needed to produce one. **It standardises no entrypoint
//! at all.** WinterTC has reserved a repository named
//! `proposal-http-server-api` and it is empty — no README, no commits. So
//! every target needs a generated wrapper, and those wrappers are not
//! cosmetic: Lambda's streaming path needs the non-standard
//! `awslambda.streamifyResponse()` global and hands the handler a **Node.js
//! writable stream**, not a WHATWG `WritableStream`.
//!
//! The good news is the shape of the result rather than a hedge about it.
//! The portable core is real and it is most of the code: the emitted
//! handler bodies are byte-identical on all four targets (asserted by
//! `tests/portability.rs`), and so are the router and the store helpers
//! layered on top of them. What differs is an entry file and a store
//! binding, both small, both mechanical, and both exactly the kind of thing
//! a compiler should be writing instead of a human. [`Shim::report`] prints
//! the line count so the claim can be checked rather than believed.
//!
//! # Azure Functions is out of scope, deliberately
//!
//! Spec §9 lists `azure` as a build target. It is not implemented here, and
//! the reason is evidence rather than effort:
//!
//! 1. **Microsoft's own documentation contradicts itself on the duration
//!    limit.** `functions-scale` says, of an HTTP-triggered function,
//!    "Regardless of the function app timeout setting, 230 seconds is the
//!    maximum amount of time that an HTTP triggered function can take to
//!    respond to a request", attributing it to the Azure Load Balancer.
//!    The API Management guidance describes the *same* load balancer limit
//!    as a four-minute **idle** timeout that keepalive traffic defeats. No
//!    Azure documentation confirms that an actively streaming SSE response
//!    survives past 230 seconds, and the footnote that would explain the
//!    number links to a page where the section no longer exists.
//! 2. **There is no atomic increment.** Nothing in the Azure serverless
//!    storage lineup gives the `incr` the store interface requires without
//!    a second system.
//!
//! A capability report for Azure would have to say "the maximum stream
//! duration is either 230 seconds or unbounded, and we cannot tell you
//! which". A report that cannot be trusted is worse than a target that does
//! not exist, so the target does not exist. Adding it should be gated on
//! measuring the behaviour on real infrastructure, not on more reading.

mod capability;
mod cloudflare;
mod deno;
mod endpoints;
mod lambda;
mod linked;
mod refusal;
mod vercel;

use std::collections::BTreeSet;

use zdc_codegen::{LinkedModule, ServerFunction};

pub use crate::capability::{Atomicity, Capabilities, LiveSync, Shim, StreamBudget};
pub use crate::refusal::Refusal;

/// The compatibility date generated Cloudflare configuration pins.
///
/// A constant rather than today's date: two builds of the same program must
/// produce the same bytes, and a config file that changes because the clock
/// moved is a diff nobody asked for.
pub const COMPATIBILITY_DATE: &str = "2026-08-03";

/// A platform an adapter can be generated for.
///
/// There is no catch-all arm anywhere this is matched, so adding a variant
/// is a compile error in every place that has to have an opinion — which is
/// the point, because the places that have to have an opinion are exactly
/// the ones a new platform would otherwise silently inherit wrong answers
/// from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    Cloudflare,
    Lambda,
    Vercel,
    Deno,
}

impl Target {
    /// Every target, in the order a report lists them.
    pub const ALL: [Target; 4] = [
        Target::Cloudflare,
        Target::Lambda,
        Target::Vercel,
        Target::Deno,
    ];

    /// The word `--target` takes.
    pub fn slug(self) -> &'static str {
        match self {
            Target::Cloudflare => "cloudflare",
            Target::Lambda => "lambda",
            Target::Vercel => "vercel",
            Target::Deno => "deno",
        }
    }

    /// The platform's own name for itself.
    pub fn title(self) -> &'static str {
        match self {
            Target::Cloudflare => "Cloudflare Workers",
            Target::Lambda => "AWS Lambda",
            Target::Vercel => "Vercel Functions",
            Target::Deno => "Deno Deploy",
        }
    }

    /// Where the browser half of the bundle sits, relative to the
    /// deployment root.
    ///
    /// It is `public` on all four, and it is a `match` anyway: the
    /// directory is not a shared convention the targets happen to agree
    /// on, it is four separate platform facts that currently coincide, and
    /// each arm names the one that makes it true. A fifth target whose
    /// static handling looks somewhere else is then a compile error here
    /// rather than a deployment whose page is served from a directory the
    /// platform does not read.
    ///
    /// This decides where a `foreign` module the *browser* imports has to
    /// be copied (#225), so it is the same value `zdc deploy` writes
    /// `client.js` under — one answer, not two that agree today.
    pub fn browser_root(self) -> &'static str {
        match self {
            // `wrangler.toml`'s `[assets] directory = "./public"`.
            Target::Cloudflare => "public",
            // Not hosted by the function at all: the report tells the
            // operator to put this directory behind S3 and CloudFront, so
            // it still has to be *in* the deployment under this name.
            Target::Lambda => "public",
            // `vercel.json`'s `outputDirectory`.
            Target::Vercel => "public",
            // `deno-entry.js` reads `./public${path}` itself.
            Target::Deno => "public",
        }
    }

    /// Parse a `--target` word, listing the alternatives when it is not one.
    pub fn parse(word: &str) -> Result<Target, String> {
        Target::ALL
            .into_iter()
            .find(|target| target.slug() == word)
            .ok_or_else(|| {
                let names: Vec<&str> = Target::ALL.into_iter().map(Target::slug).collect();
                format!(
                    "`{word}` is not a deploy target. The targets are {}. Azure Functions is \
                     deliberately absent: its own documentation contradicts itself on whether an \
                     HTTP response is capped at 230 seconds, and it has no atomic increment.",
                    names.join(", ")
                )
            })
    }
}

/// How requests reach a Lambda function. The choice is load-bearing: it
/// decides whether a stream is possible at all, and for how long.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LambdaFront {
    /// A Function URL in `RESPONSE_STREAM` invoke mode. The only shape with
    /// no documented idle timeout of its own.
    FunctionUrl,
    /// An API Gateway REST API with `STREAM` response transfer mode, on a
    /// Regional or private endpoint: 15 minutes of stream, 5 minutes idle.
    ApiGatewayRestRegional,
    /// The same, edge-optimized: 15 minutes of stream, **30 seconds** idle.
    ApiGatewayRestEdge,
    /// An Application Load Balancer. Cannot stream, at all.
    Alb,
}

impl LambdaFront {
    pub const ALL: [LambdaFront; 4] = [
        LambdaFront::FunctionUrl,
        LambdaFront::ApiGatewayRestRegional,
        LambdaFront::ApiGatewayRestEdge,
        LambdaFront::Alb,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            LambdaFront::FunctionUrl => "function-url",
            LambdaFront::ApiGatewayRestRegional => "api-gateway-rest-regional",
            LambdaFront::ApiGatewayRestEdge => "api-gateway-rest-edge",
            LambdaFront::Alb => "alb",
        }
    }

    pub fn parse(word: &str) -> Result<LambdaFront, String> {
        LambdaFront::ALL
            .into_iter()
            .find(|front| front.slug() == word)
            .ok_or_else(|| {
                let names: Vec<&str> = LambdaFront::ALL
                    .into_iter()
                    .map(LambdaFront::slug)
                    .collect();
                format!(
                    "`{word}` is not a Lambda front. The fronts are {}.",
                    names.join(", ")
                )
            })
    }
}

/// Which Vercel runtime the function runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VercelRuntime {
    /// Node with Fluid compute: 300 s on Hobby, 800 s on Pro.
    Fluid,
    /// The Edge runtime: first byte within 25 s, then up to 300 s.
    Edge,
}

impl VercelRuntime {
    pub const ALL: [VercelRuntime; 2] = [VercelRuntime::Fluid, VercelRuntime::Edge];

    pub fn slug(self) -> &'static str {
        match self {
            VercelRuntime::Fluid => "fluid",
            VercelRuntime::Edge => "edge",
        }
    }

    pub fn parse(word: &str) -> Result<VercelRuntime, String> {
        VercelRuntime::ALL
            .into_iter()
            .find(|runtime| runtime.slug() == word)
            .ok_or_else(|| {
                let names: Vec<&str> = VercelRuntime::ALL
                    .into_iter()
                    .map(VercelRuntime::slug)
                    .collect();
                format!(
                    "`{word}` is not a Vercel runtime. The runtimes are {}.",
                    names.join(", ")
                )
            })
    }
}

/// Whether the account is on the vendor's paid tier. Changes the numbers a
/// capability report is allowed to promise, and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    Free,
    Paid,
}

impl Plan {
    pub const ALL: [Plan; 2] = [Plan::Free, Plan::Paid];

    pub fn slug(self) -> &'static str {
        match self {
            Plan::Free => "free",
            Plan::Paid => "paid",
        }
    }

    pub fn parse(word: &str) -> Result<Plan, String> {
        Plan::ALL
            .into_iter()
            .find(|plan| plan.slug() == word)
            .ok_or_else(|| format!("`{word}` is not a plan. The plans are free, paid."))
    }
}

/// What to generate.
#[derive(Debug, Clone)]
pub struct Options {
    pub target: Target,
    /// The deployment's name, used for the worker name, the stack name and
    /// the secret store's path.
    pub app: String,
    pub front: LambdaFront,
    pub runtime: VercelRuntime,
    pub plan: Plan,
    /// How long a stream may sit with nothing to say before it is closed.
    ///
    /// This is the only defence against Lambda's billing model, which
    /// charges for the whole duration of a streamed response and does not
    /// stop when the client disconnects.
    pub idle_seconds: u32,
    /// How often a store with no push channel is re-read.
    pub poll_seconds: u32,
}

impl Options {
    pub fn new(target: Target, app: impl Into<String>) -> Options {
        Options {
            target,
            app: app.into(),
            front: LambdaFront::FunctionUrl,
            runtime: VercelRuntime::Fluid,
            plan: Plan::Free,
            idle_seconds: 60,
            poll_seconds: 2,
        }
    }
}

/// What the compiler produced, as much of it as a deploy adapter reads.
#[derive(Debug, Clone, Copy)]
pub struct Program<'a> {
    /// One per emitted server root. Their sources are copied byte for byte.
    pub functions: &'a [ServerFunction],
    /// Every `foreign` module the bundle imports by relative path, exactly
    /// as [`Bundle::linked_modules`](zdc_codegen::Bundle::linked_modules)
    /// reports it — both halves, with destinations relative to the *bundle*
    /// root.
    ///
    /// A deployment has its own layout, so these are re-placed rather than
    /// used as they stand; [`Deployment::linked_modules`] is the answer and
    /// `linked::place` is where the two trees are reconciled. The list
    /// arrives exactly as the compiler settled it, so the adapter re-derives
    /// nothing — which specifier the emitted `import` wrote, and which half
    /// wrote it, are questions codegen has already answered.
    pub linked: &'a BTreeSet<LinkedModule>,
    /// Every durable key the program touches.
    pub durable: &'a [String],
    /// Every environment key the program reads. Names only: a generated
    /// config file never carries a value.
    pub environment: &'a [String],
    /// Every file in the browser half whose name carries a content hash,
    /// as paths relative to [`Target::browser_root`] — #137.
    ///
    /// Exactly the files a target may tell its edge to cache for a year
    /// and never revalidate. The list arrives from the compiler for the
    /// same reason `durable` and `environment` do: an adapter that
    /// re-derived it would be guessing at names the emitter has already
    /// settled, and a guess that is one file wrong is either a stale page
    /// nobody can flush or a header nobody applies.
    ///
    /// Empty is the honest answer for a program with nothing hashed, and
    /// every target then emits no cache configuration at all rather than
    /// an empty one.
    pub immutable: &'a [String],
}

impl Program<'_> {
    /// Whether this program needs a live `durable` channel.
    ///
    /// A durable signal is shared across visitors and is specified to sync
    /// (§8.1), so the presence of one durable key is the whole test. It is
    /// stated as a method rather than inlined because it is the predicate
    /// the Lambda-behind-an-ALB refusal turns on, and a refusal whose
    /// condition is scattered is a refusal that will drift.
    pub fn live_sync(&self) -> bool {
        !self.durable.is_empty()
    }
}

/// A file to write, at a path relative to the deployment root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub path: String,
    pub contents: String,
}

impl File {
    fn new(path: impl Into<String>, contents: impl Into<String>) -> File {
        File {
            path: path.into(),
            contents: contents.into(),
        }
    }
}

/// Everything a target needs, plus what it costs.
#[derive(Debug, Clone)]
pub struct Deployment {
    pub files: Vec<File>,
    /// The `foreign` modules the deployment must contain, and where each
    /// goes relative to the deployment root (#225).
    ///
    /// Separate from [`files`](Deployment::files) because these are not
    /// generated: they are the author's own JavaScript, copied from the
    /// project by the caller. This crate reads no file and writes none, so
    /// it says which ones and where — the same division `zdc build` already
    /// runs on, and the same sandbox rule applies to the copy, because the
    /// path came out of a program's source text.
    pub linked_modules: BTreeSet<LinkedModule>,
    pub capabilities: Capabilities,
}

/// The portable adapter, shared verbatim by every target.
const ROUTER_JS: &str = include_str!("../js/router.js");
const CELLS_JS: &str = include_str!("../js/cells.js");

/// Generate a deployment, or refuse to.
///
/// Refusal is a build error rather than a warning for the same reason the
/// rest of the compiler refuses: discovering at 900 seconds that your stream
/// died, or at 3 a.m. that an ALB never streamed at all, is the failure this
/// tool exists to prevent.
pub fn generate(program: &Program<'_>, options: &Options) -> Result<Deployment, Refusal> {
    let capabilities = capability::describe(program, options)?;

    let mut files = vec![
        File::new("_zd/router.js", ROUTER_JS),
        File::new("_zd/cells.js", CELLS_JS),
        File::new("_zd/endpoints.js", endpoints::table(program.functions)),
        File::new("_zd/schedule.js", endpoints::schedule(program.functions)),
        File::new("_zd/config.js", endpoints::config(&capabilities)),
    ];
    // The handler bodies, byte for byte as the compiler emitted them. This
    // is the portability claim, and it is a copy rather than a rewrite so
    // that it cannot quietly stop being true.
    for function in program.functions {
        files.push(File::new(function.path.clone(), function.source.clone()));
    }

    files.extend(match options.target {
        Target::Cloudflare => cloudflare::files(program, options),
        Target::Lambda => lambda::files(program, options),
        Target::Vercel => vercel::files(program, options),
        Target::Deno => deno::files(program, options),
    });

    files.push(File::new("CAPABILITIES.md", capabilities.report()));
    files.sort_by(|left, right| left.path.cmp(&right.path));

    Ok(Deployment {
        files,
        linked_modules: linked::place(program, options.target),
        capabilities,
    })
}

/// The line count of a JavaScript source, for the shim report. Blank lines
/// and comment lines are excluded: the interesting number is how much
/// *code* a platform costs, and padding it with the explanation of why the
/// code is there would flatter the wrong thing.
fn code_lines(source: &str) -> usize {
    source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//") && !line.starts_with('*'))
        .count()
}
