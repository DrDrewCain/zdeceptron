# Reading a file is not importing one

A runtime `foreign` calls into a real host — a browser, a serverless
runtime — which genuinely has npm and a DOM. A build-time call has no
host at all: the compiler *is* the host.

So the honest construct is not "import a module" but "ask the compiler
for a capability". `build list`, `build read` and `build markdown` are a
closed set the compiler implements itself, and the set being closed is
the cost, stated rather than argued away.
