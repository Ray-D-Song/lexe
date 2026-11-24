import http from "node:http";
import net from "node:net";
import { startFixture, withTimeout } from "./ws-test-helpers";

// `http.WebSocket` is the Node 22+ WebSocket client.
//
// These cases used to run each client in a child process, because a listener or
// a detached child left in the test worker made the worker exit non-zero at
// teardown. That gate was a harness limitation, not a requirement of these
// tests, and it is gone: the worker exits as soon as it has reported. Each case
// now runs here and stops its fixture in a `finally`.

it("exposes the WebSocket class and readyState constants", async () => {
  const fixture = await startFixture("echo");
  try {
    const out = await withTimeout<string>((settle) => {
      const ws: any = new http.WebSocket(`ws://127.0.0.1:${fixture.port}/`);
      const description = [
        typeof (http as any).WebSocket === "function" ? "class" : "missing",
        ws.CONNECTING,
        ws.OPEN,
        ws.CLOSING,
        ws.CLOSED,
        ws.readyState,
        JSON.stringify(ws.binaryType),
        ws.url,
      ].join(" ");
      ws.onopen = () => settle(description);
      ws.onerror = () => settle(description);
      setTimeout(() => settle(description), 3000);
    });
    // `url` carries the fixture's ephemeral port, so only the prefix is fixed.
    expect(out.startsWith('class 0 1 2 3 0 "blob" ws://127.0.0.1:')).toBe(true);
  } finally {
    fixture.stop();
  }
}, 20000);

it("round-trips text through open/message/close", async () => {
  const fixture = await startFixture("echo");
  try {
    const out = await withTimeout<string>((settle) => {
      const ws: any = new http.WebSocket(`ws://127.0.0.1:${fixture.port}/`);
      const seen: string[] = [];
      ws.onopen = () => {
        seen.push("open:" + ws.readyState);
        ws.send("hello");
      };
      ws.onmessage = (ev: any) => {
        seen.push("message:" + ev.data);
        ws.close(1000, "done");
      };
      ws.onclose = (ev: any) =>
        settle(
          seen
            .concat(
              "close:" +
                ev.code +
                ":" +
                ev.reason +
                ":" +
                ev.wasClean +
                ":" +
                ws.readyState
            )
            .join("|")
        );
      ws.onerror = (ev: any) => settle("ERROR:" + (ev && ev.message));
    });
    expect(out).toBe("open:1|message:hello|close:1000:done:true:3");
  } finally {
    fixture.stop();
  }
}, 20000);

it("round-trips binary as Buffer and as ArrayBuffer", async () => {
  const fixture = await startFixture("echo");
  try {
    const out = await withTimeout<string>((settle) => {
      const ws: any = new http.WebSocket(`ws://127.0.0.1:${fixture.port}/`);
      const seen: string[] = [];
      ws.onopen = () => ws.send(Buffer.from([1, 2, 3, 4, 5]));
      ws.onmessage = (ev: any) => {
        if (seen.length === 0) {
          seen.push(
            "buffer:" +
              (Buffer.isBuffer(ev.data)
                ? ev.data.toString("hex")
                : "not-buffer")
          );
          ws.binaryType = "arraybuffer";
          ws.send(new Uint8Array([9, 8, 7]).buffer);
          return;
        }
        seen.push(
          "arraybuffer:" +
            (ev.data instanceof ArrayBuffer
              ? Array.from(new Uint8Array(ev.data)).join(",")
              : "not-arraybuffer")
        );
        ws.close();
      };
      ws.onclose = () => settle(seen.join("|"));
      ws.onerror = (ev: any) => settle("ERROR:" + (ev && ev.message));
    });
    expect(out).toBe("buffer:0102030405|arraybuffer:9,8,7");
  } finally {
    fixture.stop();
  }
}, 20000);

