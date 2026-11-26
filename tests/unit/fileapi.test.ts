import net from "node:net";
import { platform } from "node:os";

const IS_WINDOWS = platform() === "win32";

describe("FILEAPI: Blob and File", () => {
  it("validates Blob constructor parts and options", () => {
    expect(() => new (Blob as any)(123)).toThrow(TypeError);
    expect(() => new (Blob as any)(null)).toThrow(TypeError);
    expect(() => new (Blob as any)("abc")).toThrow(TypeError);
    expect(() => new (Blob as any)({ length: 1, 0: "a" })).toThrow(TypeError);
    expect(() => new (Blob as any)({ [Symbol.iterator]: null })).toThrow(TypeError);
    expect(() => new (Blob as any)([], "invalid")).toThrow(TypeError);
    expect(() => new (Blob as any)([], { endings: "bad" })).toThrow(TypeError);

    const empty = new Blob(undefined, undefined);
    expect(empty.size).toBe(0);
    expect(empty.type).toBe("");
  });

  it("handles USVString lone surrogates and native line endings", async () => {
    const surrogateBlob = new Blob(["a\uD800b"]);
    expect(await surrogateBlob.text()).toBe("a\uFFFDb");

    const nativeBlob = new Blob(["a\r\nb\rc\nd\r"], { endings: "native" });
    const eol = IS_WINDOWS ? "\r\n" : "\n";
    expect(await nativeBlob.text()).toBe(`a${eol}b${eol}c${eol}d${eol}`);
  });

  it("handles Blob.slice edge cases without panicking", async () => {
    const blob = new Blob(["0123456789"], { type: "Text/Plain" });
    expect(await blob.slice().text()).toBe("0123456789");
    expect(await blob.slice(3).text()).toBe("3456789");
    expect(await blob.slice(3, undefined).text()).toBe("3456789");
    expect(await blob.slice(5, 2).text()).toBe("");
    expect(await blob.slice(-3).text()).toBe("789");
    expect(await blob.slice(-100, 2).text()).toBe("01");
    expect(await blob.slice(0, 1e20).text()).toBe("0123456789");
    expect(blob.slice(0, 2, "Text/HTML").type).toBe("text/html");
    expect(blob.slice(0, 2, "bad\u001ftype").type).toBe("");
    expect(blob.slice(0, 2).type).toBe("");
  });

  it("strips UTF-8 BOM in Blob.text() and reads ArrayBufferView/DataView parts", async () => {
    const bom = new Blob([new Uint8Array([0xef, 0xbb, 0xbf, 0x68, 0x69])]);
    expect(await bom.text()).toBe("hi");

    const buf = new Uint8Array([1, 2, 3, 4, 5]).buffer;
    const view = new DataView(buf, 1, 3);
    const dvBlob = new Blob([view]);
    expect(Array.from(await dvBlob.bytes())).toEqual([2, 3, 4]);
  });

  it("supports Blob.stream() with default and BYOB readers and returns new object", async () => {
    const blob = new Blob(["hello world"]);
    const s1 = blob.stream();
    const s2 = blob.stream();
    expect(s1).not.toBe(s2);
    expect(s1).toBeInstanceOf(ReadableStream);

    const reader = s1.getReader();
    const chunks: Uint8Array[] = [];
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      chunks.push(value);
    }
    expect(Buffer.concat(chunks).toString()).toBe("hello world");

    const byobReader = s2.getReader({ mode: "byob" });
    const out = new Uint8Array(5);
    const r1 = await byobReader.read(out);
    expect(r1.done).toBe(false);
    expect(new TextDecoder().decode(r1.value)).toBe("hello");

    const emptyReader = new Blob([]).stream().getReader({ mode: "byob" });
    const rEmpty = await emptyReader.read(new Uint8Array(4));
    expect(rEmpty.done).toBe(true);
  });

  it("implements File inheriting from Blob with WebIDL validation", async () => {
    expect(() => new (File as any)()).toThrow(TypeError);
    expect(() => new (File as any)([])).toThrow(TypeError);
    expect(File.prototype instanceof Blob).toBe(true);

    const file = new File(["abc"], "dir/a\uD800.txt", {
      type: "Text/Plain",
      lastModified: 1234567.8,
    });
    expect(file instanceof Blob).toBe(true);
    expect(file instanceof File).toBe(true);
    expect(file.name).toBe("dir/a\uFFFD.txt");
    expect(file.type).toBe("text/plain");
    expect(file.lastModified).toBe(1234567);
    expect(file.slice(0, 2) instanceof File).toBe(false);
    expect(file.slice(0, 2) instanceof Blob).toBe(true);

    const reader = file.stream().getReader();
    const { value } = await reader.read();
    expect(new TextDecoder().decode(value)).toBe("abc");
  });
});

