import {
  parentPort,
  workerData,
  getEnvironmentData,
} from "node:worker_threads";

parentPort.on("message", (msg) => {
  if (msg?.type === "sab-add") {
    const sab = workerData?.sab;
    const view = new Int32Array(sab);
    Atomics.add(view, 0, msg.delta ?? 0);
    parentPort.postMessage({ sabOk: true, value: view[0] });
    return;
  }
  if (msg?.type === "clone-check") {
    const { when, tags } = msg.payload;
    parentPort.postMessage({
      ok: true,
      when: when instanceof Date ? when.toISOString() : String(when),
      mapSize: tags instanceof Map ? tags.size : -1,
    });
    return;
  }
  if (workerData?.askEnv) {
    const env = getEnvironmentData("lexe-worker-env-key");
    parentPort.postMessage({ envV: env?.v });
    return;
  }
  parentPort.postMessage({ pong: msg.ping, workerData });
});
