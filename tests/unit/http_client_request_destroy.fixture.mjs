// Fixture: ClientRequest.destroy() / abort() 的事件序与状态字段
import http from "node:http";
import net from "node:net";

const server = net.createServer((sock) => {
  sock.on("data", () => {});
  sock.on("error", () => {});
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const port = server.address().port;

// destroy()
await new Promise((resolve) => {
  const req = http.request({ host: "127.0.0.1", port, path: "/" });
  const events = [];
  req.on("error", (e) => events.push("error:" + e.code));
  req.on("close", () => {
    events.push("close");
    console.log("destroy-events=" + events.join(","));
    console.log("destroy-destroyed=" + req.destroyed);
    console.log("destroy-aborted=" + req.aborted);
    resolve();
  });
  req.end();
  setTimeout(() => req.destroy(), 30);
});

// abort()
await new Promise((resolve) => {
  const req = http.request({ host: "127.0.0.1", port, path: "/" });
  const events = [];
  req.on("abort", () => events.push("abort"));
  req.on("error", (e) => events.push("error:" + e.code));
  req.on("close", () => {
    events.push("close");
    console.log("abort-events=" + events.join(","));
    console.log("abort-destroyed=" + req.destroyed);
    console.log("abort-aborted=" + req.aborted);
    resolve();
  });
  req.end();
  setTimeout(() => req.abort(), 30);
});

// destroy(err) 透传同一 error 对象
await new Promise((resolve) => {
  const req = http.request({ host: "127.0.0.1", port, path: "/" });
  const custom = new Error("custom");
  custom.code = "EMYCUSTOM";
  let got = null;
  req.on("error", (e) => { got = e; });
  req.on("close", () => {
    console.log("destroy-err-same-object=" + (got === custom));
    console.log("destroy-err-code=" + (got && got.code));
    resolve();
  });
  req.end();
  setTimeout(() => req.destroy(custom), 30);
});

server.close();