describe("FILEAPI: FileList and ProgressEvent", () => {
  it("exposes FileList constructor as illegal to construct directly", () => {
    expect(typeof FileList).toBe("function");
    expect(() => new (FileList as any)()).toThrow(TypeError);
    expect(FileList.prototype[Symbol.toStringTag]).toBe("FileList");
  });

  it("supports ProgressEvent constructor and Event inheritance", () => {
    const pe = new ProgressEvent("progress", {
      lengthComputable: true,
      loaded: 42,
      total: 100,
    });
    expect(pe instanceof Event).toBe(true);
    expect(pe instanceof ProgressEvent).toBe(true);
    expect(pe.type).toBe("progress");
    expect(pe.lengthComputable).toBe(true);
    expect(pe.loaded).toBe(42);
    expect(pe.total).toBe(100);
    expect(pe.bubbles).toBe(false);
    expect(pe.cancelable).toBe(false);

    const def = new ProgressEvent("load");
    expect(def.lengthComputable).toBe(false);
    expect(def.loaded).toBe(0);
    expect(def.total).toBe(0);

    expect(() => new ProgressEvent("load", { loaded: NaN })).toThrow(TypeError);
  });
});

describe("FILEAPI: FileReader and FileReaderSync", () => {
  it("exposes constants and EventTarget inheritance", () => {
    expect(FileReader.EMPTY).toBe(0);
    expect(FileReader.LOADING).toBe(1);
    expect(FileReader.DONE).toBe(2);
    expect(FileReader.prototype.EMPTY).toBe(0);
    expect(FileReader.prototype.LOADING).toBe(1);
    expect(FileReader.prototype.DONE).toBe(2);

    const reader = new FileReader();
    expect(reader instanceof EventTarget).toBe(true);
    expect(reader.readyState).toBe(FileReader.EMPTY);
    expect(reader.result).toBeNull();
    expect(reader.error).toBeNull();
  });

  it("reads ArrayBuffer, BinaryString, DataURL, and Text asynchronously", async () => {
    const bytes = new Uint8Array([0x00, 0x41, 0xff, 0xfe]);

    // readAsArrayBuffer
    const abResult = await new Promise<ArrayBuffer>((resolve, reject) => {
      const fr = new FileReader();
      const events: string[] = [];
      fr.onloadstart = (e) => {
        expect(e).toBeInstanceOf(ProgressEvent);
        expect(e.target).toBe(fr);
        events.push("loadstart");
      };
      fr.onprogress = () => events.push("progress");
      fr.onload = () => {
        events.push("load");
        expect(fr.readyState).toBe(FileReader.DONE);
      };
      fr.onloadend = () => {
        events.push("loadend");
        expect(events).toEqual(["loadstart", "progress", "load", "loadend"]);
        resolve(fr.result as ArrayBuffer);
      };
      fr.onerror = () => reject(fr.error);
      fr.readAsArrayBuffer(new Blob([bytes]));
      expect(fr.readyState).toBe(FileReader.LOADING);
      expect(() => fr.readAsText(new Blob(["x"]))).toThrow(DOMException);
    });
    expect(Array.from(new Uint8Array(abResult))).toEqual([0x00, 0x41, 0xff, 0xfe]);

    // readAsBinaryString
    const binResult = await new Promise<string>((resolve) => {
      const fr = new FileReader();
      fr.addEventListener("load", () => resolve(fr.result as string));
      fr.readAsBinaryString(new Blob([bytes]));
    });
    expect(binResult.length).toBe(4);
    expect(binResult.charCodeAt(0)).toBe(0x00);
    expect(binResult.charCodeAt(1)).toBe(0x41);
    expect(binResult.charCodeAt(2)).toBe(0xff);
    expect(binResult.charCodeAt(3)).toBe(0xfe);

    // readAsDataURL
    const dataUrl = await new Promise<string>((resolve) => {
      const fr = new FileReader();
      fr.onload = () => resolve(fr.result as string);
      fr.readAsDataURL(new Blob(["hello"], { type: "text/plain" }));
    });
    expect(dataUrl).toBe("data:text/plain;base64,aGVsbG8=");

    const emptyTypeUrl = await new Promise<string>((resolve) => {
      const fr = new FileReader();
      fr.onload = () => resolve(fr.result as string);
      fr.readAsDataURL(new Blob(["hi"]));
    });
    expect(emptyTypeUrl).toBe("data:application/octet-stream;base64,aGk=");

    // readAsText with BOM and encoding
    const textUtf16 = await new Promise<string>((resolve) => {
      const fr = new FileReader();
      fr.onload = () => resolve(fr.result as string);
      fr.readAsText(new Blob([new Uint8Array([0xff, 0xfe, 0x61, 0x00, 0x62, 0x00])]));
    });
    expect(textUtf16).toBe("ab");

    const textWin1252 = await new Promise<string>((resolve) => {
      const fr = new FileReader();
      fr.onload = () => resolve(fr.result as string);
      fr.readAsText(new Blob([new Uint8Array([0x80])]), "windows-1252");
    });
    expect(textWin1252).toBe("€");
  });

  it("aborts an in-flight read and emits abort + loadend", () => {
    const fr = new FileReader();
    const events: string[] = [];
    fr.onabort = () => events.push("abort");
    fr.onloadend = () => events.push("loadend");
    fr.onload = () => events.push("load");

    fr.readAsText(new Blob(["hello"]));
    fr.abort();
    expect(fr.readyState).toBe(FileReader.DONE);
    expect(fr.result).toBeNull();
    expect(fr.error).toBeInstanceOf(DOMException);
    expect(fr.error?.name).toBe("AbortError");
    expect(events).toEqual(["abort", "loadend"]);
  });

  it("supports synchronous FileReaderSync reads", () => {
    const sync = new FileReaderSync();
    const blob = new Blob(["abc"], { type: "text/plain" });
    expect(Array.from(new Uint8Array(sync.readAsArrayBuffer(blob)))).toEqual([97, 98, 99]);
    expect(sync.readAsBinaryString(new Blob([new Uint8Array([0x80, 0xff])]))).toBe("\u0080\u00ff");
    expect(sync.readAsDataURL(blob)).toBe("data:text/plain;base64,YWJj");
    expect(sync.readAsText(blob)).toBe("abc");
  });
});

