// Fixture: close() 在在飞请求上等待（方案 2 与方案 1 的分界）。
// 只做一次 300ms 等待后立刻断言退出，避免依赖后续 timer 推进 ——
// Raster 在 close + keep-alive 场景下存在既有的 timer 停摆竞态
// （pristine 同样复现），修它不在本项范围内。
import http from "node:http";
import net from "node:net";

const server = http.createServer((_q, _res) => {
  // 永不响应：请求一直在飞
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));

const sock = net.connect(server.address().port, "127.0.0.1");
await new Promise((r) => sock.on("connect", r));
sock.write("GET / HTTP/1.1\r\nHost: x\r\nConnection: keep-alive\r\n\r\n");
await new Promise((r) => setTimeout(r, 100));

let fired = false;
server.close(() => {
  fired = true;
});

console.log("in-flight");

// 单次等待即可判定：若 close() 走了「立即杀掉」，回调会在这一刻之前就触发。
await new Promise((r) => setTimeout(r, 300));
console.log("at-300ms-fired=" + fired);

try { sock.destroy(); } catch {}
process.exit(0);
