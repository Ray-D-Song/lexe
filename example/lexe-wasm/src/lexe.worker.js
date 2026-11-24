import {
  WASI,
  File,
  OpenFile,
  ConsoleStdout,
  PreopenDirectory,
} from "@bjorn3/browser_wasi_shim";

let compiled;
let initPromise;

function post(message) {
  self.postMessage(message);
}

class ByteSink {
  constructor() {
    this.decoder = new TextDecoder();
  }

  write(buffer) {
    if (!buffer || buffer.length === 0) {
      return;
    }
    post({ type: this.type, text: this.decoder.decode(buffer, { stream: true }) });
  }
}

class StdoutSink extends ByteSink {
  type = "stdout";
}

class StderrSink extends ByteSink {
  type = "stderr";
}

async function compileWasm(wasmUrl) {
  const response = await fetch(wasmUrl);
  if (!response.ok) {
    throw new Error(
      `Failed to load lexe.wasm (${response.status}). Run \`make wasm\` in the repo root first.`
    );
  }
  const contentType = response.headers.get("content-type") || "";
  if (WebAssembly.compileStreaming && contentType.includes("application/wasm")) {
    return WebAssembly.compileStreaming(response);
  }
  return WebAssembly.compile(await response.arrayBuffer());
}

function ensureCompiled(wasmUrl) {
  if (!initPromise) {
    initPromise = (async () => {
      post({ type: "status", message: "Compiling lexe.wasm…" });
      compiled = await compileWasm(wasmUrl);
      post({ type: "status", message: "Ready. Click Run." });
      post({ type: "ready" });
    })().catch((error) => {
      initPromise = undefined;
      throw error;
    });
  }
  return initPromise;
}

function runEval(code) {
  const stdout = new StdoutSink();
  const stderr = new StderrSink();
  const fds = [
    new OpenFile(new File([])),
    new ConsoleStdout((buffer) => stdout.write(buffer)),
    new ConsoleStdout((buffer) => stderr.write(buffer)),
    new PreopenDirectory(".", new Map()),
  ];
  const wasi = new WASI(
    ["lexe.wasm", "--eval", code],
    [],
    fds,
    { debug: false }
  );
  const instance = new WebAssembly.Instance(compiled, {
    wasi_snapshot_preview1: wasi.wasiImport,
  });
  try {
    return wasi.start(instance);
  } catch (error) {
    if (error && typeof error === "object" && "code" in error) {
      return error.code;
    }
    throw error;
  }
}

self.addEventListener("message", async (event) => {
  const msg = event.data;
  try {
    if (msg.type === "init") {
      await ensureCompiled(msg.wasmUrl);
      return;
    }
    if (msg.type !== "run") {
      return;
    }
    await ensureCompiled(msg.wasmUrl);
    post({ type: "status", message: "Running…" });
    const exitCode = runEval(msg.code) ?? 0;
    post({ type: "done", exitCode });
  } catch (error) {
    post({
      type: "error",
      message: error instanceof Error ? error.stack || error.message : String(error),
    });
  }
});
