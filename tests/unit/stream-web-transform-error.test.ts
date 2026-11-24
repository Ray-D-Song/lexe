import { ReadableStream, TransformStream } from "node:stream/web";

// A transform that fails must also error the writable side, so the promises the
// writable side handed out settle instead of hanging forever. Skipping that
// step leaves a pending write that a `pipeThrough` consumer waits on, and the
// process then dies with a GC assertion at exit.
//
// Two behaviours to keep in mind when writing cases here, both matching Node:
// the transform only runs once the readable side is being consumed (under
// backpressure a pending write is expected to stay pending), and a write that
// reached the transform fulfils even when the stream errors right after - it is
// `ready`/`closed` that reject.

const streamOf = (bytes: Uint8Array): any =>
  new ReadableStream({
    start(controller: any) {
      controller.enqueue(bytes);
      controller.close();
    },
  });

it("rejects the pipe promise and read when the transform errors", async () => {
  const ts = new TransformStream({
    transform(_chunk: any, controller: any) {
      controller.error(new Error("piped boom"));
    },
  });

  const reader = streamOf(new Uint8Array([1, 2, 3])).pipeThrough(ts).getReader();
  await expect(reader.read()).rejects.toThrow("piped boom");
});

it("settles the writable side when the transform errors via controller.error", async () => {
  const ts: any = new TransformStream({
    transform(_chunk: any, controller: any) {
      controller.error(new Error("transform failed"));
    },
  });

  const reader = ts.readable.getReader();
  const read = reader.read().then(
    () => {},
    () => {}
  );
  const writer = ts.writable.getWriter();
  try {
    await expect(writer.write(new Uint8Array([1, 2, 3]))).resolves.toBeUndefined();
    await expect(writer.ready).rejects.toThrow("transform failed");
    await expect(writer.closed).rejects.toThrow("transform failed");
  } finally {
    await read.catch(() => {});
    reader.releaseLock();
    writer.releaseLock();
  }
});

// Known gap, tracked separately: when the transform throws instead of calling
// `controller.error()`, the readable side is never errored, so `reader.read()`
// hangs forever. Node rejects it with the thrown error. Enabled once that is
// fixed; the writable side already settles, covered by the case above.
it.skip("rejects the read when the transform throws", async () => {
  const ts: any = new TransformStream({
    transform() {
      throw new Error("sync boom");
    },
  });

  const reader = ts.readable.getReader();
  const writer = ts.writable.getWriter();
  writer.write(new Uint8Array([1])).catch(() => {});
  await expect(reader.read()).rejects.toThrow("sync boom");
});

it("keeps the normal path intact after an errored stream", async () => {
  const failing = new TransformStream({
    transform(_chunk: any, controller: any) {
      controller.error(new Error("first fails"));
    },
  });
  await expect(
    streamOf(new Uint8Array([1])).pipeThrough(failing).getReader().read()
  ).rejects.toThrow("first fails");

  const doubling = new TransformStream({
    transform(chunk: any, controller: any) {
      controller.enqueue(chunk);
      controller.enqueue(chunk);
    },
  });
  const reader = streamOf(new Uint8Array([7])).pipeThrough(doubling).getReader();
  const parts: Uint8Array[] = [];
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      parts.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  expect(parts.flatMap((p) => Array.from(p))).toEqual([7, 7]);
});
