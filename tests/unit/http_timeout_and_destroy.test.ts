import { spawn } from "node:child_process";
import { join } from "node:path";

const TIMEOUT = 20_000;

// Fixtures are standalone .mjs files, so the timer/IO paths run exactly as
// they do for users. `-e` plus an IIFE changes the continuation draining and
// would not exercise the same code.
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

describe("server.timeout", () => {
  it("has a 0 default and setTimeout returns the server", async () => {
    const { code, stdout } = await runFixture(
      "http_server_timeout_surface.fixture.mjs"
    );
    expect(stdout).toContain("default=0");
    expect(stdout).toContain("returns-server=true");
    expect(stdout).toContain("after-set=1234");
    expect(code).toBe(0);
  }, TIMEOUT);

  it("closes an idle socket when no timeout listener is installed", async () => {
    const { code, stdout } = await runFixture(
      "http_server_timeout.fixture.mjs"
    );
    expect(stdout).toContain("connected");
    expect(stdout).toContain("server-closed-idle-socket");
    expect(code).toBe(0);
  }, TIMEOUT);

  it("emits the timeout event and leaves the socket to the listener", async () => {
    const { code, stdout } = await runFixture(
      "http_server_timeout_listener.fixture.mjs"
    );
    expect(stdout).toContain("timeout-fired hasSocket=true");
    expect(stdout).toContain("client-closed fired=1");
    expect(code).toBe(0);
  }, TIMEOUT);
});

describe("ClientRequest.destroy / abort", () => {
  it("emits error then close, and sets destroyed without aborted", async () => {
    const { code, stdout } = await runFixture(
      "http_client_request_destroy.fixture.mjs"
    );
    expect(stdout).toContain("destroy-events=error:ECONNRESET,close");
    expect(stdout).toContain("destroy-destroyed=true");
    expect(stdout).toContain("destroy-aborted=false");
    expect(code).toBe(0);
  }, TIMEOUT);

  it("emits abort before error and sets both flags", async () => {
    const { code, stdout } = await runFixture(
      "http_client_request_destroy.fixture.mjs"
    );
    expect(stdout).toContain("abort-events=abort,error:ECONNRESET,close");
    expect(stdout).toContain("abort-destroyed=true");
    expect(stdout).toContain("abort-aborted=true");
    expect(code).toBe(0);
  }, TIMEOUT);

  it("delivers the error passed to destroy() as the same object", async () => {
    const { code, stdout } = await runFixture(
      "http_client_request_destroy.fixture.mjs"
    );
    expect(stdout).toContain("destroy-err-same-object=true");
    expect(stdout).toContain("destroy-err-code=EMYCUSTOM");
    expect(code).toBe(0);
  }, TIMEOUT);
});
