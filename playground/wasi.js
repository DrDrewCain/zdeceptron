// The host side of `zdc-wasm`: enough of `wasi_snapshot_preview1` to run a
// filter, and nothing else.
//
// # Why there is a shim here at all
//
// The compiler exports no function of its own. `#[no_mangle]` and
// `#[export_name]` are refused by `#![forbid(unsafe_code)]`, which every
// crate in this workspace carries and two CI gates exist to keep meaning
// something — so the usual "export `alloc`, marshal a string through linear
// memory" interface cannot be written here. `wasm-bindgen` would generate
// that `unsafe` from a proc macro, where the lint cannot see it, and add a
// version-locked `wasm-bindgen-cli` step between a contributor and this
// page.
//
// What is left is the oldest interface there is. Built for `wasm32-wasip1`
// the module exports `_start` and imports thirteen WASI calls, and the
// thirteen are below. Built for `wasm32-unknown-unknown` it exports `main`
// and imports *nothing at all* — which sounds better and is useless: with
// no host calls there is no way to hand it a program or read an answer
// back. That is the whole reason this file targets WASI.
//
// # What is implemented, and what is refused
//
// Standard input, standard output, standard error, and the exit code. Every
// other call answers with an error, deliberately:
//
//   * `path_open` and the `fd_prestat_*` pair — there is no filesystem, so
//     a program with a `use` of another module fails its read and the
//     compiler reports it. That refusal is the honest one; a shim that
//     faked a directory would make the playground compile a program `zdc`
//     would not.
//   * `environ_*` — an empty environment. `zdc-diagnostics` consults
//     `NO_COLOR`; the answer here is "unset", which is a fact rather than
//     an accident, and the compiler asks for colour off explicitly anyway.
//
// # One instance per compile
//
// A WASI module runs `_start` once. `WebAssembly.Module` is compiled once
// and reused, so the cost per compile is an instantiation rather than a
// recompile — and no compile can leak state into the next, which for a box
// people will paste anything into is worth having.

// The errno numbers this file returns, from the preview-1 table. Named
// rather than inlined: `8` at a call site says nothing.
const ERRNO_SUCCESS = 0;
const ERRNO_BADF = 8;
const ERRNO_NOTSUP = 58;

const STDIN = 0;
const STDOUT = 1;
const STDERR = 2;

/// Thrown by `proc_exit` to unwind out of `_start`, which never returns
/// normally. Its own class so a genuine trap is not mistaken for an exit.
class Exit extends Error {
  constructor(code) {
    super(`the module exited with ${code}`);
    this.code = code;
  }
}

/// Run one compile: a module, a program, and what it wrote.
///
/// `module` is a compiled `WebAssembly.Module`; `input` is the source text.
/// The answer is `{ stdout, stderr, code }` with both streams decoded as
/// UTF-8 — which is what the compiler writes and what a `<textarea>`
/// produces.
export async function run(module, input) {
  const stdin = new TextEncoder().encode(input);
  let read = 0;
  const stdout = [];
  const stderr = [];
  let memory = null;

  const view = () => new DataView(memory.buffer);
  const bytes = () => new Uint8Array(memory.buffer);

  // `iovs` is an array of (pointer, length) pairs, eight bytes each, and
  // every read and write below walks it the same way.
  const iovecs = (pointer, count) => {
    const out = [];
    const data = view();
    for (let i = 0; i < count; i += 1) {
      out.push({
        buffer: data.getUint32(pointer + i * 8, true),
        length: data.getUint32(pointer + i * 8 + 4, true),
      });
    }
    return out;
  };

  const wasi = {
    fd_write(fd, iovsPointer, iovsCount, writtenPointer) {
      if (fd !== STDOUT && fd !== STDERR) return ERRNO_BADF;
      const sink = fd === STDOUT ? stdout : stderr;
      let written = 0;
      for (const iov of iovecs(iovsPointer, iovsCount)) {
        // Copied rather than referenced: the buffer is detached and
        // replaced whenever the module grows its memory, and a `Uint8Array`
        // kept across that boundary reads zeroes.
        sink.push(bytes().slice(iov.buffer, iov.buffer + iov.length));
        written += iov.length;
      }
      view().setUint32(writtenPointer, written, true);
      return ERRNO_SUCCESS;
    },

    fd_read(fd, iovsPointer, iovsCount, readPointer) {
      if (fd !== STDIN) return ERRNO_BADF;
      let total = 0;
      for (const iov of iovecs(iovsPointer, iovsCount)) {
        const take = Math.min(iov.length, stdin.length - read);
        if (take <= 0) break;
        bytes().set(stdin.subarray(read, read + take), iov.buffer);
        read += take;
        total += take;
      }
      // Zero is end of stream, which is how `read_to_end` terminates.
      view().setUint32(readPointer, total, true);
      return ERRNO_SUCCESS;
    },

    // The standard streams are character devices with no flags. The struct
    // is 24 bytes: a filetype at 0, flags at 2, and two rights bitsets at 8
    // and 16 that this shim leaves as zero — nothing consults them once
    // `fd_read` and `fd_write` answer.
    fd_fdstat_get(fd, pointer) {
      if (fd !== STDIN && fd !== STDOUT && fd !== STDERR) return ERRNO_BADF;
      const data = view();
      for (let offset = 0; offset < 24; offset += 1) data.setUint8(pointer + offset, 0);
      data.setUint8(pointer, 2); // character device
      return ERRNO_SUCCESS;
    },

    fd_filestat_get: () => ERRNO_BADF,
    fd_close: () => ERRNO_SUCCESS,

    // No preopened directory. `EBADF` on the first descriptor is what ends
    // the standard library's scan for them, and it is what makes every
    // later `path_open` of a relative path fail — which is the truth: there
    // is no filesystem in a browser tab.
    fd_prestat_get: () => ERRNO_BADF,
    fd_prestat_dir_name: () => ERRNO_BADF,
    path_open: () => ERRNO_NOTSUP,
    path_readlink: () => ERRNO_NOTSUP,

    environ_sizes_get(countPointer, sizePointer) {
      const data = view();
      data.setUint32(countPointer, 0, true);
      data.setUint32(sizePointer, 0, true);
      return ERRNO_SUCCESS;
    },
    environ_get: () => ERRNO_SUCCESS,

    // Reached by the standard library's hash seed, not by the compiler.
    // The browser has a real entropy source, which is exactly what
    // `getrandom` could not assume for `wasm32-unknown-unknown` and what
    // made the whole feature seam necessary.
    random_get(pointer, length) {
      crypto.getRandomValues(bytes().subarray(pointer, pointer + length));
      return ERRNO_SUCCESS;
    },

    proc_exit(code) {
      throw new Exit(code);
    },
  };

  const instance = await WebAssembly.instantiate(module, {
    wasi_snapshot_preview1: wasi,
  });
  memory = instance.exports.memory;

  let code = 0;
  try {
    instance.exports._start();
  } catch (error) {
    if (!(error instanceof Exit)) throw error;
    code = error.code;
  }

  return {
    stdout: decode(stdout),
    stderr: decode(stderr),
    code,
  };
}

function decode(chunks) {
  let length = 0;
  for (const chunk of chunks) length += chunk.length;
  const joined = new Uint8Array(length);
  let at = 0;
  for (const chunk of chunks) {
    joined.set(chunk, at);
    at += chunk.length;
  }
  return new TextDecoder().decode(joined);
}
