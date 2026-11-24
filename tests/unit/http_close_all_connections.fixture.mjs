// Fixture: close(cb) 之后立刻 closeAllConnections()，回调应立即触发。
// 用回调本身作为完成信号，不靠 setTimeout 轮询，避开 Raster 在
// close + keep-alive 场景下既有的 timer 停摆竞态（pristine 同样复现）。
import http from "node:http";
import net from "node:net";

const s = http.createServer((_q, _res) => { /* never responds */ });
await new Promise((r) => s.listen(0, "127.0.0.1", r));

const sock = net.connect(s.address().port, "127.0.0.1");
await new Promise((r) => sock.on("connect", r));
sock.write("GET / HTTP/1.1\r\nHost: x\r\nConnection: keep-alive\r\n\r\n");
// 让请求进入 handler；用 socket 收到数据的可能性为零（服务端不响应），
// 所以只等一个很短的 timer 让 handler 跑起来。
await new Promise((r) => setTimeout(r, 150));

let fired = false;
const closed = new Promise((resolve) => {
  s.close(() => {
    fired = true;
    resolve();
  });
});

// 回调此刻不应已触发（在飞请求还在）
console.log("before-close-all-fired=" + fired);

s.closeAllConnections();

// 只等回调，不设超时 timer：如果回调不来，进程会被外层测试的 SIGKILL 兜住，
// 而不是被我们自己的 timer 掩盖。
await closed;
console.log("after-close-all-fired=" + fired);

try { sock.destroy(); } catch {}
process.exit(0);
