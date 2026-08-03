use std::collections::BTreeMap;

use zdc_bench::{Measurement, Report, BENCHMARK_JS, DOM_SHIM_JS, INSTRUMENT_JS, ROW_ZD};

fn measurement(arm: &str, step: &str, fields: &[(&str, i64)]) -> Measurement {
    Measurement {
        arm: arm.into(),
        step: step.into(),
        fields: fields
            .iter()
            .map(|(key, value)| ((*key).to_string(), *value))
            .collect::<BTreeMap<_, _>>(),
    }
}

#[test]
fn omitted_measurement_fields_read_as_zero() {
    let measured = measurement("compiled", "create", &[("rows", 10), ("effects", -2)]);

    assert_eq!(measured.get("rows"), 10);
    assert_eq!(measured.get("effects"), -2);
    assert_eq!(measured.get("crossings"), 0);
}

#[test]
fn report_lookup_matches_both_arm_and_step_exactly() {
    let report = Report(vec![
        measurement("compiled", "create", &[("rows", 10)]),
        measurement("compiled", "clear", &[("rows", 0)]),
        measurement("direct", "create", &[("rows", 20)]),
    ]);

    assert_eq!(report.find("compiled", "create").get("rows"), 10);
    assert_eq!(report.find("compiled", "clear").get("rows"), 0);
    assert_eq!(report.find("direct", "create").get("rows"), 20);
}

#[test]
fn report_axes_are_unique_in_first_observation_order() {
    let report = Report(vec![
        measurement("b", "second", &[]),
        measurement("a", "first", &[]),
        measurement("b", "first", &[]),
        measurement("c", "second", &[]),
    ]);

    assert_eq!(report.arms(), ["b", "a", "c"]);
    assert_eq!(report.steps(), ["second", "first"]);
}

#[test]
fn empty_reports_have_empty_axes() {
    let report = Report(Vec::new());

    assert!(report.arms().is_empty());
    assert!(report.steps().is_empty());
}

#[test]
fn missing_measurements_fail_with_the_requested_coordinates() {
    let report = Report(vec![measurement("compiled", "create", &[])]);
    let panic = std::panic::catch_unwind(|| report.find("missing", "clear"))
        .expect_err("a missing coordinate must not fabricate a measurement");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("");

    assert!(message.contains("missing"), "{message}");
    assert!(message.contains("clear"), "{message}");
}

#[test]
fn every_embedded_benchmark_input_is_present_and_names_its_role() {
    assert!(INSTRUMENT_JS.contains("crossings"));
    assert!(BENCHMARK_JS.contains("RESULT"));
    assert!(DOM_SHIM_JS.contains("document"));
    assert!(ROW_ZD.contains("state rowId"));
    assert!(ROW_ZD.contains("view"));
}
