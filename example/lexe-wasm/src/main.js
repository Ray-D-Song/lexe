import * as monaco from "monaco-editor";
import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
import { Terminal } from "@xterm/xterm";
import LexeWorker from "./lexe.worker.js?worker";
import "monaco-editor/min/vs/editor/editor.main.css";
import "@xterm/xterm/css/xterm.css";
import "./style.css";

self.MonacoEnvironment = {
  getWorker() {
    return new editorWorker();
  },
};

const DEFAULT_SOURCE = `console.log("Hello from Lexe");
console.log("arch:", process.arch);
console.log("pid:", process.pid);
`;

const runButton = document.getElementById("run");
const editorHost = document.getElementById("editor");
const terminalHost = document.getElementById("terminal");

const editor = monaco.editor.create(editorHost, {
  value: DEFAULT_SOURCE,
  language: "javascript",
  theme: "vs-dark",
  automaticLayout: true,
  minimap: { enabled: false },
  fontSize: 14,
  scrollBeyondLastLine: false,
  tabSize: 2,
});

const term = new Terminal({
  convertEol: true,
  cursorBlink: false,
  disableStdin: true,
  fontSize: 13,
  theme: {
    background: "#000000",
    foreground: "#d4d4d4",
  },
});
term.open(terminalHost);

function fitTerminal() {
  const cols = Math.max(20, Math.floor(terminalHost.clientWidth / 9));
  const rows = Math.max(5, Math.floor(terminalHost.clientHeight / 17));
  term.resize(cols, rows);
}

fitTerminal();
window.addEventListener("resize", fitTerminal);

const worker = new LexeWorker();
let running = false;

function write(text) {
  term.write(String(text).replace(/\n/g, "\r\n"));
}

function setRunning(next) {
  running = next;
  runButton.disabled = next;
  runButton.textContent = next ? "Running…" : "Run";
}

worker.addEventListener("message", (event) => {
  const msg = event.data;
  switch (msg.type) {
    case "status":
      write(`${msg.message}\r\n`);
      break;
    case "ready":
      runButton.disabled = false;
      break;
    case "stdout":
    case "stderr":
      write(msg.text);
      break;
    case "done":
      if (msg.exitCode) {
        write(`\r\n[exit ${msg.exitCode}]\r\n`);
      }
      setRunning(false);
      break;
    case "error":
      write(`\r\n${msg.message}\r\n`);
      setRunning(false);
      break;
    default:
      break;
  }
});

worker.addEventListener("error", (event) => {
  write(`\r\nWorker error: ${event.message || event}\r\n`);
  setRunning(false);
});

function run() {
  if (running) {
    return;
  }
  setRunning(true);
  term.reset();
  worker.postMessage({
    type: "run",
    code: editor.getValue(),
    wasmUrl: new URL("/lexe.wasm", window.location.href).href,
  });
}

runButton.addEventListener("click", run);
editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, run);

term.write("Compiling Lexe WASM…\r\n");
worker.postMessage({
  type: "init",
  wasmUrl: new URL("/lexe.wasm", window.location.href).href,
});
