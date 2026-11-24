import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const exampleDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(exampleDir, "../..");
const wasmPath = path.join(
  repoRoot,
  "target/wasm32-wasip1/debug/lexe.wasm"
);

function sendWasm(res) {
  if (!fs.existsSync(wasmPath)) {
    res.statusCode = 404;
    res.setHeader("Content-Type", "text/plain; charset=utf-8");
    res.end("lexe.wasm not found. Run `make wasm` in the repo root first.");
    return;
  }
  res.setHeader("Content-Type", "application/wasm");
  fs.createReadStream(wasmPath).pipe(res);
}

function wasmMiddleware(req, res, next) {
  const url = req.url?.split("?")[0];
  if (url === "/lexe.wasm") {
    sendWasm(res);
    return;
  }
  next();
}

export default defineConfig({
  server: {
    fs: {
      allow: [repoRoot],
    },
  },
  plugins: [
    {
      name: "lexe-wasm",
      configureServer(server) {
        server.middlewares.use(wasmMiddleware);
      },
      configurePreviewServer(server) {
        server.middlewares.use(wasmMiddleware);
      },
      closeBundle() {
        if (!fs.existsSync(wasmPath)) {
          return;
        }
        const outDir = path.resolve(exampleDir, "dist");
        fs.mkdirSync(outDir, { recursive: true });
        fs.copyFileSync(wasmPath, path.join(outDir, "lexe.wasm"));
      },
    },
  ],
  worker: {
    format: "es",
  },
});
