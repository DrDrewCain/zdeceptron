//! The two generated modules that are the same on every target.
//!
//! They are generated rather than copied because they carry this program's
//! endpoint table and this deployment's timings — but nothing in either of
//! them depends on which platform it is bound for, which is why they live
//! here and not in a target module.

use zdc_codegen::{FunctionKind, ServerFunction};

use crate::capability::Capabilities;

/// `_zd/endpoints.js` — name to handler, with the calling convention the
/// compiler chose for each.
pub fn table(functions: &[ServerFunction]) -> String {
    let mut out = String::from(
        "// zdc · generated, do not edit\n\
         // The endpoint table. Portable: this file is the same on every target.\n\
         //\n\
         // Scheduled jobs are not here, and that is not an omission: nothing on\n\
         // the wire may start one (§14G.4). The platform entry calls them.\n",
    );
    let routable = routable(functions);
    if routable.is_empty() {
        out.push_str("\nexport const endpoints = {};\n");
        return out;
    }

    out.push('\n');
    for (index, function) in routable.iter().enumerate() {
        out.push_str(&format!(
            "import {{ handler as ${index} }} from '../{}';\n",
            function.path
        ));
    }
    out.push_str("\nexport const endpoints = {\n");
    for (index, function) in routable.iter().enumerate() {
        let inputs: Vec<String> = function
            .inputs
            .iter()
            .map(|input| format!("'{}'", escape(input)))
            .collect();
        // A command's handler takes the argument array positionally; a
        // value endpoint destructures a parameter object (§17.2.7). The
        // router cannot guess which, so the compiler records it.
        let command = match function.kind {
            FunctionKind::Value => "false",
            FunctionKind::Command => "true",
            FunctionKind::Trigger(_) => unreachable!("`routable` filtered the triggers out"),
        };
        out.push_str(&format!(
            "  '{}': {{ handler: ${index}, inputs: [{}], command: {command} }},\n",
            escape(&function.name),
            inputs.join(", ")
        ));
    }
    out.push_str("};\n");
    out
}

/// The functions a request may reach, which is not all of them.
///
/// **A scheduled job is deliberately absent from the endpoint table.** The
/// router dispatches by name over whatever this map contains, so listing a
/// job here would put a URL in front of it — and a job is the one server
/// root a program never meant anybody to be able to start. That is the
/// same hazard `inbound` is refused for, and it would arrive by accident
/// rather than by design.
///
/// The one target where the hazard cannot be avoided is Vercel, whose cron
/// mechanism *is* an HTTP request to a route; `vercel.rs` says so and the
/// capability report repeats it.
pub fn routable(functions: &[ServerFunction]) -> Vec<&ServerFunction> {
    functions
        .iter()
        .filter(|function| match function.kind {
            FunctionKind::Value | FunctionKind::Command => true,
            FunctionKind::Trigger(_) => false,
        })
        .collect()
}

/// Every scheduled job in the bundle, in emission order.
pub fn triggers(functions: &[ServerFunction]) -> Vec<&ServerFunction> {
    functions
        .iter()
        .filter(|function| match function.kind {
            FunctionKind::Trigger(_) => true,
            FunctionKind::Value | FunctionKind::Command => false,
        })
        .collect()
}

/// The distinct cron rules this program needs, sorted.
///
/// Distinct, because a platform is told about *rules* and not about jobs:
/// two hourly jobs are one `crons` entry, and the entry that receives the
/// beat runs both. Sorted, because two builds of one program must produce
/// the same bytes.
pub fn cron_rules(functions: &[ServerFunction]) -> Vec<String> {
    let mut rules: Vec<String> = triggers(functions)
        .iter()
        .filter_map(|function| match function.kind {
            FunctionKind::Trigger(cadence) => Some(cadence.cron()),
            FunctionKind::Value | FunctionKind::Command => None,
        })
        .collect();
    rules.sort();
    rules.dedup();
    rules
}

/// `_zd/schedule.js` — the jobs, each with the cron rule that fires it.
///
/// Portable for the same reason the endpoint table is: it says which job
/// runs how often, and nothing about how a platform delivers a beat. What
/// differs per target is the entry that reads it, which is why only the
/// targets that can express a schedule import this file.
///
/// The cron expression is carried beside the handler rather than left to
/// the platform configuration alone, because a Cloudflare `scheduled()`
/// invocation says *which* rule fired (`controller.cron`) and one worker
/// may hold several. Matching on the rule is what keeps an hourly job from
/// running on the minutely job's beat.
pub fn schedule(functions: &[ServerFunction]) -> String {
    let mut out = String::from(
        "// zdc · generated, do not edit\n\
         // The scheduled jobs (§14G.4). Portable: the same on every target.\n\
         //\n\
         // `input` is the name the job's cell has in the program. The beat's\n\
         // scheduled start time, in seconds since 1970, is passed under it.\n",
    );
    let triggers = triggers(functions);
    if triggers.is_empty() {
        out.push_str("\nexport const schedule = [];\n");
        return out;
    }
    out.push('\n');
    for (index, function) in triggers.iter().enumerate() {
        out.push_str(&format!(
            "import {{ handler as $job{index} }} from '../{}';\n",
            function.path
        ));
    }
    out.push_str("\nexport const schedule = [\n");
    for (index, function) in triggers.iter().enumerate() {
        let FunctionKind::Trigger(cadence) = function.kind else {
            unreachable!("`triggers` admits only scheduled jobs")
        };
        // A job's one input is its own cell; a job with none would have
        // nowhere to deliver the beat, and `emit_trigger` always names one.
        let input = function.inputs.first().map(String::as_str).unwrap_or("");
        out.push_str(&format!(
            "  {{ name: '{}', cron: '{}', input: '{}', handler: $job{index} }},\n",
            escape(&function.name),
            cadence.cron(),
            escape(input),
        ));
    }
    out.push_str("];\n");
    out
}

/// `_zd/config.js` — the timings the capability report just promised.
///
/// Generated from the same [`Capabilities`] the report renders, so the
/// numbers a user is told and the numbers the stream actually obeys are one
/// value rather than two that agree today.
pub fn config(capabilities: &Capabilities, durable: &[String]) -> String {
    let keys = durable
        .iter()
        .map(|key| format!("'{}'", escape(key)))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "// zdc · generated, do not edit\n\
         // What the router needs that is not the same on every target: the\n\
         // timings `CAPABILITIES.md` reports, and the keys this program\n\
         // declares.\n\
         //\n\
         // `maxStreamSeconds` of 0 means the platform documents no ceiling.\n\
         //\n\
         // `durableKeys` is the whole of what a subscriber may ask for. The\n\
         // `?keys=` list arrives from outside, and a key the program never\n\
         // declared would otherwise be a way to read any value in the store\n\
         // by guessing its name — which is what `zdc dev` refuses in\n\
         // `permitted`, and what this file exists to let the router refuse\n\
         // the same way.\n\
         \nexport const config = {{\n\
         \x20 heartbeatSeconds: {},\n\
         \x20 idleSeconds: {},\n\
         \x20 maxStreamSeconds: {},\n\
         \x20 pollSeconds: {},\n\
         \x20 durableKeys: [{}],\n\
         }};\n",
        capabilities.heartbeat_seconds,
        capabilities.idle_seconds,
        capabilities.stream.ceiling_seconds(),
        capabilities.poll_seconds,
        keys,
    )
}

/// A JavaScript single-quoted string body. Endpoint names come from
/// identifiers and the `.` separator, so this only ever has to be correct
/// rather than clever.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\'', "\\'")
}
