// Fixture: close() 之后新连接应被拒（ECONNREFUSED）
import http from "node:http";
import net from "node:net";

const s = http.createServer((_q, res) => res.end("ok"));
await new Promise((r) => s.listen(0, "127.0.0.1", r));
const port = s.address().port;

s.close();
await new Promise((r) => setTimeout(r, 150));

const outcome = await new Promise((resolve) => {
  const c = net.connect(port, "127.0.0.1");
  c.on("connect", () => { c.destroy(); resolve("CONNECTED"); });
  c.on("error", (e) => resolve("ERROR:" + (e.code || e.message)));
});

console.log("new-connection-after-close=" + (outcome.startsWith("ERROR:") ? "ERROR" : outcome));
process.exit(0);
