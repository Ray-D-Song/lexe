// Fixture: close() 的返回、listening、address、重复 close、未 listen 就 close
import http from "node:http";

const log = (...a) => console.log(...a);

// 返回 server 自身；listening 同步 false；address() 变 null
{
  const s = http.createServer((_q, res) => res.end("ok"));
  await new Promise((r) => s.listen(0, "127.0.0.1", r));
  const before = s.listening;
  const hadPort = s.address() && s.address().port != null;
  const ret = s.close();
  log("return-is-server=" + (ret === s));
  log("listening-before=" + before);
  log("listening-after=" + s.listening);
  log("address-had-port=" + hadPort);
  log("address-after=" + JSON.stringify(s.address()));
  await new Promise((r) => setTimeout(r, 100));
}

// 重复 close：第一次 undefined，第二次 ERR_SERVER_NOT_RUNNING，不抛
{
  const s = http.createServer((_q, res) => res.end("ok"));
  await new Promise((r) => s.listen(0, "127.0.0.1", r));
  let first = "NOT-CALLED";
  let second = "NOT-CALLED";
  let threw = false;
  s.close((e) => { first = e === undefined ? "undefined" : e.code; });
  try {
    s.close((e) => { second = e === undefined ? "undefined" : e.code; });
  } catch {
    threw = true;
  }
  log("second-close-threw=" + threw);
  await new Promise((r) => setTimeout(r, 250));
  log("first-cb=" + first);
  log("second-cb=" + second);
}

// 未 listen 就 close：不抛，回调收到 ERR_SERVER_NOT_RUNNING
{
  const s = http.createServer((_q, res) => res.end("ok"));
  let got = "NOT-CALLED";
  let threw = false;
  try {
    s.close((e) => { got = e === undefined ? "undefined" : e.code; });
  } catch {
    threw = true;
  }
  log("close-before-listen-threw=" + threw);
  await new Promise((r) => setTimeout(r, 200));
  log("close-before-listen-cb=" + got);
}
