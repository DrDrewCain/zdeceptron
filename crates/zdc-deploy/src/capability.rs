//! What a target can and cannot do, decided before anything is written.
//!
//! `zdc deploy --target lambda` has to say what you are giving up *before*
//! you deploy. Finding out at 900 seconds that your stream died, or after
//! the bill arrives that Lambda kept charging for a browser tab that closed
//! twenty minutes ago, is precisely the failure this report exists to
//! prevent.
//!
//! Every number here is from the platform's own documentation. Where the
//! documentation gives none, the report says so rather than inventing one.

use crate::refusal::Refusal;
use crate::{cloudflare, code_lines, deno, lambda, vercel};
use crate::{LambdaFront, Options, Plan, Program, Target, VercelRuntime, CELLS_JS, ROUTER_JS};

/// How long a `text/event-stream` may be held open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamBudget {
    /// The platform documents no hard duration limit.
    Unlimited { note: &'static str },
    /// A documented ceiling, in seconds.
    Seconds { seconds: u32, note: &'static str },
    /// Streaming does not work in this deployment shape at all.
    Impossible { note: &'static str },
}

impl StreamBudget {
    /// The number `_zd/config.js` enforces.
    ///
    /// `0` means "the platform documents no ceiling, so do not impose one".
    /// A shape that cannot stream gets `1` rather than `0`: if a request
    /// ever reaches the watch endpoint there anyway, closing it at once is
    /// the honest outcome.
    pub fn ceiling_seconds(&self) -> u32 {
        match self {
            StreamBudget::Unlimited { .. } => 0,
            StreamBudget::Seconds { seconds, .. } => *seconds,
            StreamBudget::Impossible { .. } => 1,
        }
    }

    fn summary(&self) -> String {
        match self {
            StreamBudget::Unlimited { note } => format!("unlimited — {note}"),
            StreamBudget::Seconds { seconds, note } => format!("{seconds} s — {note}"),
            StreamBudget::Impossible { note } => format!("impossible — {note}"),
        }
    }
}

/// Whether one client's write reaches another client's screen, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveSync {
    /// The store pushes. No polling, no wasted round trips.
    Push { mechanism: &'static str },
    /// The store has no change feed, so the adapter re-reads it.
    Poll { reason: &'static str },
    /// Not available in this deployment shape.
    Impossible { reason: &'static str },
}

impl LiveSync {
    fn summary(&self, poll_seconds: u32) -> String {
        match self {
            LiveSync::Push { mechanism } => format!("yes, pushed — {mechanism}"),
            LiveSync::Poll { reason } => {
                format!("yes, polled every {poll_seconds} s — {reason}")
            }
            LiveSync::Impossible { reason } => format!("**no** — {reason}"),
        }
    }
}

/// Whether `add 1 to visits` is safe when two visitors click at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Atomicity {
    /// The store has a native atomic add.
    Native { mechanism: &'static str },
    /// A versionstamp check with a bounded retry.
    CompareAndSet { mechanism: &'static str },
    /// The store serialises every operation, so nothing can interleave.
    Serialised { mechanism: &'static str },
}

impl Atomicity {
    fn summary(&self) -> String {
        match self {
            Atomicity::Native { mechanism } => format!("yes, natively — {mechanism}"),
            Atomicity::CompareAndSet { mechanism } => {
                format!("yes, compare-and-set — {mechanism}")
            }
            Atomicity::Serialised { mechanism } => format!("yes, serialised — {mechanism}"),
        }
    }
}

/// How much code this platform costs over the portable core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shim {
    /// Lines of code in the entry file — the part ECMA-429 does not define.
    pub entry_lines: usize,
    /// Lines of code in the store binding.
    pub store_lines: usize,
    /// Lines of code in the portable adapter, shared by every target.
    pub portable_lines: usize,
    /// Lines of emitted handler body, identical on every target.
    pub handler_lines: usize,
}

impl Shim {
    pub fn total(&self) -> usize {
        self.entry_lines + self.store_lines
    }

    /// One line, for a terminal.
    pub fn report(&self) -> String {
        format!(
            "shim: {} lines ({} entry + {} store) against {} portable + {} generated handler",
            self.total(),
            self.entry_lines,
            self.store_lines,
            self.portable_lines,
            self.handler_lines
        )
    }
}

/// Everything a user should know before running the deploy command.
#[derive(Debug, Clone)]
pub struct Capabilities {
    pub target: Target,
    /// How requests arrive, in the platform's own words.
    pub front: String,
    pub stream: StreamBudget,
    pub live_sync: LiveSync,
    pub atomicity: Atomicity,
    /// What the clock is charged against.
    pub billing: &'static str,
    pub heartbeat_seconds: u32,
    pub idle_seconds: u32,
    pub poll_seconds: u32,
    /// Ceilings worth designing against, one per line.
    pub ceilings: Vec<String>,
    /// What this tool did not do for you.
    pub manual: Vec<String>,
    /// Environment keys the program reads, and where the platform keeps
    /// them. Names only — a generated file never carries a value.
    pub secrets: Vec<String>,
    pub shim: Shim,
}

impl Capabilities {
    /// The report, as `CAPABILITIES.md` and as what `zdc deploy` prints.
    pub fn report(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "# {} — what you are getting\n\n",
            self.target.title()
        ));
        out.push_str(&format!("Requests arrive: {}\n\n", self.front));
        out.push_str("| | |\n|---|---|\n");
        out.push_str(&format!(
            "| Max stream duration | {} |\n",
            self.stream.summary()
        ));
        out.push_str(&format!(
            "| Live sync | {} |\n",
            self.live_sync.summary(self.poll_seconds)
        ));
        out.push_str(&format!(
            "| Atomic writes | {} |\n",
            self.atomicity.summary()
        ));
        out.push_str(&format!("| Billed on | {} |\n", self.billing));
        out.push_str(&format!(
            "| Heartbeat | every {} s |\n",
            self.heartbeat_seconds
        ));
        out.push_str(&format!(
            "| Idle timeout | {} |\n",
            if self.idle_seconds == 0 {
                "none".to_string()
            } else {
                format!("{} s", self.idle_seconds)
            }
        ));
        out.push_str(&format!("| Adapter size | {} |\n", self.shim.report()));

        out.push_str("\n## Not atomic\n\n");
        out.push_str(
            "`incr` and `decr` are atomic as reported above. `append` and `remove` are a \
             read-modify-write of one cell and are only as safe as the store's own \
             serialisation, and a write through a record field is a read-modify-write \
             everywhere.\n",
        );

        out.push_str("\n## Ceilings\n\n");
        for ceiling in &self.ceilings {
            out.push_str(&format!("- {ceiling}\n"));
        }

        out.push_str("\n## You still have to do this by hand\n\n");
        for step in &self.manual {
            out.push_str(&format!("- {step}\n"));
        }

        out.push_str("\n## Secrets\n\n");
        if self.secrets.is_empty() {
            out.push_str("This program reads no environment keys.\n");
        } else {
            out.push_str(
                "No generated file contains a secret value. Each of these is a reference to \
                 the platform's own secret store:\n\n",
            );
            for secret in &self.secrets {
                out.push_str(&format!("- {secret}\n"));
            }
        }

        out.push_str(
            "\n## Azure Functions\n\nDeliberately not a target. Its own documentation \
             contradicts itself on whether an HTTP response is capped at 230 seconds or \
             whether that is a four-minute idle timeout a keepalive defeats, no Azure \
             documentation confirms that a streaming response survives past 230 seconds, and \
             it has no atomic increment. A capability report for Azure could not be \
             truthful, so there is no Azure adapter.\n",
        );
        out
    }
}

/// The stream ceiling, which several places need and only one may decide.
pub(crate) fn stream_budget(options: &Options) -> StreamBudget {
    match options.target {
        Target::Cloudflare => StreamBudget::Unlimited {
            note: "Workers document no hard duration limit for an HTTP-triggered Worker while \
                   the client stays connected",
        },
        Target::Lambda => match options.front {
            // The function timeout is the ceiling; there is no separate,
            // longer budget for a streamed response.
            LambdaFront::FunctionUrl => StreamBudget::Seconds {
                seconds: 900,
                note: "the Lambda function timeout, 15 minutes, is a hard maximum",
            },
            LambdaFront::ApiGatewayRestRegional => StreamBudget::Seconds {
                seconds: 900,
                note: "API Gateway streams for up to 15 minutes; the 5-minute idle timeout on a \
                       Regional endpoint is defeated by the heartbeat",
            },
            LambdaFront::ApiGatewayRestEdge => StreamBudget::Seconds {
                seconds: 900,
                note: "API Gateway streams for up to 15 minutes; the idle timeout on an \
                       edge-optimized endpoint is 30 seconds, so the heartbeat is not optional",
            },
            LambdaFront::Alb => StreamBudget::Impossible {
                note: "an ALB takes one JSON response of at most 1 MB, does not honour \
                       `Transfer-Encoding`, and rejects upgrade requests with HTTP 400",
            },
        },
        Target::Vercel => match options.runtime {
            VercelRuntime::Fluid => match options.plan {
                Plan::Free => StreamBudget::Seconds {
                    seconds: 300,
                    note: "Hobby's `maxDuration` is 300 s by default and by maximum, and \
                           includes time spent streaming",
                },
                Plan::Paid => StreamBudget::Seconds {
                    seconds: 800,
                    note: "Pro's `maxDuration` maximum is 800 s; the 1800 s extension is beta \
                           and not generated here",
                },
            },
            VercelRuntime::Edge => StreamBudget::Seconds {
                seconds: 300,
                note: "the Edge runtime must send a first byte within 25 s and may then stream \
                       for up to 300 s",
            },
        },
        Target::Deno => StreamBudget::Unlimited {
            note: "Deno Deploy documents no request timeout, and sending response bytes is \
                   itself what keeps the app alive — but an isolate can be evicted mid-stream",
        },
    }
}

/// Why this target cannot run a scheduled job, or `None` if it can.
///
/// Exhaustive over [`Target`], so a fifth target has to answer the question
/// rather than inherit whichever arm was written last — and the answer is a
/// *platform* fact each time, not a note about what has been implemented
/// here. Three of the four are `Some`, which is the honest shape of this
/// feature today and is why the refusal names Cloudflare by name.
pub(crate) fn unschedulable(target: Target, plan: Plan) -> Option<&'static str> {
    match target {
        // `[triggers] crons` plus a `scheduled()` export, one-minute
        // granularity, and the invocation is not an HTTP request at all —
        // so a job is reachable by the scheduler and by nobody else.
        Target::Cloudflare => None,
        // EventBridge expresses every one of these cadences exactly, as
        // `rate(n unit)`; what is missing is on this side of the wire. The
        // generated entry is `awslambda.streamifyResponse()`-shaped for an
        // HTTP request, and a scheduled invocation arrives as a plain
        // event with no `requestContext`, through a handler signature the
        // shim does not have. That is a shim to write, and writing it
        // untested against real infrastructure is what this crate's Azure
        // note already declines to do.
        Target::Lambda => Some(
            "EventBridge expresses this cadence exactly — the SAM template would carry \
             `Events: Schedule` with a `rate()` expression. What is missing is the entry: \
             `lambda.mjs` is shaped by `awslambda.streamifyResponse()` for an HTTP request, and \
             a scheduled invocation arrives as a bare event through a different signature.",
        ),
        // Two independent reasons, and the second is the serious one.
        Target::Vercel => Some(match plan {
            Plan::Free => {
                "Vercel Cron on Hobby runs a job **once a day**, and only guarantees the hour \
                 rather than the minute, so no cadence here can be honoured as written. Beyond \
                 that, a Vercel cron is an ordinary HTTP request to a route: the job would need \
                 a public URL, guarded only by a `CRON_SECRET` bearer token this router does not \
                 check. A job with a URL is the hazard `inbound` is refused for."
            }
            Plan::Paid => {
                "A Vercel cron is an ordinary HTTP request to a route, so the job would need a \
                 public URL, guarded only by a `CRON_SECRET` bearer token this router does not \
                 check — and the endpoint table deliberately has no entry for a job. A job with \
                 a URL anyone can fetch is the hazard `inbound` is refused for, and it must not \
                 arrive by accident through the scheduling mechanism."
            }
        }),
        // The same evidence this module already records for queues.
        Target::Deno => Some(
            "`Deno.cron` is not available on the Deno Deploy platform this adapter targets, \
             alongside `Deno.Kv.enqueue()` and `listenQueue()`, which `deno.rs` already records \
             as unsupported. Scheduling against a documented-elsewhere API would be a capability \
             report that cannot be trusted.",
        ),
    }
}

/// Decide what a target gives, or refuse the combination.
pub(crate) fn describe(program: &Program<'_>, options: &Options) -> Result<Capabilities, Refusal> {
    if options.poll_seconds == 0 {
        return Err(Refusal::new(
            "A poll interval of 0 s is a busy loop. Give `--poll-seconds` a positive number.",
        ));
    }

    let stream = stream_budget(options);

    // §14G.4's schedules. **A target that cannot run one refuses the build
    // rather than writing the job out and never scheduling it**, which is
    // the same rule the ALB refusal below is an instance of: a deployment
    // that silently drops a construct is worse than one that will not
    // build, because nothing later reports it.
    //
    // Cloudflare is the target that genuinely supports it, and the other
    // three are refused for three different reasons — none of them effort,
    // and each of them checkable against the platform's own documentation.
    if let Some(why) = unschedulable(options.target, options.plan) {
        let jobs = crate::endpoints::triggers(program.functions);
        if !jobs.is_empty() {
            let names: Vec<&str> = jobs.iter().map(|job| job.name.as_str()).collect();
            return Err(Refusal::new(format!(
                "This program schedules {} ({}), and {} cannot run one.\n\n{why}\n\nDeploy to \
                 Cloudflare Workers (`--target cloudflare`), which expresses this cadence \
                 exactly, or remove the schedule.",
                if names.len() == 1 { "a job" } else { "jobs" },
                names.join(", "),
                options.target.title(),
            )));
        }
    }

    // The refusal that matters. An ALB does not stream, so `durable` live
    // sync cannot work behind one — not slowly, not with a workaround.
    if options.target == Target::Lambda && options.front == LambdaFront::Alb && program.live_sync()
    {
        return Err(Refusal::new(format!(
            "This program has durable state ({}), and durable signals sync live (§8.1). AWS \
             Lambda behind an Application Load Balancer cannot hold a `text/event-stream` open: \
             the load balancer invokes the function and expects a single JSON response of at \
             most 1 MB, it does not honour hop-by-hop headers such as `Transfer-Encoding`, and \
             it rejects upgrade requests with HTTP 400. There is no workaround at this \
             front.\n\nDeploy behind a Lambda Function URL in `RESPONSE_STREAM` invoke mode \
             (`--front function-url`, the default), or an API Gateway REST API with `STREAM` \
             response transfer mode (`--front api-gateway-rest-regional`).",
            program.durable.join(", ")
        )));
    }

    if options.target == Target::Lambda && options.idle_seconds == 0 {
        return Err(Refusal::new(
            "AWS Lambda bills the full duration of a streamed response and does not stop when \
             the invoking client's connection is broken, so a stream with no idle timeout bills \
             until the function times out — 15 minutes per abandoned browser tab. Give \
             `--idle-seconds` a positive number.",
        ));
    }

    if let StreamBudget::Seconds { seconds, .. } = stream {
        if options.idle_seconds >= seconds {
            return Err(Refusal::new(format!(
                "An idle timeout of {} s can never fire on {}, whose streams end at {seconds} s \
                 anyway. Give `--idle-seconds` a number below {seconds}.",
                options.idle_seconds,
                options.target.title()
            )));
        }
    }

    // The heartbeat has one job: stay under the shortest idle timeout
    // anything between the client and the function enforces. 30 seconds on
    // an edge-optimized API Gateway endpoint is the shortest of them, and
    // 15 s clears it with room for a retransmit.
    let heartbeat_seconds = 15.min((options.idle_seconds.max(2)) / 2).max(1);

    let (front, live_sync, atomicity, billing, ceilings, manual, shim) = match options.target {
        Target::Cloudflare => cloudflare::capabilities(options),
        Target::Lambda => lambda::capabilities(options),
        Target::Vercel => vercel::capabilities(options),
        Target::Deno => deno::capabilities(options),
    };

    let secrets = match options.target {
        Target::Cloudflare => program
            .environment
            .iter()
            .map(|key| format!("`{key}` — `wrangler secret put {key}`"))
            .collect(),
        Target::Lambda => program
            .environment
            .iter()
            .map(|key| {
                format!(
                    "`{key}` — resolved by CloudFormation from Secrets Manager at \
                     `zd/{}/secrets`, key `{key}`. The template holds the reference, never \
                     the value.",
                    options.app
                )
            })
            .collect(),
        Target::Vercel => program
            .environment
            .iter()
            .map(|key| format!("`{key}` — `vercel env add {key}`"))
            .collect(),
        Target::Deno => program
            .environment
            .iter()
            .map(|key| format!("`{key}` — the app's Environment Variables, marked as a secret"))
            .collect(),
    };

    Ok(Capabilities {
        target: options.target,
        front,
        stream,
        live_sync,
        atomicity,
        billing,
        heartbeat_seconds,
        idle_seconds: options.idle_seconds,
        poll_seconds: options.poll_seconds,
        ceilings,
        manual,
        secrets,
        shim: Shim {
            entry_lines: shim.0,
            store_lines: shim.1,
            portable_lines: code_lines(ROUTER_JS) + code_lines(CELLS_JS),
            handler_lines: program
                .functions
                .iter()
                .map(|function| code_lines(&function.source))
                .sum(),
        },
    })
}

/// What a target module returns to the report.
pub(crate) type Described = (
    String,
    LiveSync,
    Atomicity,
    &'static str,
    Vec<String>,
    Vec<String>,
    (usize, usize),
);
