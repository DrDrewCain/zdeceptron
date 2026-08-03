use std::sync::Arc;

use zdc_host::batch;
use zdc_host::{Endpoint, Endpoints, Environment, Host, HostError, Shape};
use zdc_store::{DurableStore, EmbeddedStore};

fn endpoint(name: &str, shape: Shape) -> Endpoint {
    Endpoint {
        name: name.into(),
        shape,
        inputs: match shape {
            Shape::Value => vec!["input".into()],
            Shape::Command => Vec::new(),
        },
        source: format!("export default function handler() {{ return '{name}'; }}"),
    }
}

#[test]
fn endpoint_shapes_round_trip_through_the_manifest_vocabulary() {
    for (shape, word) in [(Shape::Value, "value"), (Shape::Command, "command")] {
        assert_eq!(shape.word(), word);
        assert_eq!(Shape::parse(word), Some(shape));
    }
}

#[test]
fn endpoint_shape_words_are_exact_and_case_sensitive() {
    for invalid in ["", "Value", "COMMAND", "values", "write", " command"] {
        assert_eq!(Shape::parse(invalid), None, "accepted `{invalid}`");
    }
}

#[test]
fn endpoint_names_iterate_in_deterministic_lexical_order() {
    let endpoints = Endpoints::from_iter([
        endpoint("zeta.set", Shape::Command),
        endpoint("alpha.read", Shape::Value),
        endpoint("middle.incr", Shape::Command),
    ]);

    assert_eq!(
        endpoints.names().collect::<Vec<_>>(),
        ["alpha.read", "middle.incr", "zeta.set"]
    );
    assert_eq!(endpoints.len(), 3);
    assert!(!endpoints.is_empty());
}

#[test]
fn inserting_the_same_endpoint_name_replaces_its_definition() {
    let mut endpoints = Endpoints::default();
    endpoints.insert(endpoint("counter", Shape::Value));
    endpoints.insert(endpoint("counter", Shape::Command));

    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints.get("counter").unwrap().shape, Shape::Command);
    assert_eq!(endpoints.get("count"), None, "lookup must not use prefixes");
}

#[test]
fn environments_are_empty_by_default_and_support_replacement() {
    let mut environment = Environment::default();
    assert_eq!(environment.get("TOKEN"), None);

    environment.set("TOKEN", "first");
    environment.set("TOKEN", "second");
    assert_eq!(environment.get("TOKEN"), Some("second"));
}

#[test]
fn environment_pairs_use_the_last_value_for_a_duplicate_key() {
    let environment =
        Environment::from_pairs([("TOKEN", "old"), ("REGION", "north"), ("TOKEN", "new")]);

    assert_eq!(environment.get("TOKEN"), Some("new"));
    assert_eq!(environment.get("REGION"), Some("north"));
    assert_eq!(environment.get("MISSING"), None);
}

#[test]
fn host_errors_have_stable_statuses_and_public_messages() {
    let cases = [
        (
            HostError::Unknown {
                name: "missing".into(),
            },
            404,
            "`missing` is not an endpoint in this build",
        ),
        (
            HostError::BadRequest {
                message: "arguments must be an array".into(),
            },
            400,
            "arguments must be an array",
        ),
        (
            HostError::Failed {
                endpoint: "counter.incr".into(),
                message: "could not commit".into(),
                detail: Some("secret store detail".into()),
            },
            500,
            "`counter.incr` failed: could not commit",
        ),
    ];

    for (error, status, public_message) in cases {
        assert_eq!(error.status(), status);
        assert_eq!(error.to_string(), public_message);
    }
}

#[test]
fn failure_detail_is_opt_in_and_never_part_of_display() {
    let error = HostError::Failed {
        endpoint: "profile.load".into(),
        message: "configuration is missing".into(),
        detail: Some("API_SECRET_KEY was absent".into()),
    };

    assert_eq!(error.detail(), Some("API_SECRET_KEY was absent"));
    assert!(!error.to_string().contains("API_SECRET_KEY"));
    assert_eq!(HostError::Unknown { name: "x".into() }.detail(), None);
    assert_eq!(
        HostError::BadRequest {
            message: "bad".into()
        }
        .detail(),
        None
    );
}

#[test]
fn host_accessors_return_the_exact_dependencies_it_was_constructed_with() {
    let endpoints = Endpoints::from_iter([endpoint("value", Shape::Value)]);
    let store: Arc<dyn DurableStore> = Arc::new(EmbeddedStore::in_memory().unwrap());
    let host = Host::new(endpoints.clone(), Arc::clone(&store), Environment::empty());

    assert_eq!(host.endpoints(), &endpoints);
    assert!(Arc::ptr_eq(host.store(), &store));
}

#[test]
fn transaction_parsing_preserves_argument_json_verbatim() {
    let body = " [ [ \"first.set\" , [ {\"nested\":[1,\"]\"]} ] ], [\"second.incr\",[-2]] ] ";
    let calls = batch::parse(body).expect("valid transaction");

    assert_eq!(
        calls,
        [
            ("first.set".into(), "[ {\"nested\":[1,\"]\"]} ]".into()),
            ("second.incr".into(), "[-2]".into()),
        ]
    );
}

#[test]
fn transaction_parser_refuses_every_truncated_prefix_without_panicking() {
    let complete = "[[\"counter.incr\",[1]],[\"label.set\",[\"ok\"]]]";

    for boundary in complete.char_indices().map(|(offset, _)| offset).skip(1) {
        let prefix = &complete[..boundary];
        assert!(batch::parse(prefix).is_err(), "accepted prefix `{prefix}`");
    }
    assert!(batch::parse(complete).is_ok());
}
