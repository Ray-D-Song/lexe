import zlib from "node:zlib";

// The compression glue waits for the zlib handle's `drain` event when it
// reports backpressure. If the handle errors instead, no `drain` will ever
// come. A waiter left pending forever keeps the transform - and the handle it
// closes over - reachable, so the worker dies with a GC assertion at exit.

const gzipBytes = (): Uint8Array =>
  new Uint8Array(
    zlib.gzipSync(new TextEncoder().encode(JSON.stringify({ hello: "world" })))
  );

/// Resolves to "settled" once `promise` settles, or "pending" after `ms`.
/// The timer is always cleared so a passing case leaves nothing behind.
const raceWithTimeout = (promise: Promise<any>, ms = 1000): Promise<string> => {
  let timer: any;
  const timeout = new Promise<string>((resolve) => {
    timer = setTimeout(() => resolve("pending"), ms);
  });
  return Promise.race([
    promise.then(
      () => "settled",
      () => "settled"
    ),
    timeout,
  ]).finally(() => clearTimeout(timer));
};

it("settles the write instead of hanging when the handle errors", async () => {
  // A real inflate handle fed gzip bytes errors; before the fix the write
  // stayed pending because the drain waiter was never settled.
  const { DecompressionStream } = await import("node:stream/web");
  const ds: any = new DecompressionStream("deflate");
  const writer = ds.writable.getWriter();
  const reader = ds.readable.getReader();
  const read = reader.read().then(
    () => "settled",
    () => "settled"
  );
  const write = writer.write(gzipBytes());

  try {
    expect(await raceWithTimeout(write)).toBe("settled");
    expect(await raceWithTimeout(read)).toBe("settled");
  } finally {
    reader.releaseLock();
    writer.releaseLock();
  }
});
