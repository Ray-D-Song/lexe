import { ReadableStream, TransformStream } from "node:stream/web";

// A transform that throws must error both sides of the stream. Returning the
// error only as a rejected promise from the write algorithm leaves the readable
// side in "readable", so `reader.read()` never settles and the process dies with
// a GC assertion at exit. Node rejects the read with the thrown error.
//
// The transform only runs once a chunk arrives, so each case writes. Values
// asserted here are what Node v24 produces for the same shape.

it("rejects the read when the transform throws", async () => {
  const ts: any = new TransformStream({
    transform() {
      throw new Error("sync boom");
    },
  });

  const reader = ts.readable.getReader();
  const writer = ts.writable.getWriter();
  const read = reader.read();
  const write = writer.write(new Uint8Array([1, 2, 3]));
  try {
    await expect(read).rejects.toThrow("sync boom");
    await expect(write).rejects.toThrow("sync boom");
  } finally {
    reader.releaseLock();
    writer.releaseLock();
  }
});

it("settles every promise on the errored stream when the transform throws", async () => {
  const ts: any = new TransformStream({
    transform() {
      throw new Error("sync boom");
    },
  });

  const reader = ts.readable.getReader();
  const writer = ts.writable.getWriter();
  const read = reader.read();
  const write = writer.write(new Uint8Array([1, 2, 3]));
  try {
    await expect(read).rejects.toThrow("sync boom");
    await expect(write).rejects.toThrow("sync boom");
    await expect(writer.ready).rejects.toThrow("sync boom");
    await expect(writer.closed).rejects.toThrow("sync boom");
  } finally {
    reader.releaseLock();
    writer.releaseLock();
  }
});

it("settles every promise when the transform errors and throws", async () => {
  const ts: any = new TransformStream({
    transform(_chunk: any, controller: any) {
      controller.error(new Error("both boom"));
      throw new Error("both boom");
    },
  });

  const reader = ts.readable.getReader();
  const writer = ts.writable.getWriter();
  const read = reader.read();
  const write = writer.write(new Uint8Array([1, 2, 3]));
  try {
    await expect(read).rejects.toThrow("both boom");
    await expect(write).rejects.toThrow("both boom");
    await expect(writer.ready).rejects.toThrow("both boom");
    await expect(writer.closed).rejects.toThrow("both boom");
  } finally {
    reader.releaseLock();
    writer.releaseLock();
  }
});

it("rejects the piped read when the transform throws", async () => {
  const ts = new TransformStream({
    transform() {
      throw new Error("piped boom");
    },
  });

  const rs: any = new ReadableStream({
    start(controller: any) {
      controller.enqueue(new Uint8Array([1, 2, 3]));
      controller.close();
    },
  });

  const reader = rs.pipeThrough(ts).getReader();
  await expect(reader.read()).rejects.toThrow("piped boom");
});
