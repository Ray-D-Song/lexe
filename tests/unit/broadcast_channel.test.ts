import { BroadcastChannel as WtBroadcastChannel, Worker } from "node:worker_threads";

describe("BroadcastChannel", () => {
  it("matches between globalThis and node:worker_threads and inherits EventTarget", () => {
    expect(BroadcastChannel).toBe(WtBroadcastChannel);
    const ch = new BroadcastChannel("test-proto");
    expect(ch).toBeInstanceOf(EventTarget);
    expect(ch.name).toBe("test-proto");
    expect((ch as any)[Symbol.toStringTag]).toBe("BroadcastChannel");
    ch.close();
  });

  it("broadcasts MessageEvent to other channels with same name, excluding sender", async () => {
    const a = new BroadcastChannel("room-1");
    const b = new BroadcastChannel("room-1");
    const other = new BroadcastChannel("room-2");

    let aGot = false;
    let otherGot = false;
    a.onmessage = () => {
      aGot = true;
    };
    other.onmessage = () => {
      otherGot = true;
    };

    const ev = await new Promise<MessageEvent>((resolve) => {
      b.addEventListener("message", (e) => resolve(e as MessageEvent));
      a.postMessage({ hello: "world", list: [1, 2, 3] });
    });

    expect(ev).toBeInstanceOf(MessageEvent);
    expect(ev).toBeInstanceOf(Event);
    expect(ev.type).toBe("message");
    expect(ev.data).toEqual({ hello: "world", list: [1, 2, 3] });
    expect(aGot).toBe(false);
    expect(otherGot).toBe(false);

    a.close();
    b.close();
    other.close();
  });

  it("throws InvalidStateError when posting on closed channel and DataCloneError on functions", () => {
    const ch = new BroadcastChannel("room-err");
    let cloneErr: any;
    try {
      ch.postMessage(() => 1);
    } catch (e) {
      cloneErr = e;
    }
    expect(cloneErr).toBeInstanceOf(DOMException);
    expect(cloneErr.name).toBe("DataCloneError");

    ch.close();
    let closedErr: any;
    try {
      ch.postMessage("after-close");
    } catch (e) {
      closedErr = e;
    }
    expect(closedErr).toBeInstanceOf(DOMException);
    expect(closedErr.name).toBe("InvalidStateError");
  });

  it("delivers messages across Worker threads", async () => {
    const mainCh = new BroadcastChannel("cross-worker");
    const workerCode = `
      import { BroadcastChannel, parentPort } from "node:worker_threads";
      const ch = new BroadcastChannel("cross-worker");
      ch.onmessage = (ev) => {
        ch.postMessage({ echo: ev.data });
        ch.close();
      };
      parentPort.postMessage("ready");
    `;
    const worker = new Worker(
      new URL("data:text/javascript;base64," + Buffer.from(workerCode).toString("base64"))
    );
    await new Promise<void>((resolve) => {
      worker.once("message", (m) => {
        if (m === "ready") resolve();
      });
    });

    const got = await new Promise<unknown>((resolve) => {
      mainCh.onmessage = (ev) => resolve(ev.data);
      mainCh.postMessage("ping-bc");
    });

    expect(got).toEqual({ echo: "ping-bc" });
    mainCh.close();
    await worker.terminate();
  });
});