describe("FILEAPI: fetch('blob:...')", () => {
  it("fetches GET/HEAD for valid blob URLs and rejects revoked or non-GET", async () => {
    const blob = new Blob(["blob payload"], { type: "Text/Plain" });
    const url = URL.createObjectURL(blob);
    try {
      const res = await fetch(url);
      expect(res.status).toBe(200);
      expect(res.headers.get("content-type")).toBe("text/plain");
      expect(res.headers.get("content-length")).toBe("12");
      expect(await res.text()).toBe("blob payload");

      const resFrag = await fetch(`${url}#section`);
      expect(resFrag.status).toBe(200);
      expect(await resFrag.text()).toBe("blob payload");

      const headRes = await fetch(url, { method: "HEAD" });
      expect(headRes.status).toBe(200);
      expect(headRes.headers.get("content-length")).toBe("12");
      expect(await headRes.text()).toBe("");

      let postFailed = false;
      try {
        await fetch(url, { method: "POST" });
      } catch (e) {
        postFailed = e instanceof TypeError;
      }
      expect(postFailed).toBe(true);
    } finally {
      URL.revokeObjectURL(url);
    }

    let revokedFailed = false;
    try {
      await fetch(url);
    } catch (e) {
      revokedFailed = e instanceof TypeError;
    }
    expect(revokedFailed).toBe(true);
  });
});

