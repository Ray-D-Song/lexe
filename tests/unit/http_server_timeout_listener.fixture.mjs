// Fixture: server.timeout 有 'timeout' 监听器时不应自动断开，由用户负责
import http from "node:http";
import net from "node:net";

const server = http.createServer((_req, _res) => {});
server.timeout = 50;

let fired = 0;
server.on("timeout", (socket) => {
  fired++;
  console.log("timeout-fired hasSocket=" + (socket != null));
  // 用户负责销毁
  socket.destroy();
});

await new Promise((r) => server.listen(0, "127.0.0.1", r));
const socket = net.connect(server.address().port, "127.0.0.1");
await new Promise((r) => socket.on("connect", r));
socket.write("GET / HTTP/1.1\r\nHost: x\r\nConnection: keep-alive\r\n\r\n");

await new Promise((r) => socket.on("close", r));
console.log("client-closed fired=" + fired);
try { socket.destroy(); } catch {}
server.close();
process.exit(0);
