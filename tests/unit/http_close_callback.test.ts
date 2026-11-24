import { spawn } from "node:child_process";
import { join } from "node:path";

const TIMEOUT = 20_000;

// Fixtures 是独立的 .mjs 文件，不用 -e：定时器与 IO 的驱动路径不同，
// -e 加 IIFE 跑不到同一段代码。
const runFixture = (
  name: string
): Promise<{ code: number; stdout: string; stderr: string }> =>
  new Promise((resolve, reject) => {
    let stdout = "";
    let stderr = "";
    const child = spawn(process.argv[0], [
      join(process.cwd(), "tests/unit", name),
    ]);
    const timer = setTimeout(() => child.kill("SIGKILL"), TIMEOUT - 2_000);
    child.stdout.on("data", (d) => (stdout += d.toString()));
    child.stderr.on("data", (d) => (stderr += d.toString()));
    child.on("close", (code) => {
      clearTimeout(timer);
      resolve({ code: code ?? -1, stdout, stderr });
    });
    child.on("error", (e) => {
      clearTimeout(timer);
      reject(e);
    });
  });

describe("server.close()", () => {
  it("returns the server, flips listening synchronously and nulls address()", async () => {
    const { code, stdout } = await runFixture(
      "http_close_semantics.fixture.mjs"
    );
    expect(stdout).toContain("return-is-server=true");
    expect(stdout).toContain("listening-before=true");
    expect(stdout).toContain("listening-after=false");
    expect(stdout).toContain("address-had-port=true");
    expect(stdout).toContain("address-after=null");
    expect(code).toBe(0);
  }, TIMEOUT);

  it("reports ERR_SERVER_NOT_RUNNING to a repeated close without throwing", async () => {
    const { code, stdout } = await runFixture(
      "http_close_semantics.fixture.mjs"
    );
    expect(stdout).toContain("second-close-threw=false");
    expect(stdout).toContain("first-cb=undefined");
    expect(stdout).toContain("second-cb=ERR_SERVER_NOT_RUNNING");
    expect(code).toBe(0);
  }, TIMEOUT);

  it("reports ERR_SERVER_NOT_RUNNING when closed before listening", async () => {
    const { code, stdout } = await runFixture(
      "http_close_semantics.fixture.mjs"
    );
    expect(stdout).toContain("close-before-listen-threw=false");
    expect(stdout).toContain("close-before-listen-cb=ERR_SERVER_NOT_RUNNING");
    expect(code).toBe(0);
  }, TIMEOUT);

  // 这条是方案 2 与方案 1 的分界：立即杀掉连接会让回调在 300ms 前就触发。
  it("waits for an in-flight request instead of killing the connection", async () => {
    const { code, stdout } = await runFixture(
      "http_close_waits_inflight.fixture.mjs"
    );
    expect(stdout).toContain("in-flight");
    // 300ms 时有在飞请求，回调必须仍未触发。这是与「立即杀掉」的分界点。
    expect(stdout).toContain("at-300ms-fired=false");
    expect(code).toBe(0);
  }, TIMEOUT);

  it("refuses new connections once closed", async () => {
    const { code, stdout } = await runFixture(
      "http_close_refuses_new.fixture.mjs"
    );
    expect(stdout).toContain("new-connection-after-close=ERROR");
    expect(code).toBe(0);
  }, TIMEOUT);

  it("releases a waiting close callback when closeAllConnections runs", async () => {
    const { code, stdout } = await runFixture(
      "http_close_all_connections.fixture.mjs"
    );
    expect(stdout).toContain("before-close-all-fired=false");
    expect(stdout).toContain("after-close-all-fired=true");
    expect(code).toBe(0);
  }, TIMEOUT);
});
