// Launches `zdc lsp` and speaks the Language Server Protocol to it.
//
// This file is the whole of the extension's logic. Everything a user sees
// beyond syntax colouring — diagnostics, hover, go to definition, semantic
// tokens, completion — is computed by the compiler, on the other side of a
// pipe. Nothing about the language is modelled here, deliberately: a second
// model of the grammar is the thing `README.md` explains this extension
// exists to avoid.

const vscode = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

/** @type {LanguageClient | undefined} */
let client;

function activate(context) {
  const command = vscode.workspace
    .getConfiguration("zdeceptron")
    .get("server.path", "zdc");

  // `zdc lsp` reads and writes the protocol on stdin and stdout, so the
  // transport is stdio and there is no port to configure. It prints
  // nothing else on stdout; anything it has to say about itself goes to
  // stderr, which the client surfaces in the output channel below.
  const server = { command, args: ["lsp"], transport: TransportKind.stdio };

  client = new LanguageClient(
    "zdeceptron",
    "ZDeceptron",
    { run: server, debug: server },
    {
      documentSelector: [{ scheme: "file", language: "zdeceptron" }],
      outputChannelName: "ZDeceptron",
    },
  );

  context.subscriptions.push({ dispose: () => client && client.stop() });

  client.start().catch((error) => {
    // The overwhelmingly likely cause is that `zdc` is not on PATH, so say
    // that rather than repeating a spawn error the user cannot act on.
    vscode.window.showErrorMessage(
      `ZDeceptron: could not start \`${command} lsp\`. ` +
        "Install the compiler, or set `zdeceptron.server.path` to its " +
        `location. (${error && error.message ? error.message : error})`,
    );
  });
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
