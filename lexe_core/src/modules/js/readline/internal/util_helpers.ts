// Local util helpers for readline port.
import { stripVTControlCharacters } from "node:util";

// Prefer Rust-backed display width when available; fall back to code-point heuristic.
let stringWidthFn: ((s: string) => number) | null = null;
try {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const ru = require("lexe:util") as { stringWidth?: (s: string) => number };
  if (typeof ru?.stringWidth === "function") stringWidthFn = ru.stringWidth.bind(ru);
} catch {
  /* optional */
}

export const kEmptyObject = Object.freeze(Object.create(null));

export function assignFunctionName(name: string | symbol, fn: Function) {
  try {
    Object.defineProperty(fn, "name", { value: String(name), configurable: true });
  } catch {
    /* ignore */
  }
  return fn;
}

export function getStringWidth(str: string, removeControlChars = true): number {
  if (removeControlChars) str = stripVTControlCharacters(str);
  if (stringWidthFn) return stringWidthFn(str);
  let width = 0;
  for (const ch of str) {
    const cp = ch.codePointAt(0)!;
    // Rough East-Asian width: fullwidth / emoji ranges count as 2
    if (
      (cp >= 0x1100 &&
        (cp <= 0x115f ||
          cp === 0x2329 ||
          cp === 0x232a ||
          (cp >= 0x2e80 && cp <= 0xa4cf && cp !== 0x303f) ||
          (cp >= 0xac00 && cp <= 0xd7a3) ||
          (cp >= 0xf900 && cp <= 0xfaff) ||
          (cp >= 0xfe10 && cp <= 0xfe19) ||
          (cp >= 0xfe30 && cp <= 0xfe6f) ||
          (cp >= 0xff00 && cp <= 0xff60) ||
          (cp >= 0xffe0 && cp <= 0xffe6) ||
          (cp >= 0x20000 && cp <= 0x3fffd))) ||
      (cp >= 0x1f300 && cp <= 0x1f64f)
    ) {
      width += 2;
    } else if (cp <= 0x1f || (cp >= 0x7f && cp <= 0x9f)) {
      // control
    } else {
      width += 1;
    }
  }
  return width;
}

export function inspect(value: unknown): string {
  try {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const util = require("node:util") as { inspect?: (v: unknown) => string };
    if (typeof util.inspect === "function") return util.inspect(value);
  } catch {
    /* ignore */
  }
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

export function addAbortListener(signal: AbortSignal, listener: () => void) {
  signal.addEventListener("abort", listener, { once: true });
  return {
    [Symbol.dispose || Symbol.for("nodejs.dispose")]() {
      signal.removeEventListener("abort", listener);
    },
  };
}

export function isWritable(stream: unknown): boolean {
  return (
    stream != null &&
    typeof stream === "object" &&
    typeof (stream as { write?: unknown }).write === "function"
  );
}
