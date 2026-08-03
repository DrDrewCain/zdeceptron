//! The two generated modules that are the same on every target.
//!
//! They are generated rather than copied because they carry this program's
//! endpoint table and this deployment's timings — but nothing in either of
//! them depends on which platform it is bound for, which is why they live
//! here and not in a target module.

use zdc_codegen::{Call, ServerFunction};

use crate::capability::Capabilities;

/// `_zd/endpoints.js` — name to handler, with the calling convention the
/// compiler chose for each.
pub fn table(functions: &[ServerFunction]) -> String {
    let mut out = String::from(
        "// zdc · generated, do not edit\n\
         // The endpoint table. Portable: this file is the same on every target.\n",
    );
    if functions.is_empty() {
        out.push_str("\nexport const endpoints = {};\n");
        return out;
    }

    out.push('\n');
    for (index, function) in functions.iter().enumerate() {
        out.push_str(&format!(
            "import {{ handler as ${index} }} from '../{}';\n",
            function.path
        ));
    }
    out.push_str("\nexport const endpoints = {\n");
    for (index, function) in functions.iter().enumerate() {
        let inputs: Vec<String> = function
            .inputs
            .iter()
            .map(|input| format!("'{}'", escape(input)))
            .collect();
        // A command's handler takes the argument array positionally; a
        // value endpoint destructures a parameter object (§17.2.7). The
        // router cannot guess which, so the compiler records it.
        let command = match function.call {
            Call::Named => "false",
            Call::Positional => "true",
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

/// `_zd/config.js` — the timings the capability report just promised.
///
/// Generated from the same [`Capabilities`] the report renders, so the
/// numbers a user is told and the numbers the stream actually obeys are one
/// value rather than two that agree today.
pub fn config(capabilities: &Capabilities) -> String {
    format!(
        "// zdc · generated, do not edit\n\
         // The timings `CAPABILITIES.md` reports, as the stream obeys them.\n\
         //\n\
         // `maxStreamSeconds` of 0 means the platform documents no ceiling.\n\
         \nexport const config = {{\n\
         \x20 heartbeatSeconds: {},\n\
         \x20 idleSeconds: {},\n\
         \x20 maxStreamSeconds: {},\n\
         \x20 pollSeconds: {},\n\
         }};\n",
        capabilities.heartbeat_seconds,
        capabilities.idle_seconds,
        capabilities.stream.ceiling_seconds(),
        capabilities.poll_seconds,
    )
}

/// A JavaScript single-quoted string body. Endpoint names come from
/// identifiers and the `.` separator, so this only ever has to be correct
/// rather than clever.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\'', "\\'")
}
