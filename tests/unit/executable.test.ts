import {
  access,
  constants,
  mkdtemp,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { arch, platform, tmpdir } from "node:os";
import { join } from "node:path";
import { spawnCapture } from "./test-utils";

const TIMEOUT = 30000;
const IS_WINDOWS = platform() === "win32";

const currentLexePlatform = (): string => {
  const os = platform();
  const cpu = arch();
  if (os === "linux") return cpu === "arm64" ? "linux-arm64" : "linux-x64";
  if (os === "darwin") return cpu === "arm64" ? "darwin-arm64" : "darwin-x64";
  if (os === "win32") return cpu === "arm64" ? "windows-arm64" : "windows-x64";
  throw new Error(`Unsupported test platform: ${os}-${cpu}`);
};

const compile = async (
  filename: string,
  outputFilename: string,
  extraArgs: string[] = []
): Promise<void> => {
  const args = ["compile", filename, outputFilename, "--executable", ...extraArgs];
  const { code: compileResult, stdout, stderr } = await spawnCapture(
    process.argv0,
    args
  );
  if (compileResult !== 0) {
    throw new Error(
      `Compilation failed with exit code ${compileResult}: ${stdout}${stderr}`
    );
  }
};

const run = async (
  filename: string,
  args: string[] = []
): Promise<{ code: number; output: string }> => {
  const { code, stdout, stderr } = await spawnCapture(filename, args);
  return { code, output: stdout + stderr };
};

describe("executable compilation", () => {
  let tmpDir: string;
  let exePath: string;

  const createTestScript = async (content: string): Promise<string> => {
    const scriptPath = join(tmpDir, "exe_test.js");
    await writeFile(scriptPath, content, "utf8");
    return scriptPath;
  };

  beforeEach(async () => {
    tmpDir = await mkdtemp(join(tmpdir(), "lexe-test-compile"));
    exePath = join(tmpDir, "exe_test");
  });

  afterEach(async () => {
    await rm(tmpDir, { force: true, recursive: true });
  });

  it(
    "should compile JavaScript to a self-contained executable",
    async () => {
      const scriptPath = await createTestScript(
        "console.log('test'); process.exit(0);"
      );

      await compile(scriptPath, exePath);

      const stats = await stat(exePath);

      if (!stats.isFile()) {
        throw new Error("Created file is not a regular file");
      }
      if (stats.size <= 0) {
        throw new Error("Created file is empty");
      }

      if (!IS_WINDOWS) {
        await access(exePath, constants.X_OK);
      }
      const { code: execResult, output } = await run(exePath);
      if (output.trim() !== "test") {
        throw new Error(`Expected output 'test', got '${output}'`);
      }
      if (execResult !== 0) {
        throw new Error(`Expected exit code 0, got ${execResult}`);
      }
    },
    TIMEOUT
  );

  it(
    "should result in the correct exit code",
    async () => {
      const scriptPath = await createTestScript(
        "console.log('exiting'); process.exit(42);"
      );
      await compile(scriptPath, exePath);

      const { code: execResult, output } = await run(exePath);
      if (output.trim() !== "exiting") {
        throw new Error(`Expected output 'exiting', got '${output}'`);
      }
      if (execResult !== 42) {
        throw new Error(`Expected exit code 42, got ${execResult}`);
      }
    },
    TIMEOUT
  );

  it(
    "should result in the correct exit code passed via argument",
    async () => {
      const scriptPath = await createTestScript(
        "console.log('arg:', process.argv[1]); process.exit(process.argv[1]);"
      );
      await compile(scriptPath, exePath);

      const { code: execResult, output } = await run(exePath, ["1"]);
      if (output.trim() !== "arg: 1") {
        throw new Error(`Expected output 'arg: 1', got '${output}'`);
      }
      if (execResult !== 1) {
        throw new Error(`Expected exit code 1, got ${execResult}`);
      }
    },
    TIMEOUT
  );

  it(
    "should forward --version and --help to user script instead of lexe CLI",
    async () => {
      const scriptPath = await createTestScript(
        "console.log(JSON.stringify(process.argv.slice(1)));"
      );
      await compile(scriptPath, exePath);

      const { code: execResult, output } = await run(exePath, [
        "--version",
        "--help",
        "arg3",
      ]);
      if (execResult !== 0) {
        throw new Error(`Expected exit code 0, got ${execResult}: ${output}`);
      }
      if (output.trim() !== '["--version","--help","arg3"]') {
        throw new Error(`Unexpected argv output: '${output}'`);
      }
    },
    TIMEOUT
  );

  it(
    "should support explicit -p and -d flags for current platform",
    async () => {
      const targetPlatform = currentLexePlatform();
      const scriptPath = await createTestScript("console.log('platform-ok');");
      const { code: compileResult, stdout, stderr } = await spawnCapture(
        process.argv0,
        [
          "compile",
          scriptPath,
          "--executable",
          "-o",
          "custom",
          "-d",
          tmpDir,
          "-p",
          targetPlatform,
        ]
      );
      if (compileResult !== 0) {
        throw new Error(
          `Compilation failed (${compileResult}): ${stdout}${stderr}`
        );
      }

      const outName = IS_WINDOWS
        ? `custom-${targetPlatform}.exe`
        : `custom-${targetPlatform}`;
      const packagedPath = join(tmpDir, outName);
      const { code: execResult, output } = await run(packagedPath);
      if (execResult !== 0 || output.trim() !== "platform-ok") {
        throw new Error(`Unexpected run result (${execResult}): ${output}`);
      }
    },
    TIMEOUT
  );

  it(
    "should fail cleanly on invalid platform",
    async () => {
      const scriptPath = await createTestScript("console.log('nope');");
      const { code, stdout, stderr } = await spawnCapture(process.argv0, [
        "compile",
        scriptPath,
        "--executable",
        "-p",
        "invalid-platform",
        "-d",
        tmpDir,
      ]);
      if (code === 0) {
        throw new Error("Expected non-zero exit code for invalid platform");
      }
      const combined = stdout + stderr;
      if (!combined.includes("Unknown platform")) {
        throw new Error(`Expected 'Unknown platform' in error, got: ${combined}`);
      }
    },
    TIMEOUT
  );

  it(
    "should log output from the executable",
    async () => {
      const testOutput = "test content";
      const scriptPath = await createTestScript(`
        console.log('${testOutput}');
      `);

      await compile(scriptPath, exePath);

      const { code: execResult, output } = await run(exePath);
      if (execResult !== 0) {
        throw new Error(`Expected exit code 0, got ${execResult}`);
      }

      if (output.trim() !== testOutput) {
        throw new Error(
          `Expected output to be '${testOutput}', but got '${output}'`
        );
      }
    },
    TIMEOUT
  );
});
