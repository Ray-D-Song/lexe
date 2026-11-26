import defaultImport from "node:stream";
import legacyImport from "stream";
import Module from "node:module";

const _require = require;

it("node:stream should be the same as stream", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
});

it("require('stream') matches the ESM default export", () => {
  const required = _require("stream");
  expect(required).toStrictEqual(legacyImport);
});

it("require('node:stream') shares the same cache object as bare require", () => {
  expect(_require("node:stream")).toBe(_require("stream"));
});

it("require('stream/promises') and node: alias share one cache object", () => {
  const bare = _require("stream/promises");
  const aliased = _require("node:stream/promises");
  expect(bare).toBe(aliased);
  expect(bare).toBeDefined();
});

it("Module.isBuiltin recognizes stream and stream/promises", () => {
  expect(Module.isBuiltin("stream")).toBe(true);
  expect(Module.isBuiltin("node:stream")).toBe(true);
  expect(Module.isBuiltin("stream/promises")).toBe(true);
  expect(Module.isBuiltin("node:stream/promises")).toBe(true);
});

it("Module.builtinModules lists public embedded stream modules only", () => {
  expect(Module.builtinModules).toContain("stream");
  expect(Module.builtinModules).toContain("stream/promises");
  expect(Module.builtinModules).not.toContain("lexe:test/index");
});

import {
  TextEncoderStream,
  TextDecoderStream,
  ReadableStream,
  WritableStream,
} from "node:stream/web";

it("exposes encoding streams on globals and stream/web", () => {
  expect(globalThis.TextEncoderStream).toBe(TextEncoderStream);
  expect(globalThis.TextDecoderStream).toBe(TextDecoderStream);
  expect(new TextEncoderStream().encoding).toBe("utf-8");
});

it("uses Lexe stream instances for encoding stream sides", () => {
  const encoder = new TextEncoderStream();
  expect(encoder.readable).toBeInstanceOf(ReadableStream);
  expect(encoder.writable).toBeInstanceOf(WritableStream);
  expect(new TextDecoderStream().encoding).toBe("utf-8");
});

it("BYOB read preserves Uint8ClampedArray type and content", async () => {
  const stream = new ReadableStream({
    type: "bytes",
    start(controller) {
      controller.enqueue(new Uint8Array([1, 2, 3, 4, 5]));
      controller.close();
    },
  });
  const reader = stream.getReader({ mode: "byob" });
  const { value, done } = await reader.read(new Uint8ClampedArray(3));
  expect(done).toEqual(false);
  expect(value).toBeInstanceOf(Uint8ClampedArray);
  expect([...value!]).toEqual([1, 2, 3]);
  await reader.cancel();
});

