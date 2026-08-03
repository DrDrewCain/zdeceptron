#![allow(dead_code)]

//! Shared fixture plumbing: source text in, `Hir` and `TierSplit` out.

use zdc_graph::{ifc, split, TierSplit, Verdict};
use zdc_hir::{DefId, DefKind, Hir};

pub fn compile(src: &str) -> (Hir, TierSplit) {
    let program = zdc_parser::parse(src).unwrap_or_else(|e| panic!("parse failed: {}", e.message));
    let hir = zdc_resolve::Resolver::new(&program)
        .resolve()
        .unwrap_or_else(|errors| {
            panic!(
                "resolution failed: {}",
                errors
                    .iter()
                    .map(|e| e.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
    let split = split(&hir);
    (hir, split)
}

pub fn verdict(src: &str) -> (Hir, TierSplit, Verdict) {
    let (hir, split) = compile(src);
    let verdict = ifc(&hir, &split);
    (hir, split, verdict)
}

pub fn def_named(hir: &Hir, name: &str) -> DefId {
    hir.defs
        .iter()
        .find(|(_, def)| def.name == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no definition named `{name}`"))
}

/// Every error code a pass reported, in order.
pub fn codes(errors: &[zdc_graph::GraphError]) -> Vec<&str> {
    errors
        .iter()
        .filter(|e| e.is_error())
        .map(|e| e.code)
        .collect()
}

pub fn names(hir: &Hir, ids: impl IntoIterator<Item = DefId>) -> Vec<String> {
    let mut out: Vec<String> = ids
        .into_iter()
        .map(|id| match &hir.defs[id].kind {
            DefKind::View(_) => "view".to_string(),
            DefKind::Signal(_) | DefKind::Function(_) | DefKind::Record(_) | DefKind::Choice(_) => {
                hir.defs[id].name.clone()
            }
        })
        .collect();
    out.sort();
    out
}

pub const GUESTBOOK: &str = include_str!("../../../../examples/guestbook.zd");
