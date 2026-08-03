use zdc_runtime::{Sandbox, RPC_JS, SIGNAL_JS, STORE_JS, WIRE_JS};

fn without_imports(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("import "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn rpc() -> Sandbox {
    let mut sandbox = Sandbox::new();
    sandbox.load(SIGNAL_JS).expect("signal module loads");
    sandbox.load(WIRE_JS).expect("wire module loads");
    sandbox
        .load(&without_imports(RPC_JS))
        .expect("RPC module loads");
    sandbox
}

fn store() -> Sandbox {
    let mut sandbox = rpc();
    sandbox
        .load("const decodeValue = decode;")
        .expect("wire import alias installs");
    sandbox
        .load(&without_imports(STORE_JS))
        .expect("store module loads");
    sandbox
}

#[test]
fn endpoint_urls_encode_names_as_one_path_segment() {
    let mut sandbox = rpc();
    assert_eq!(
        sandbox.text("endpointUrl('visits.incr')").unwrap(),
        "/_zd/visits.incr"
    );
    assert_eq!(
        sandbox.text("endpointUrl('a/b c')").unwrap(),
        "/_zd/a%2Fb%20c"
    );
    assert_eq!(
        sandbox.text("endpointUrl('café')").unwrap(),
        "/_zd/caf%C3%A9"
    );
}

#[test]
fn the_atomic_endpoint_name_cannot_collide_with_a_language_identifier() {
    let mut sandbox = rpc();
    assert_eq!(sandbox.text("ATOMIC").unwrap(), "~atomic");
    assert_eq!(sandbox.text("endpointUrl(ATOMIC)").unwrap(), "/_zd/~atomic");
}

#[test]
fn calls_preserve_endpoint_argument_order_and_atomic_batch_shape() {
    let mut sandbox = rpc();
    sandbox
        .load(
            "const seen = []; setTransport((name, args) => { seen.push([name, args]); return null; });",
        )
        .unwrap();
    sandbox
        .load("call('single', 1, 'two'); atomic([]); atomic([['first', [3]], ['second', [4, 5]]]);")
        .unwrap();

    assert_eq!(
        sandbox.text("JSON.stringify(seen)").unwrap(),
        r#"[["single",[1,"two"]],["~atomic",[["first",[3]],["second",[4,5]]]]]"#
    );
}

#[test]
fn absent_atomic_batches_do_not_make_transport_requests() {
    let mut sandbox = rpc();
    sandbox
        .load("let calls = 0; setTransport(() => { calls += 1; });")
        .unwrap();
    sandbox.load("atomic(); atomic(null); atomic([]);").unwrap();

    assert_eq!(sandbox.text("calls").unwrap(), "0");
}

#[test]
fn custom_failure_sinks_receive_the_original_error() {
    let mut sandbox = rpc();
    sandbox
        .load("let reported = null; setFailureSink(error => { reported = error; });")
        .unwrap();
    sandbox
        .load("const originalFailure = new Error('write failed'); reportFailure(originalFailure);")
        .unwrap();

    assert_eq!(
        sandbox.text("reported === originalFailure").unwrap(),
        "true"
    );
    assert_eq!(sandbox.text("reported.message").unwrap(), "write failed");
}

#[test]
fn a_fresh_store_has_no_watched_keys_or_bound_updates() {
    let mut sandbox = store();
    assert_eq!(sandbox.text("JSON.stringify(watchedKeys())").unwrap(), "[]");
    assert_eq!(sandbox.text("applyUpdate('missing', 1)").unwrap(), "false");
    assert_eq!(
        sandbox.text("typeof subscribe({keys: []})").unwrap(),
        "function"
    );
}

#[test]
fn live_sync_receive_advances_only_numeric_cursors() {
    let mut sandbox = store();
    assert_eq!(
        sandbox
            .text("receive({event: 'ready', seq: 8}, 3)")
            .unwrap(),
        "8"
    );
    assert_eq!(
        sandbox
            .text("receive({event: 'newer', seq: '9'}, 3)")
            .unwrap(),
        "3"
    );
    assert_eq!(
        sandbox
            .text("receive({event: 'update', key: 'none', value: 1}, 4)")
            .unwrap(),
        "4"
    );
    assert_eq!(sandbox.text("receive({event: 'resync'}, 5)").unwrap(), "5");
}

#[test]
fn stream_frames_decode_payloads_cursors_and_nested_maps() {
    let mut sandbox = store();
    let expression = r#"
        (() => {
          const frame = decodeFrame(
            'update',
            '{"key":"cards","value":{"$map":[["ada",1]]}}',
            '12'
          );
          return frame.event === 'update' && frame.seq === 12 && frame.key === 'cards' &&
            frame.value instanceof Map && frame.value.get('ada') === 1;
        })()
    "#;
    assert_eq!(sandbox.text(expression).unwrap(), "true");
}

#[test]
fn malformed_stream_frames_are_safe_empty_events() {
    let mut sandbox = store();
    assert_eq!(
        sandbox
            .text(
                "(() => { const f = decodeFrame('ready', '{bad', 'not-a-number'); return f.event === 'ready' && f.seq === undefined && f.key === undefined && f.value === null; })()",
            )
            .unwrap(),
        "true"
    );
}