describe("stream state predicates and helpers", () => {
  it("exposes the Node helper surface on both the namespace and default export", async () => {
    const S = await import("node:stream");
    const names = [
      "isReadable",
      "isWritable",
      "isDestroyed",
      "isDisturbed",
      "isErrored",
      "isNodeStream",
      "compose",
      "duplexPair",
      "addAbortSignal",
      "getDefaultHighWaterMark",
      "setDefaultHighWaterMark",
      "destroy",
    ];
    for (const name of names) {
      expect(typeof (S as any)[name]).toBe("function");
      expect(typeof (S.default as any)[name]).toBe("function");
    }
  });

  it("predicates return null for non-streams and booleans otherwise", async () => {
    const { Readable, isReadable, isWritable, isDestroyed, isNodeStream } =
      await import("node:stream");

    const r = Readable.from([Buffer.from("x")]);
    expect(isReadable(r)).toBe(true);
    // A Readable has no writable side: Node reports null, not false.
    expect(isWritable(r)).toBe(null);
    expect(isDestroyed(r)).toBe(false);
    expect(isNodeStream({})).toBe(false);
    expect(isReadable({})).toBe(null);
    expect(isWritable({})).toBe(null);

    r.destroy();
    await new Promise((res) => setTimeout(res, 10));
    expect(isDestroyed(r)).toBe(true);
    expect(isReadable(r)).toBe(false);
  });

  it("get/setDefaultHighWaterMark use object and byte modes", async () => {
    const S = await import("node:stream");
    expect(S.getDefaultHighWaterMark()).toBe(16384);
    expect(S.getDefaultHighWaterMark(true)).toBe(16);

    S.setDefaultHighWaterMark(true, 32);
    expect(S.getDefaultHighWaterMark(true)).toBe(32);
    // Bytes mode is unaffected by the object-mode change.
    expect(S.getDefaultHighWaterMark()).toBe(16384);
    S.setDefaultHighWaterMark(true, 16);
  });

  it("duplexPair wires two duplex streams together", async () => {
    const { duplexPair } = await import("node:stream");
    const [a, b] = duplexPair();
    const received: string[] = [];
    b.on("data", (c: Buffer) => received.push(c.toString()));
    a.write("ping");
    a.end();
    await new Promise((res) => setTimeout(res, 30));
    expect(received.join("")).toBe("ping");
  });

  it("compose pipes streams left to right", async () => {
    const { Readable, PassThrough, compose } = await import("node:stream");
    const upper = new PassThrough();
    upper.on("data", () => {});
    const out: string[] = [];
    const sink = new PassThrough();
    sink.on("data", (c: Buffer) => out.push(c.toString().toUpperCase()));
    compose(Readable.from([Buffer.from("ab")]), upper, sink);
    await new Promise((res) => setTimeout(res, 30));
    expect(out.join("")).toBe("AB");
  });

  it("addAbortSignal destroys the stream when the signal aborts", async () => {
    const { PassThrough, addAbortSignal } = await import("node:stream");
    const ac = new AbortController();
    const stream = new PassThrough();
    addAbortSignal(ac.signal, stream);
    ac.abort();
    await new Promise((res) => setTimeout(res, 10));
    expect(stream.destroyed).toBe(true);
  });
});

describe("bare specifier with trailing separator", () => {
  it("require('string_decoder/') resolves to the builtin", () => {
    // Bundled CommonJS helpers (inside node:stream) emit exactly this form.
    // Regression: with a node_modules directory present, resolution fell through
    // to the package resolver and node:stream failed to load entirely.
    const r = _require("string_decoder/");
    expect(typeof r.StringDecoder).toBe("function");
    expect(new r.StringDecoder("utf8").write(Buffer.from("hi"))).toBe("hi");
  });

  it("node:stream still loads from a directory that has node_modules", async () => {
    const S = await import("node:stream");
    expect(typeof S.Readable).toBe("function");
    expect(typeof S.isReadable).toBe("function");
  });
});

describe("destroy() without an argument", () => {
  it("closes cleanly instead of throwing undefined", async () => {
    const { PassThrough } = await import("node:stream");
    const stream = new PassThrough();
    // `destroy()` with no argument broadcasts `Some(undefined)` internally.
    // It used to be treated as an error and rethrown, so callers got a bare
    // `undefined` with no message or stack.
    await new Promise<void>((resolve) => {
      stream.on("close", resolve);
      stream.destroy();
    });
    expect(stream.destroyed).toBe(true);
  });

  it("still surfaces a real error passed to destroy()", async () => {
    const { PassThrough } = await import("node:stream");
    const stream = new PassThrough();
    const boom = new Error("boom");
    const seen = new Promise<Error>((resolve) => stream.on("error", resolve));
    stream.destroy(boom);
    await expect(seen).resolves.toBe(boom);
  });

  it("stream.finished returns a cleanup function and fires callback on end", async () => {
    const { PassThrough, finished } = await import("node:stream");
    const pt = new PassThrough();
    let calls = 0;
    let seenErr: any = "unset";
    const cleanup = finished(pt, { readable: true, writable: false }, (err: any) => {
      calls++;
      seenErr = err;
    });
    expect(typeof cleanup).toBe("function");
    pt.resume();
    pt.end("done");
    await new Promise((r) => setTimeout(r, 20));
    expect(calls).toBe(1);
    expect(seenErr).toBeUndefined();

    // Calling cleanup before end suppresses callback
    const pt2 = new PassThrough();
    let suppressedCalls = 0;
    const cleanup2 = finished(pt2, { readable: true, writable: false }, () => {
      suppressedCalls++;
    });
    expect(typeof cleanup2).toBe("function");
    cleanup2();
    pt2.resume();
    pt2.end("x");
    await new Promise((r) => setTimeout(r, 20));
    expect(suppressedCalls).toBe(0);
  });
});
