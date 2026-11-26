describe("structuredClone", () => {
  it("Clones a simple object", () => {
    const originalObject = { foo: "bar", num: 42 };
    const clonedObject = structuredClone(originalObject);

    expect(clonedObject).toStrictEqual(originalObject);
    originalObject.foo += "extra";
    expect(clonedObject).not.toStrictEqual(originalObject);
  });

  it("Clones an array", () => {
    const originalArray = [1, 2, 3, 4, 5];
    const clonedArray = structuredClone(originalArray);
    expect(clonedArray).toStrictEqual(originalArray);
  });

  it("Clones an array of objects", () => {
    let obj = { foo: "bar" };
    const originalArray = [obj, obj, obj, obj, obj];
    const clonedArray = structuredClone(originalArray);
    expect(clonedArray).toStrictEqual(originalArray);
    expect(clonedArray[0]).not.toBe(originalArray[0]);
  });

  it("Clones nested objects", () => {
    const originalObject = { foo: { bar: { baz: "qux" } } };
    const clonedObject = structuredClone(originalObject);
    expect(clonedObject).toStrictEqual(originalObject);
  });

  it("Handles circular references", () => {
    const originalObject: any = { foo: { bar: "baz", arr: [1, 2, 3] } };
    originalObject.foo.circularRef = originalObject;
    originalObject.foo.circularRef2 = originalObject;
    originalObject.foo.circularRef3 = originalObject.foo;
    originalObject.ref2 = originalObject;
    const clonedObject = structuredClone(originalObject);
    expect(clonedObject).toStrictEqual(originalObject);
  });

  it("Clones a Map", () => {
    const originalMap = new Map([
      ["key1", "value1"],
      ["key2", "value2"],
    ]);
    const clonedMap = structuredClone(originalMap);
    expect(clonedMap).toStrictEqual(originalMap);
  });

  it("Clones a Set", () => {
    const originalSet = new Set([1, 2, 3, 4, 5]);
    const clonedSet = structuredClone(originalSet);
    expect(clonedSet).toStrictEqual(originalSet);
  });

  it("Clones a Date object", () => {
    const originalDate = new Date("2022-01-31T12:00:00Z");
    const clonedDate = structuredClone(originalDate);
    expect(clonedDate.getTime()).toEqual(originalDate.getTime());
  });

  it("Clones a Buffer", () => {
    const buffer = Buffer.alloc(11);
    buffer.write("hello world");
    const clonedBuffer = structuredClone(buffer);
    expect(clonedBuffer.buffer).toEqual(buffer.buffer);
    buffer.set([1, 2, 3, 4, 5, 6, 7, 8]);
    expect(clonedBuffer).not.toStrictEqual(buffer);
  });

  it("Handles transfer list", () => {
    const originalObject: any = { foo: { bar: "baz", arr: [1, 2, 3] } };
    const clonedObject1 = structuredClone(originalObject);

    expect(clonedObject1.foo.arr).not.toBe(originalObject.foo.arr);

    const clonedObject2 = structuredClone(originalObject, {
      transfer: [originalObject.foo.arr],
    });
    expect(clonedObject2.foo.arr).toEqual(originalObject.foo.arr);
  });

  it("Clones DOMException, Blob, and File", async () => {
    const err = new DOMException("bad state", "InvalidStateError");
    const clonedErr = structuredClone(err);
    expect(clonedErr).not.toBe(err);
    expect(clonedErr).toBeInstanceOf(DOMException);
    expect(clonedErr.name).toBe("InvalidStateError");
    expect(clonedErr.message).toBe("bad state");
    expect(clonedErr.code).toBe(11);

    const blob = new Blob(["hello blob"], { type: "text/plain" });
    const clonedBlob = structuredClone(blob);
    expect(clonedBlob).not.toBe(blob);
    expect(clonedBlob).toBeInstanceOf(Blob);
    expect(clonedBlob.size).toBe(10);
    expect(clonedBlob.type).toBe("text/plain");
    expect(await clonedBlob.text()).toBe("hello blob");

    const file = new File(["file body"], "doc.txt", {
      type: "text/markdown",
      lastModified: 1700000000000,
    });
    const clonedFile = structuredClone(file);
    expect(clonedFile).not.toBe(file);
    expect(clonedFile).toBeInstanceOf(File);
    expect(clonedFile).toBeInstanceOf(Blob);
    expect(clonedFile.name).toBe("doc.txt");
    expect(clonedFile.type).toBe("text/markdown");
    expect(clonedFile.lastModified).toBe(1700000000000);
    expect(await clonedFile.text()).toBe("file body");
  });

  it("Clones CryptoKey", async () => {
    const key = await crypto.subtle.generateKey(
      { name: "AES-GCM", length: 256 },
      true,
      ["encrypt", "decrypt"]
    );
    const clonedKey = structuredClone(key);
    expect(clonedKey).not.toBe(key);
    expect(clonedKey).toBeInstanceOf(CryptoKey);
    expect(clonedKey.type).toBe("secret");
    expect(clonedKey.extractable).toBe(true);
    expect(clonedKey.usages).toEqual(["encrypt", "decrypt"]);

    const iv = new Uint8Array(12);
    const pt = new TextEncoder().encode("secret message");
    const ct = await crypto.subtle.encrypt({ name: "AES-GCM", iv }, key, pt);
    const dec = await crypto.subtle.decrypt({ name: "AES-GCM", iv }, clonedKey, ct);
    expect(new TextDecoder().decode(dec)).toBe("secret message");
  });

  it("Transfers MessagePort and rejects non-transferred MessagePort", async () => {
    const { port1, port2 } = new MessageChannel();

    let nonTransferredErr: any;
    try {
      structuredClone(port1);
    } catch (e) {
      nonTransferredErr = e;
    }
    expect(nonTransferredErr).toBeInstanceOf(DOMException);
    expect(nonTransferredErr.name).toBe("DataCloneError");

    const moved = structuredClone({ port: port1 }, { transfer: [port1] });
    expect(moved.port).toBeInstanceOf(MessagePort);
    expect(moved.port).not.toBe(port1);

    // Original port1 was detached; postMessage on it does nothing.
    port1.postMessage("ignored");

    // Re-transferring an already detached port throws DataCloneError.
    let detachedErr: any;
    try {
      structuredClone(port1, { transfer: [port1] });
    } catch (e) {
      detachedErr = e;
    }
    expect(detachedErr).toBeInstanceOf(DOMException);
    expect(detachedErr.name).toBe("DataCloneError");

    const received = await new Promise<unknown>((resolve) => {
      moved.port.onmessage = (ev: any) => resolve(ev.data);
      port2.postMessage("ping-after-transfer");
    });
    expect(received).toBe("ping-after-transfer");

    const reply = await new Promise<unknown>((resolve) => {
      port2.once("message", resolve);
      moved.port.postMessage("pong-after-transfer");
    });
    expect(reply).toBe("pong-after-transfer");

    moved.port.close();
    port2.close();
  });

  it("Clones Blob, File, DOMException, CryptoKey, and transfers MessagePort across Worker", async () => {
    const { Worker } = await import("node:worker_threads");
    const { port1, port2 } = new MessageChannel();
    const key = await crypto.subtle.generateKey(
      { name: "HMAC", hash: "SHA-256" },
      false,
      ["sign", "verify"]
    );
    const file = new File(["worker-file"], "w.txt", { type: "text/plain" });
    const blob = new Blob(["worker-blob"], { type: "application/octet-stream" });
    const err = new DOMException("oops", "NotFoundError");

    const code = `
      import { parentPort, workerData } from "node:worker_threads";
      const sig = await crypto.subtle.sign("HMAC", workerData.key, new TextEncoder().encode("abc"));
      const fileText = await workerData.file.text();
      const blobText = await workerData.blob.text();
      workerData.port.on("message", (msg) => {
        workerData.port.postMessage({
          got: msg,
          sigLen: sig.byteLength,
          fileName: workerData.file.name,
          fileText,
          blobText,
          errName: workerData.err.name,
          isDom: workerData.err instanceof DOMException,
        });
      });
    `;
    const url = new URL(
      "data:text/javascript;base64," + Buffer.from(code).toString("base64")
    );
    const worker = new Worker(url, {
      workerData: { port: port1, key, file, blob, err },
      transferList: [port1],
    } as any);
    await new Promise<void>((resolve) => worker.once("online", resolve));

    const reply1 = await new Promise<any>((resolve) => {
      port2.on("message", resolve);
      port2.postMessage("hello-worker-data-port");
    });
    expect(reply1).toEqual({
      got: "hello-worker-data-port",
      sigLen: 32,
      fileName: "w.txt",
      fileText: "worker-file",
      blobText: "worker-blob",
      errName: "NotFoundError",
      isDom: true,
    });

    const { port1: p1, port2: p2 } = new MessageChannel();
    const w2Code = `
      import { parentPort, workerData } from "node:worker_threads";
      const fileText = await workerData.file.text();
      const blobText = await workerData.blob.text();
      const sig = await crypto.subtle.sign("HMAC", workerData.key, new TextEncoder().encode("abc"));
      parentPort.on("message", (msg) => {
        msg.port.on("message", (v) => {
          msg.port.postMessage({
            got: v,
            sigLen: sig.byteLength,
            fileText,
            blobText,
            errName: workerData.err.name,
            isDom: workerData.err instanceof DOMException,
          });
        });
      });
    `;
    await worker.terminate();
    port2.close();

    const w2 = new Worker(
      new URL("data:text/javascript;base64," + Buffer.from(w2Code).toString("base64")),
      { workerData: { key, file, blob, err } }
    );
    await new Promise<void>((resolve) => w2.once("online", resolve));
    w2.postMessage({ port: p1 }, [p1]);

    const reply = await new Promise<any>((resolve) => {
      p2.on("message", resolve);
      p2.postMessage("hello-worker-port");
    });

    expect(reply).toEqual({
      got: "hello-worker-port",
      sigLen: 32,
      fileText: "worker-file",
      blobText: "worker-blob",
      errName: "NotFoundError",
      isDom: true,
    });

    p2.close();
    await w2.terminate();
  });
});
