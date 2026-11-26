import defaultImport from "node:inspector";
import legacyImport from "inspector";
import Module from "node:module";

it("node:inspector should be the same as inspector", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
});

it("url() returns undefined and named/default share the same function", () => {
  expect(typeof defaultImport.url).toBe("function");
  expect(defaultImport.url()).toBeUndefined();
  expect(defaultImport.url).toBe(legacyImport.url);
  expect(defaultImport.default?.url ?? defaultImport.url).toBe(defaultImport.url);
});

it("exposes a Session shim that reports the unavailable Inspector protocol", async () => {
  expect(typeof defaultImport.Session).toBe("function");
  const session = new defaultImport.Session();
  session.connect();
  const error = await new Promise<any>((resolve) => session.post("Runtime.enable", resolve));
  expect(error.code).toBe("ERR_INSPECTOR_NOT_AVAILABLE");
  await expect(session.post("Runtime.enable")).rejects.toThrow("not available");
  session.disconnect();
  await expect(session.post("Runtime.enable")).rejects.toThrow("not connected");
  expect(defaultImport.open).toBeUndefined();
  expect(defaultImport.close).toBeUndefined();
  expect(defaultImport.waitForDebugger).toBeUndefined();
});

it("Module.builtinModules includes inspector", () => {
  expect(Module.builtinModules).toContain("inspector");
});
