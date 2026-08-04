//! `zdc lsp`, driven as a real process over a real pipe.
//!
//! The unit tests in `zdc-lsp` check what each feature answers. This
//! checks the part they cannot: that the subcommand exists, that it speaks
//! the protocol's framing on stdin and stdout, that it survives a
//! malformed message, and that it shuts down when asked. All four break
//! silently — an editor with a language server that failed to start shows
//! no error, only an absence of features.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// A running `zdc lsp`, with the request id counter its caller needs.
struct Server {
    process: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl Server {
    fn start() -> Server {
        let mut process = Command::new(env!("CARGO_BIN_EXE_zdc"))
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to run `zdc lsp`");

        let stdin = process.stdin.take().expect("a pipe to the server");
        let stdout = BufReader::new(process.stdout.take().expect("a pipe from the server"));
        Server {
            process,
            stdin,
            stdout,
            next_id: 0,
        }
    }

    /// Write one message with the protocol's `Content-Length` framing.
    fn send(&mut self, message: &serde_json::Value) {
        let body = serde_json::to_string(message).expect("a serializable message");
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len())
            .expect("failed to write to the server");
        self.stdin.flush().expect("failed to flush");
    }

    fn notify(&mut self, method: &str, params: serde_json::Value) {
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }));
    }

    /// Send a request and return its id.
    fn request(&mut self, method: &str, params: serde_json::Value) -> i64 {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }));
        id
    }

    /// Read one framed message.
    fn receive(&mut self) -> serde_json::Value {
        let mut length = None;
        loop {
            let mut header = String::new();
            let read = self
                .stdout
                .read_line(&mut header)
                .expect("failed to read a header");
            assert!(read > 0, "the server closed the connection early");
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            if let Some(value) = header.strip_prefix("Content-Length: ") {
                length = Some(value.parse::<usize>().expect("a numeric content length"));
            }
        }

        let length = length.expect("every message is framed with a length");
        let mut body = vec![0u8; length];
        self.stdout
            .read_exact(&mut body)
            .expect("failed to read a message body");
        serde_json::from_slice(&body).expect("the server emitted valid JSON")
    }

    /// Read messages until one is the response to `id`.
    fn response_to(&mut self, id: i64) -> serde_json::Value {
        for _ in 0..64 {
            let message = self.receive();
            if message.get("id").and_then(|id| id.as_i64()) == Some(id)
                && message.get("method").is_none()
            {
                return message;
            }
        }
        panic!("no response to request {id} arrived");
    }

    /// Read messages until one is a `publishDiagnostics` notification.
    fn diagnostics(&mut self) -> Vec<serde_json::Value> {
        for _ in 0..64 {
            let message = self.receive();
            if message.get("method").and_then(|m| m.as_str())
                == Some("textDocument/publishDiagnostics")
            {
                return message["params"]["diagnostics"]
                    .as_array()
                    .expect("a diagnostics array")
                    .clone();
            }
        }
        panic!("no diagnostics arrived");
    }

    fn initialize(&mut self) -> serde_json::Value {
        let id = self.request(
            "initialize",
            serde_json::json!({ "capabilities": {}, "processId": null }),
        );
        let response = self.response_to(id);
        self.notify("initialized", serde_json::json!({}));
        response
    }

    fn open(&mut self, text: &str) {
        self.notify(
            "textDocument/didOpen",
            serde_json::json!({
                "textDocument": {
                    "uri": URI,
                    "languageId": "zdeceptron",
                    "version": 1,
                    "text": text,
                }
            }),
        );
    }

    /// Ask the server to stop, and assert that it actually does.
    ///
    /// Polled rather than waited on: a server that never exits is the
    /// failure being tested for, and blocking on it would turn a clear
    /// failure into a hung suite.
    fn shut_down(mut self) {
        let id = self.request("shutdown", serde_json::Value::Null);
        let _ = self.response_to(id);
        self.notify("exit", serde_json::Value::Null);

        for _ in 0..100 {
            match self.process.try_wait().expect("failed to poll the server") {
                Some(status) => {
                    assert!(status.success(), "the server exited with {status}");
                    return;
                }
                None => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        }
        panic!("the server did not exit within five seconds of being told to");
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

const URI: &str = "file:///counter.zd";

const COUNTER: &str = "state count is client Whole starting 0\n\
                       state votes is durable Whole starting 0\n\
                       view\n    Text count\n";

/// The same program without the `durable` signal, for the tests that need a
/// file the compiler has nothing at all to say about. `COUNTER` is not one:
/// the emitter refuses a placement boundary until M6 (§16.5), and the
/// editor shows that refusal now that it runs the emitter.
const CLEAN: &str = "state count is client Whole starting 0\nview\n    Text count\n";

#[test]
fn the_server_starts_advertises_its_features_and_shuts_down() {
    let mut server = Server::start();
    let response = server.initialize();
    let capabilities = &response["result"]["capabilities"];

    assert!(capabilities["hoverProvider"].as_bool().unwrap_or(false));
    assert!(capabilities["definitionProvider"]
        .as_bool()
        .unwrap_or(false));
    assert!(capabilities["completionProvider"].is_object());

    let legend = &capabilities["semanticTokensProvider"]["legend"];
    let types = legend["tokenTypes"].as_array().expect("token types");
    let modifiers = legend["tokenModifiers"].as_array().expect("modifiers");
    assert!(types.iter().any(|t| t == "keyword"));
    // The placement modifiers are what make the boundary visible.
    for placement in ["client", "server", "durable"] {
        assert!(
            modifiers.iter().any(|m| m == placement),
            "{placement} is missing from the legend"
        );
    }

    server.shut_down();
}

#[test]
fn opening_a_broken_file_publishes_every_diagnostic() {
    let mut server = Server::start();
    server.initialize();
    server.open(
        "state a is client Whole from one\n\
         state b is client Whole from two\n\
         state c is client Whole from three\n",
    );

    let diagnostics = server.diagnostics();
    assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");
    for diagnostic in &diagnostics {
        assert_eq!(diagnostic["source"], "zdc");
        assert_eq!(diagnostic["severity"], 1);
    }

    server.shut_down();
}

#[test]
fn editing_a_file_republishes_and_clears() {
    let mut server = Server::start();
    server.initialize();
    server.open("state a is client Whole from missing\n");
    assert_eq!(server.diagnostics().len(), 1);

    server.notify(
        "textDocument/didChange",
        serde_json::json!({
            "textDocument": { "uri": URI, "version": 2 },
            "contentChanges": [{ "text": "state a is client Whole starting 0\n" }],
        }),
    );
    assert!(server.diagnostics().is_empty());

    server.shut_down();
}

/// The hover this whole crate exists for: reading `durable` state names
/// where the value lives.
#[test]
fn hover_says_where_a_value_lives() {
    let mut server = Server::start();
    server.initialize();
    server.open(COUNTER);
    let _ = server.diagnostics();

    let id = server.request(
        "textDocument/hover",
        serde_json::json!({
            "textDocument": { "uri": URI },
            "position": { "line": 1, "character": 7 },
        }),
    );
    let response = server.response_to(id);
    let value = response["result"]["contents"]["value"]
        .as_str()
        .expect("markdown contents");
    assert!(value.contains("persistent store"), "{value}");
    assert!(value.contains("durable"), "{value}");

    server.shut_down();
}

#[test]
fn definition_points_at_the_declaration() {
    let mut server = Server::start();
    server.initialize();
    server.open(COUNTER);
    let _ = server.diagnostics();

    // The `count` in `Text count`, on the fourth line.
    let id = server.request(
        "textDocument/definition",
        serde_json::json!({
            "textDocument": { "uri": URI },
            "position": { "line": 3, "character": 10 },
        }),
    );
    let response = server.response_to(id);
    assert_eq!(response["result"]["uri"], URI);
    assert_eq!(response["result"]["range"]["start"]["line"], 0);
    assert_eq!(response["result"]["range"]["start"]["character"], 6);

    server.shut_down();
}

#[test]
fn semantic_tokens_arrive_as_a_multiple_of_five_integers() {
    let mut server = Server::start();
    server.initialize();
    server.open(COUNTER);
    let _ = server.diagnostics();

    let id = server.request(
        "textDocument/semanticTokens/full",
        serde_json::json!({ "textDocument": { "uri": URI } }),
    );
    let response = server.response_to(id);
    let data = response["result"]["data"].as_array().expect("token data");
    assert!(!data.is_empty());
    assert_eq!(data.len() % 5, 0, "the encoding is five integers per token");

    server.shut_down();
}

#[test]
fn completion_offers_the_placements_after_a_declarations_is() {
    let mut server = Server::start();
    server.initialize();
    server.open("state count is ");
    let _ = server.diagnostics();

    let id = server.request(
        "textDocument/completion",
        serde_json::json!({
            "textDocument": { "uri": URI },
            "position": { "line": 0, "character": 15 },
        }),
    );
    let response = server.response_to(id);
    let labels: Vec<&str> = response["result"]
        .as_array()
        .expect("completion items")
        .iter()
        .filter_map(|item| item["label"].as_str())
        .collect();
    assert_eq!(labels, ["client", "static", "server", "durable"]);

    server.shut_down();
}

/// A request the server does not implement, a request whose parameters
/// are nonsense, and a request about a file that was never opened. None
/// of the three may end the session — the next request must still work.
#[test]
fn the_server_survives_messages_it_cannot_use() {
    let mut server = Server::start();
    server.initialize();

    let unknown = server.request("textDocument/rename", serde_json::json!({}));
    assert!(server.response_to(unknown)["error"].is_object());

    let malformed = server.request("textDocument/hover", serde_json::json!({ "nope": true }));
    assert!(server.response_to(malformed)["error"].is_object());

    let unopened = server.request(
        "textDocument/hover",
        serde_json::json!({
            "textDocument": { "uri": "file:///never-opened.zd" },
            "position": { "line": 0, "character": 0 },
        }),
    );
    let response = server.response_to(unopened);
    assert!(response["error"].is_null());
    assert!(response["result"].is_null());

    server.notify("textDocument/didSomethingElse", serde_json::json!({}));

    // Still alive, and still answering.
    server.open(CLEAN);
    assert!(server.diagnostics().is_empty());

    server.shut_down();
}

/// A file that is not a program at all, arriving one keystroke at a time.
/// The session must outlive every one of them.
#[test]
fn the_server_survives_a_file_being_typed_into() {
    let mut server = Server::start();
    server.initialize();
    server.open("");
    let _ = server.diagnostics();

    let target = "state count is client Whole starting 0\nview\n    Text count\n";
    for (version, upto) in (1..=target.len()).enumerate() {
        let Some(text) = target.get(..upto) else {
            continue;
        };
        server.notify(
            "textDocument/didChange",
            serde_json::json!({
                "textDocument": { "uri": URI, "version": version + 2 },
                "contentChanges": [{ "text": text }],
            }),
        );
        let _ = server.diagnostics();
    }

    // The last prefix is the whole file, which compiles.
    let id = server.request(
        "textDocument/semanticTokens/full",
        serde_json::json!({ "textDocument": { "uri": URI } }),
    );
    assert!(!server.response_to(id)["result"]["data"]
        .as_array()
        .expect("token data")
        .is_empty());

    server.shut_down();
}
