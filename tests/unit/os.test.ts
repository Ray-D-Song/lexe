import defaultImport from "node:os";
import legacyImport from "os";

it("node:os should be the same as os", () => {
  expect(defaultImport).toStrictEqual(legacyImport);
});

it("exposes os.constants (signals, errno, dlopen, priority, UV_UDP_REUSEADDR)", () => {
  const { constants } = defaultImport;
  expect(constants).toBeDefined();
  expect(constants.UV_UDP_REUSEADDR).toBe(4);
  expect(constants.signals.SIGINT).toBeGreaterThan(0);
  expect(constants.signals.SIGTERM).toBeGreaterThan(0);
  expect(constants.errno.ENOENT).toBeGreaterThan(0);
  expect(constants.errno.EACCES).toBeGreaterThan(0);
  expect(constants.priority).toEqual({
    PRIORITY_LOW: 19,
    PRIORITY_BELOW_NORMAL: 10,
    PRIORITY_NORMAL: 0,
    PRIORITY_ABOVE_NORMAL: -7,
    PRIORITY_HIGH: -14,
    PRIORITY_HIGHEST: -20,
  });
  if (defaultImport.platform() !== "win32") {
    expect(typeof constants.dlopen.RTLD_LAZY).toBe("number");
    expect(typeof constants.dlopen.RTLD_NOW).toBe("number");
  }
});

it("reports real IPv6 scopeid values on link-local addresses", () => {
  if (defaultImport.platform() === "win32") return;
  const ifaces = defaultImport.networkInterfaces();
  const linkLocals: Array<{ iface: string; scopeid: number }> = [];
  for (const [name, list] of Object.entries(ifaces)) {
    for (const entry of list ?? []) {
      if (entry.family === "IPv6" && entry.address.startsWith("fe80:")) {
        linkLocals.push({ iface: name, scopeid: entry.scopeid! });
      }
    }
  }
  if (linkLocals.length > 0) {
    expect(linkLocals.some((e) => e.scopeid > 0)).toBe(true);
  }
});
