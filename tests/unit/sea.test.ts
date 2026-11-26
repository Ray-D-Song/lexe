import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import defaultSea, {
  getAsset,
  getAssetAsBlob,
  getAssetKeys,
  getRawAsset,
  isSea,
} from "node:sea";
import { spawnCapture } from "./test-utils";

const TIMEOUT = 30000;

describe("node:sea", () => {
  it("reports non-SEA mode and throws ERR_NOT_IN_SINGLE_EXECUTABLE_APPLICATION", () => {
    expect(isSea()).toBe(false);
    expect(defaultSea.isSea()).toBe(false);
    expect(() => getAssetKeys()).toThrow();
    try {
      getAssetKeys();
    } catch (e: any) {
      expect(e.code).toBe("ERR_NOT_IN_SINGLE_EXECUTABLE_APPLICATION");
    }
    try {
      getRawAsset("missing.txt");
    } catch (e: any) {
      expect(e.code).toBe("ERR_NOT_IN_SINGLE_EXECUTABLE_APPLICATION");
    }
  });

  it("cannot be imported via bare 'sea' specifier", async () => {
    const bare = ["s", "e", "a"].join("");
    await expect(import(bare as any)).rejects.toThrow();
  });

  it(
    "packages and executes SEA executables with --sea-config, --asset, --assets-dir, and passes all flags in process.argv",
    async () => {
      const root = await mkdtemp(join(tmpdir(), "lexe-test-sea-"));
      try {
        const entry = join(root, "app.mjs");
        const cfgAsset = join(root, "from-cfg.txt");
        const cliAsset = join(root, "from-cli.bin");
        const dirAssets = join(root, "bundle-assets");
        const nestedDir = join(dirAssets, "sub");
        await mkdir(nestedDir, { recursive: true });

        await writeFile(cfgAsset, "hello from config 🚀", "utf8");
        await writeFile(cliAsset, Buffer.from([1, 2, 3, 4]));
        await writeFile(join(nestedDir, "nested.json"), '{"ok":true}', "utf8");

        const seaConfigPath = join(root, "sea.json");
        await writeFile(
          seaConfigPath,
          JSON.stringify({
            main: "app.mjs",
            mainFormat: "module",
            disableExperimentalSEAWarning: true,
            assets: {
              "cfg.txt": "from-cfg.txt",
            },
          }),
          "utf8"
        );

        await writeFile(
          entry,
          `import { isSea, getAssetKeys, getRawAsset, getAsset, getAssetAsBlob } from "node:sea";
const keys = getAssetKeys();
const rawAb = getRawAsset("cfg.txt");
const utf8Text = getAsset("cfg.txt", "utf8");
const rawBin = Array.from(new Uint8Array(getRawAsset("extra/bin")));
const blob = getAssetAsBlob("sub/nested.json", { type: "application/json" });
const blobText = await blob.text();
let missingCode = "";
try {
  getRawAsset("does-not-exist");
} catch (err) {
  missingCode = err.code;
}
console.log(JSON.stringify({
  isSea: isSea(),
  argvTail: process.argv.slice(2),
  argv0Eq1: process.argv[0] === process.argv[1],
  keys,
  isAb: rawAb instanceof ArrayBuffer,
  utf8Text,
  rawBin,
  blobType: blob.type,
  blobText,
  missingCode,
}));
`,
          "utf8"
        );

        const outBin = join(
          root,
          process.platform === "win32" ? "sea-app.exe" : "sea-app"
        );
        const compileRes = await spawnCapture(process.argv0, [
          "compile",
          "--executable",
          "--sea-config",
          seaConfigPath,
          "--assets-dir",
          dirAssets,
          "--asset",
          `extra/bin=${cliAsset}`,
          "-o",
          outBin,
        ]);
        if (compileRes.code !== 0) {
          throw new Error(
            `Compile failed (${compileRes.code}): ${compileRes.stdout}${compileRes.stderr}`
          );
        }

        // Include flags like --no-warnings and --permission to verify packaged binary passes all flags to app
        const runRes = await spawnCapture(outBin, [
          "--no-warnings",
          "--permission",
          "--foo",
          "bar",
        ]);
        if (runRes.code !== 0) {
          throw new Error(
            `SEA run failed (${runRes.code}): ${runRes.stdout}${runRes.stderr}`
          );
        }

        const parsed = JSON.parse(runRes.stdout.trim());
        expect(parsed.isSea).toBe(true);
        expect(parsed.argv0Eq1).toBe(true);
        expect(parsed.argvTail).toEqual(["--no-warnings", "--permission", "--foo", "bar"]);
        expect(parsed.keys).toEqual(["cfg.txt", "sub/nested.json", "extra/bin"]);
        expect(parsed.isAb).toBe(true);
        expect(parsed.utf8Text).toBe("hello from config 🚀");
        expect(parsed.rawBin).toEqual([1, 2, 3, 4]);
        expect(parsed.blobType).toBe("application/json");
        expect(parsed.blobText).toBe('{"ok":true}');
        expect(parsed.missingCode).toBe("ERR_SINGLE_EXECUTABLE_APPLICATION_ASSET_NOT_FOUND");
      } finally {
        await rm(root, { recursive: true, force: true });
      }
    },
    TIMEOUT
  );
});
