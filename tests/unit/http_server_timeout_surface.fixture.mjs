// Fixture: server.timeout 表面行为（默认值、setTimeout 返回值与副作用）
import http from "node:http";

const server = http.createServer((_q, res) => res.end("ok"));

console.log("default=" + server.timeout);

const returned = server.setTimeout(1234);
console.log("returns-server=" + (returned === server));
console.log("after-set=" + server.timeout);

server.setTimeout(0);
console.log("after-zero=" + server.timeout);

let listenerFired = false;
server.setTimeout(99, () => {
  listenerFired = true;
});
console.log("listener-count=" + server.listenerCount("timeout"));
console.log("listener-not-fired-sync=" + (listenerFired === false));

server.close();