it("round-trips a message larger than 64KB", async () => {
  const fixture = await startFixture("echo");
  try {
    const out = await withTimeout<string>((settle) => {
      const ws: any = new http.WebSocket(`ws://127.0.0.1:${fixture.port}/`);
      const payload = "x".repeat(200 * 1024);
      ws.onopen = () => ws.send(payload);
      ws.onmessage = (ev: any) => {
        const ok =
          typeof ev.data === "string" &&
          ev.data.length === payload.length &&
          ev.data === payload;
        settle("len:" + (ev.data && ev.data.length) + ":match:" + ok);
      };
      ws.onerror = (ev: any) => settle("ERROR:" + (ev && ev.message));
    }, 15000);
    expect(out).toBe("len:204800:match:true");
  } finally {
    fixture.stop();
  }
}, 30000);

it("reassembles fragmented messages and answers ping", async () => {
  // The `fragmented` fixture sends "he" + "ll" + "o" as three frames plus a ping.
  const fixture = await startFixture("fragmented");
  try {
    const out = await withTimeout<string>((settle) => {
      const ws: any = new http.WebSocket(`ws://127.0.0.1:${fixture.port}/`);
      let messages = 0;
      ws.onmessage = (ev: any) => {
        messages++;
        settle("message:" + JSON.stringify(ev.data) + ":" + messages);
      };
      ws.onclose = () => settle("CLOSED_BEFORE_MESSAGE");
      ws.onerror = (ev: any) => settle("ERROR:" + (ev && ev.message));
    });
    // One message, not three: continuations belong to a single logical message.
    expect(out).toBe('message:"hello":1');
  } finally {
    fixture.stop();
  }
}, 20000);

it("reports a bad Sec-WebSocket-Accept instead of accepting it", async () => {
  const server: any = net.createServer((sock: any) => {
    let raw = "";
    sock.on("data", (d: any) => {
      raw += d.toString();
      if (!raw.includes("\r\n\r\n")) return;
      // 101 with the right headers but a token that cannot match our key.
      sock.write(
        "HTTP/1.1 101 Switching Protocols\r\n" +
          "Upgrade: websocket\r\n" +
          "Connection: Upgrade\r\n" +
          "Sec-WebSocket-Accept: AAAAAAAAAAAAAAAAAAAAAAAAAAA=\r\n\r\n"
      );
    });
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  try {
    const out = await withTimeout<string>((settle) => {
      const ws: any = new http.WebSocket(`ws://127.0.0.1:${port}/`);
      const seen: string[] = [];
      ws.onopen = () => settle("UNEXPECTED_OPEN");
      ws.onerror = (ev: any) => seen.push("error:" + (ev && ev.message));
      ws.onclose = (ev: any) => {
        seen.push("close:" + ev.code);
        settle(seen.join("|"));
      };
    });
    expect(out).toContain("invalid Sec-WebSocket-Accept");
    expect(out).toContain("close:1006");
  } finally {
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
}, 20000);

it("reports a refused connection", async () => {
  const out = await withTimeout<string>((settle) => {
    const ws: any = new http.WebSocket("ws://127.0.0.1:1/");
    const seen: string[] = [];
    ws.onopen = () => settle("UNEXPECTED_OPEN");
    ws.onerror = (ev: any) =>
      seen.push("error:" + (ev && ev.message ? "has-message" : "no-message"));
    ws.onclose = (ev: any) => {
      seen.push("close:" + ev.code + ":" + ws.readyState);
      settle(seen.join("|"));
    };
  });
  expect(out).toContain("error:has-message");
  expect(out).toContain("close:1006:3");
}, 20000);

it("rejects a non-websocket scheme", async () => {
  let message = "";
  let threw = false;
  try {
    new (http as any).WebSocket("http://127.0.0.1:1/");
  } catch (err: any) {
    threw = true;
    message = String(err && err.message);
  }
  expect(`${threw}:${message.includes("ws:")}`).toBe("true:true");
}, 20000);