describe("FILEAPI: FormData file uploads", () => {
  it("supports append/set 3rd filename arg, USVString coercion, and first-entry get()", () => {
    const fd = new FormData();
    fd.append("k", 123 as any);
    fd.append("k", "second");
    expect(fd.get("k")).toBe("123");
    expect(fd.getAll("k")).toEqual(["123", "second"]);

    expect(() => fd.append("bad", "text", "name.txt")).toThrow(TypeError);

    const blob = new Blob(["abc"], { type: "text/plain" });
    fd.append("b1", blob);
    const b1 = fd.get("b1") as File;
    expect(b1).toBeInstanceOf(File);
    expect(b1.name).toBe("blob");

    fd.append("b2", blob, "custom.txt");
    expect((fd.get("b2") as File).name).toBe("custom.txt");

    const file = new File(["xyz"], "orig.txt", { type: "text/plain" });
    fd.set("b2", file, "renamed.txt");
    expect((fd.get("b2") as File).name).toBe("renamed.txt");
  });

  it("round-trips binary, empty, and special-character multipart File uploads", async () => {
    const fd = new FormData();
    const bin = new Uint8Array([0x00, 0x0d, 0x0a, 0xff, 0xfe, 0x1a]);
    fd.append("bin", new File([bin], 'a"b\nc\rd—é.bin', { type: "application/octet-stream" }));
    fd.append("empty", new File([], "empty.txt", { type: "text/plain" }));
    fd.append("text", "hello\nworld");

    const req = new Request("http://localhost/upload", { method: "POST", body: fd });
    const parsed = await req.formData();

    const outBin = parsed.get("bin") as File;
    expect(outBin).toBeInstanceOf(File);
    expect(outBin.name).toBe('a"b\nc\rd—é.bin');
    expect(Array.from(await outBin.bytes())).toEqual(Array.from(bin));

    const outEmpty = parsed.get("empty") as File;
    expect(outEmpty).toBeInstanceOf(File);
    expect(outEmpty.size).toBe(0);
    expect(outEmpty.name).toBe("empty.txt");

    expect(parsed.get("text")).toBe("hello\nworld");
  });

  it("posts FormData and File directly via fetch()", async () => {
    let receivedContentType = "";
    let receivedBody = Buffer.alloc(0);

    const server = net.createServer((socket) => {
      const chunks: Buffer[] = [];
      socket.on("data", (chunk) => {
        chunks.push(chunk);
        const buf = Buffer.concat(chunks);
        const headerEnd = buf.indexOf("\r\n\r\n");
        if (headerEnd === -1) return;
        const headersText = buf.slice(0, headerEnd).toString();
        const ctMatch = headersText.match(/content-type:\s*([^\r\n]+)/i);
        const clMatch = headersText.match(/content-length:\s*(\d+)/i);
        const cl = clMatch ? parseInt(clMatch[1], 10) : 0;
        if (buf.length < headerEnd + 4 + cl) return;
        receivedContentType = ctMatch ? ctMatch[1] : "";
        receivedBody = buf.slice(headerEnd + 4, headerEnd + 4 + cl);
        socket.write("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
        socket.end();
      });
    });

    await new Promise<void>((resolve) => server.listen(resolve));
    const port = (server.address() as net.AddressInfo).port;
    const host = IS_WINDOWS ? "localhost" : "127.0.0.1";

    try {
      const fd = new FormData();
      fd.append("file", new File(["payload"], "up.txt", { type: "text/plain" }));
      const res = await fetch(`http://${host}:${port}/upload`, {
        method: "POST",
        body: fd,
      });
      expect(await res.text()).toBe("ok");
      expect(receivedContentType.startsWith("multipart/form-data; boundary=")).toBe(true);

      const roundtrip = await new Response(receivedBody, {
        headers: { "content-type": receivedContentType },
      }).formData();
      const uploaded = roundtrip.get("file") as File;
      expect(uploaded.name).toBe("up.txt");
      expect(await uploaded.text()).toBe("payload");

      const fileRes = await fetch(`http://${host}:${port}/raw`, {
        method: "POST",
        body: new File(["rawfile"], "raw.txt", { type: "text/plain" }),
      });
      expect(await fileRes.text()).toBe("ok");
      expect(receivedContentType).toBe("text/plain");
      expect(receivedBody.toString()).toBe("rawfile");
    } finally {
      server.close();
    }
  });
});
