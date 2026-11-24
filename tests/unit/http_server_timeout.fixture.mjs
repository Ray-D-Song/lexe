// Fixture: server.timeout 无 'timeout' 监听器时应自动断开空闲 socket
import http from "node:http";
import net from "node:net";

const server = http.createServer((_req, _res) => {
  // 永不响应，让 server.timeout 生效
});
server.timeout = 100;
// 关键：不注册 'timeout' 监听器

await new Promise((r) => server.listen(0, "127.0.0.1", r));
const socket = net.connect(server.address().port, "127.0.0.1");
await new Promise((r) => socket.on("connect", r));
socket.write("GET / HTTP/1.1\r\nHost: x\r\nConnection: keep-alive\r\n\r\n");
console.log("connected");

// 等 socket 被服务端断开
await new Promise((r) => socket.on("close", r));
console.log("server-closed-idle-socket");
try { socket.destroy(); } catch {}
server.close();
// 既有缺陷：server.close() 回调不触发（PR #84 处理中），此处显式结束
process.exit(0);
