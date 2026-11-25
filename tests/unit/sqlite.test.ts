import Module, { createRequire } from "node:module";
import process, { versions } from "node:process";
import os from "node:os";
import path from "node:path";
import {
  DatabaseSync,
  StatementSync,
  constants,
  backup,
} from "node:sqlite";
import { spawnCapture } from "./test-utils";

const require = createRequire(import.meta.url);

function nullProto<T extends Record<string, unknown>>(value: T): T {
  return { __proto__: null, ...value };
}

describe("node:sqlite module access", () => {
  it("cannot be accessed without the node: scheme", () => {
    expect(() => require("sqlite")).toThrow(/Cannot find module 'sqlite'/);
    expect(Module.isBuiltin("sqlite")).toBe(false);
  });

  it("is available through node:sqlite", () => {
    expect(() => require("node:sqlite")).not.toThrow();
    expect(Module.isBuiltin("node:sqlite")).toBe(true);
    expect(Module.builtinModules).toContain("node:sqlite");
  });

  it("exports only Node 24.3 own keys from CJS require", () => {
    const cjs = require("node:sqlite");
    expect(Object.keys(cjs).sort()).toEqual(
      ["DatabaseSync", "StatementSync", "backup", "constants"].sort()
    );
    expect("SQLITE_VERSION" in cjs).toBe(false);
    expect("default" in cjs).toBe(false);
    expect("__esModule" in cjs).toBe(false);
    expect("Session" in cjs).toBe(false);
  });
});

describe("process.versions.sqlite", () => {
  it("reports SQLite 3.50.1", () => {
    expect(versions.sqlite).toBe("3.50.1");
  });
});

describe("sqlite constants", () => {
  it("exports non-writable changeset constants", () => {
    expect(constants.SQLITE_CHANGESET_OMIT).toBe(0);
    expect(constants.SQLITE_CHANGESET_REPLACE).toBe(1);
    expect(constants.SQLITE_CHANGESET_ABORT).toBe(2);
    const desc = Object.getOwnPropertyDescriptor(
      constants,
      "SQLITE_CHANGESET_OMIT"
    );
    expect(desc?.writable).toBe(false);
    expect(desc?.configurable).toBe(false);
    expect(desc?.enumerable).toBe(true);
  });
});

describe("DatabaseSync basics", () => {
  it("supports independent in-memory databases", () => {
    const db1 = new DatabaseSync(":memory:");
    const db2 = new DatabaseSync(":memory:");
    try {
      db1.exec(`
        CREATE TABLE data(key INTEGER PRIMARY KEY);
        INSERT INTO data (key) VALUES (1);
      `);
      db2.exec(`
        CREATE TABLE data(key INTEGER PRIMARY KEY);
        INSERT INTO data (key) VALUES (1);
      `);
      expect(db1.prepare("SELECT * FROM data").all()).toEqual([
        nullProto({ key: 1 }),
      ]);
      expect(db2.prepare("SELECT * FROM data").all()).toEqual([
        nullProto({ key: 1 }),
      ]);
    } finally {
      db1.close();
      db2.close();
    }
  });

  it("exec, prepare, get, all, and run work together", () => {
    const db = new DatabaseSync(":memory:");
    try {
      db.exec(`
        CREATE TABLE data(
          key INTEGER PRIMARY KEY,
          val INTEGER
        ) STRICT;
        INSERT INTO data (key, val) VALUES (1, 2);
        INSERT INTO data (key, val) VALUES (8, 9);
      `);
      const stmt = db.prepare("SELECT * FROM data ORDER BY key");
      expect(stmt.all()).toEqual([
        nullProto({ key: 1, val: 2 }),
        nullProto({ key: 8, val: 9 }),
      ]);
      expect(stmt.get()).toEqual(nullProto({ key: 1, val: 2 }));
      const insert = db.prepare("INSERT INTO data (key, val) VALUES (?, ?)");
      expect(insert.run(42, 43)).toEqual({
        changes: 1,
        lastInsertRowid: 42,
      });
    } finally {
      db.close();
    }
  });

  it("prepare returns a StatementSync instance", () => {
    const db = new DatabaseSync(":memory:");
    try {
      const stmt = db.prepare("CREATE TABLE webstorage(key TEXT)");
      expect(stmt).toBeInstanceOf(StatementSync);
    } finally {
      db.close();
    }
  });

  it("has sqlite-type symbol property", () => {
    const db = new DatabaseSync(":memory:");
    try {
      const sqliteTypeSymbol = Symbol.for("sqlite-type");
      expect(db[sqliteTypeSymbol as keyof DatabaseSync]).toBe("node:sqlite");
    } finally {
      db.close();
    }
  });
});

describe("scalar UDF", () => {
  it("passes varargs as separate arguments", () => {
    const db = new DatabaseSync(":memory:");
    try {
      db.function("args", { varargs: true }, (...args) => JSON.stringify(args));
      expect(db.prepare("SELECT args(1, 2) AS v").get()).toEqual(
        nullProto({ v: "[1,2]" })
      );
    } finally {
      db.close();
    }
  });

  it("truncates embedded NUL in text results like Node", () => {
    const db = new DatabaseSync(":memory:");
    try {
      db.function("text", () => "a\0b");
      const row = db.prepare("SELECT text() AS v").get() as { v: string };
      expect(row.v).toBe("a");
      expect(row.v.length).toBe(1);
    } finally {
      db.close();
    }
  });
});

describe("aggregate UDF", () => {
  it("sums values across rows", () => {
    const db = new DatabaseSync(":memory:");
    try {
      db.aggregate("agg", {
        start: 0,
        step: (acc, value) => acc + (value as number),
      });
      const row = db
        .prepare(
          `SELECT agg(x) AS v FROM (SELECT 1 x UNION ALL SELECT 2 UNION ALL SELECT 3)`
        )
        .get() as { v: number };
      expect(row.v).toBe(6);
    } finally {
      db.close();
    }
  });
});

describe("ERR_SQLITE_ERROR metadata", () => {
  it("includes code, message, errcode, and errstr", () => {
    const dbPath = path.join(
      os.tmpdir(),
      `lexe-sqlite-err-${process.pid}-${Date.now()}.db`
    );
    const db = new DatabaseSync(dbPath);
    try {
      db.exec(`
        CREATE TABLE test(
          key INTEGER PRIMARY KEY
        ) STRICT;
      `);
      const stmt = db.prepare("INSERT INTO test (key) VALUES (?)");
      expect(stmt.run(1)).toEqual({ changes: 1, lastInsertRowid: 1 });
      try {
        stmt.run(1);
        throw new Error("expected duplicate insert to throw");
      } catch (err: any) {
        expect(err.code).toBe("ERR_SQLITE_ERROR");
        expect(err.message).toBe("UNIQUE constraint failed: test.key");
        expect(err.errcode).toBe(1555);
        expect(err.errstr).toBe("constraint failed");
      }
    } finally {
      db.close();
    }
  });
});

describe("named parameters", () => {
  it("supports bare named parameters", () => {
    const db = new DatabaseSync(":memory:");
    try {
      db.exec(
        "CREATE TABLE data(key INTEGER PRIMARY KEY, val INTEGER) STRICT;"
      );
      const stmt = db.prepare("INSERT INTO data (key, val) VALUES ($k, $v)");
      stmt.run({ k: 1, v: 9 });
      expect(db.prepare("SELECT * FROM data").get()).toEqual(
        nullProto({ key: 1, val: 9 })
      );
    } finally {
      db.close();
    }
  });

  it("throws on unknown named parameters", () => {
    const db = new DatabaseSync(":memory:");
    try {
      db.exec(
        "CREATE TABLE types(key INTEGER PRIMARY KEY, val INTEGER) STRICT;"
      );
      const stmt = db.prepare("INSERT INTO types (key, val) VALUES ($k, $v)");
      expect(() => stmt.run({ $k: 1, $unknown: 1 })).toThrow(
        /Unknown named parameter '\$unknown'/
      );
    } finally {
      db.close();
    }
  });
});

describe("Statement metadata", () => {
  it("exposes sourceSQL and expandedSQL", () => {
    const db = new DatabaseSync(":memory:");
    try {
      const stmt = db.prepare("SELECT ? AS x");
      expect(stmt.sourceSQL).toBe("SELECT ? AS x");
      stmt.get(42);
      expect(stmt.expandedSQL).toContain("42");
      expect("sourceSql" in stmt).toBe(false);
      expect("expandedSql" in stmt).toBe(false);
    } finally {
      db.close();
    }
  });

  it("returns column origin metadata", () => {
    const db = new DatabaseSync(":memory:");
    try {
      db.exec("CREATE TABLE test (value INTEGER)");
      const stmt = db.prepare("SELECT value AS foo FROM test");
      expect(stmt.columns()).toEqual([
        nullProto({
          column: "value",
          database: "main",
          name: "foo",
          table: "test",
          type: "INTEGER",
        }),
      ]);
    } finally {
      db.close();
    }
  });
});

describe("changeset", () => {
  it("applies a basic changeset and returns true", () => {
    const createDb = () => {
      const db = new DatabaseSync(":memory:");
      db.exec(`
        CREATE TABLE data(key INTEGER PRIMARY KEY, value TEXT) STRICT;
      `);
      return db;
    };
    const from = createDb();
    const session = from.createSession();
    from.prepare("INSERT INTO data (key, value) VALUES (?, ?)").run(1, "hello");
    from.prepare("INSERT INTO data (key, value) VALUES (?, ?)").run(2, "world");
    const to = createDb();
    expect(to.applyChangeset(session.changeset())).toBe(true);
    expect(to.prepare("SELECT * FROM data ORDER BY key").all()).toEqual([
      nullProto({ key: 1, value: "hello" }),
      nullProto({ key: 2, value: "world" }),
    ]);
    from.close();
    to.close();
  });

  it("returns false when default conflict aborts", () => {
    const db1 = new DatabaseSync(":memory:");
    const db2 = new DatabaseSync(":memory:");
    const sql = `CREATE TABLE data (
      key INTEGER PRIMARY KEY,
      value TEXT UNIQUE
    ) STRICT`;
    db1.exec(sql);
    db2.exec(sql);
    const insert = "INSERT INTO data (key, value) VALUES (?, ?)";
    const session = db1.createSession();
    db1.prepare(insert).run(1, "hello");
    db1.prepare(insert).run(2, "foo");
    db2.prepare(insert).run(1, "world");
    expect(db2.applyChangeset(session.changeset())).toBe(false);
    expect(db2.prepare("SELECT value FROM data").all()).toEqual([
      nullProto({ value: "world" }),
    ]);
    db1.close();
    db2.close();
  });

  it("supports onConflict REPLACE and returns true", () => {
    const db1 = new DatabaseSync(":memory:");
    const db2 = new DatabaseSync(":memory:");
    const sql = `CREATE TABLE data (
      key INTEGER PRIMARY KEY,
      value TEXT UNIQUE
    ) STRICT`;
    db1.exec(sql);
    db2.exec(sql);
    db1.prepare("INSERT INTO data (key, value) VALUES (?, ?)").run(1, "hello");
    db2.prepare("INSERT INTO data (key, value) VALUES (?, ?)").run(1, "other");
    const session = db1.createSession();
    db1.prepare("UPDATE data SET value = ? WHERE key = ?").run("foo", 1);
    let conflictType: number | null = null;
    const result = db2.applyChangeset(session.changeset(), {
      onConflict: (type) => {
        conflictType = type;
        return constants.SQLITE_CHANGESET_REPLACE;
      },
    });
    expect(result).toBe(true);
    expect(conflictType).toBe(constants.SQLITE_CHANGESET_DATA);
    expect(db2.prepare("SELECT value FROM data").all()).toEqual([
      nullProto({ value: "foo" }),
    ]);
    db1.close();
    db2.close();
  });

  it("supports filter with truthy values", () => {
    const db1 = new DatabaseSync(":memory:");
    const db2 = new DatabaseSync(":memory:");
    db1.exec(`
      CREATE TABLE data1(key INTEGER PRIMARY KEY, value TEXT) STRICT;
      CREATE TABLE data2(key INTEGER PRIMARY KEY, value TEXT) STRICT;
    `);
    db2.exec(`
      CREATE TABLE data1(key INTEGER PRIMARY KEY, value TEXT) STRICT;
      CREATE TABLE data2(key INTEGER PRIMARY KEY, value TEXT) STRICT;
    `);
    const session = db1.createSession({ table: "data1" });
    db1.prepare("INSERT INTO data1 (key, value) VALUES (?, ?)").run(1, "a");
    db1.prepare("INSERT INTO data2 (key, value) VALUES (?, ?)").run(1, "b");
    expect(
      db2.applyChangeset(session.changeset(), {
        filter: (table) => table === "data1" || 1,
      })
    ).toBe(true);
    expect(db2.prepare("SELECT * FROM data1").all()).toEqual([
      nullProto({ key: 1, value: "a" }),
    ]);
    expect(db2.prepare("SELECT * FROM data2").all()).toEqual([]);
    db1.close();
    db2.close();
  });

  it("treats BigInt zero as falsy in filter", () => {
    const db1 = new DatabaseSync(":memory:");
    const db2 = new DatabaseSync(":memory:");
    db1.exec("CREATE TABLE data(key INTEGER PRIMARY KEY, value TEXT) STRICT;");
    db2.exec("CREATE TABLE data(key INTEGER PRIMARY KEY, value TEXT) STRICT;");
    const session = db1.createSession();
    db1.prepare("INSERT INTO data (key, value) VALUES (?, ?)").run(1, "a");
    db2.applyChangeset(session.changeset(), {
      filter: () => 0n,
    });
    expect(db2.prepare("SELECT COUNT(*) AS c FROM data").get()).toEqual(
      nullProto({ c: 0 })
    );
    db1.close();
    db2.close();
  });

  it("rejects non-integer onConflict return values", () => {
    const db1 = new DatabaseSync(":memory:");
    const db2 = new DatabaseSync(":memory:");
    const sql = `CREATE TABLE data (
      key INTEGER PRIMARY KEY,
      value TEXT UNIQUE
    ) STRICT`;
    db1.exec(sql);
    db2.exec(sql);
    db1.prepare("INSERT INTO data (key, value) VALUES (?, ?)").run(1, "hello");
    db2.prepare("INSERT INTO data (key, value) VALUES (?, ?)").run(1, "other");
    const session = db1.createSession();
    db1.prepare("UPDATE data SET value = ? WHERE key = ?").run("foo", 1);
    let err: any;
    try {
      db2.applyChangeset(session.changeset(), {
        onConflict: () => 1.5,
      });
    } catch (e: any) {
      err = e;
    }
    expect(err?.code).toBe("ERR_SQLITE_ERROR");
    expect(err?.errcode).toBe(21);
    db1.close();
    db2.close();
  });

  it("does not invoke unrelated getters in applyChangeset options", () => {
    const db1 = new DatabaseSync(":memory:");
    const db2 = new DatabaseSync(":memory:");
    db1.exec("CREATE TABLE data(key INTEGER PRIMARY KEY, value TEXT) STRICT;");
    db2.exec("CREATE TABLE data(key INTEGER PRIMARY KEY, value TEXT) STRICT;");
    const session = db1.createSession();
    db1.prepare("INSERT INTO data (key, value) VALUES (?, ?)").run(1, "a");
    expect(
      db2.applyChangeset(session.changeset(), {
        get unrelated() {
          throw new Error("sentinel");
        },
      })
    ).toBe(true);
    db1.close();
    db2.close();
  });
});

describe("createSession", () => {
  it("rejects null options", () => {
    const db = new DatabaseSync(":memory:");
    try {
      let code = "";
      try {
        db.createSession(null as any);
      } catch (err: any) {
        code = err.code;
      }
      expect(code).toBe("ERR_INVALID_ARG_TYPE");
    } finally {
      db.close();
    }
  });

  it("tracks only the requested table", () => {
    const db1 = new DatabaseSync(":memory:");
    const db2 = new DatabaseSync(":memory:");
    db1.exec(`
      CREATE TABLE data1(key INTEGER PRIMARY KEY, value TEXT) STRICT;
      CREATE TABLE data2(key INTEGER PRIMARY KEY, value TEXT) STRICT;
    `);
    db2.exec(`
      CREATE TABLE data1(key INTEGER PRIMARY KEY, value TEXT) STRICT;
      CREATE TABLE data2(key INTEGER PRIMARY KEY, value TEXT) STRICT;
    `);
    const session = db1.createSession({ table: "data1" });
    db1.prepare("INSERT INTO data1 (key, value) VALUES (?, ?)").run(1, "a");
    db1.prepare("INSERT INTO data2 (key, value) VALUES (?, ?)").run(1, "b");
    expect(db2.applyChangeset(session.changeset())).toBe(true);
    expect(db2.prepare("SELECT * FROM data1").all()).toEqual([
      nullProto({ key: 1, value: "a" }),
    ]);
    expect(db2.prepare("SELECT * FROM data2").all()).toEqual([]);
    db1.close();
    db2.close();
  });
});

describe("disabled sqlite mode", () => {
  it("rejects require of node:sqlite but keeps builtin listing", async () => {
    const { stdout } = await spawnCapture(process.argv[0], [
      "--no-experimental-sqlite",
      "-e",
      `
        const Module = require('node:module');
        let code = '';
        try {
          require('node:sqlite');
        } catch (err) {
          code = err.code;
        }
        console.log(JSON.stringify({
          code,
          isBuiltin: Module.isBuiltin('node:sqlite'),
          listed: Module.builtinModules.includes('node:sqlite'),
          version: process.versions.sqlite,
        }));
      `,
    ]);
    const result = JSON.parse(stdout.trim());
    expect(result.code).toBe("ERR_UNKNOWN_BUILTIN_MODULE");
    expect(result.isBuiltin).toBe(false);
    expect(result.listed).toBe(true);
    expect(result.version).toBe("3.50.1");
  });

  it("reports MODULE_NOT_FOUND for bare sqlite specifier", async () => {
    const { stdout } = await spawnCapture(process.argv[0], [
      "--no-experimental-sqlite",
      "-e",
      `
        let code = '';
        try {
          require('sqlite');
        } catch (err) {
          code = err.code;
        }
        console.log(code);
      `,
    ]);
    expect(stdout.trim()).toBe("MODULE_NOT_FOUND");
  });
});

describe("import styles", () => {
  it("supports CommonJS require of node:sqlite", () => {
    const cjs = require("node:sqlite");
    expect(typeof cjs.DatabaseSync).toBe("function");
    expect(cjs.constants.SQLITE_CHANGESET_OMIT).toBe(0);
    expect(typeof cjs.backup).toBe("function");
  });

  it("supports ESM import of node:sqlite", async () => {
    const mod = await import("node:sqlite");
    expect(typeof mod.DatabaseSync).toBe("function");
    expect(mod.constants.SQLITE_CHANGESET_OMIT).toBe(0);
    expect(typeof mod.backup).toBe("function");
    expect("Session" in mod).toBe(false);
    expect("SQLITE_VERSION" in mod).toBe(false);
    expect("default" in mod).toBe(true);
  });
});

describe("backup", () => {
  it("copies a database to a new file", async () => {
    const sourcePath = path.join(
      os.tmpdir(),
      `lexe-sqlite-src-${process.pid}-${Date.now()}.db`
    );
    const targetPath = path.join(
      os.tmpdir(),
      `lexe-sqlite-dst-${process.pid}-${Date.now()}.db`
    );
    const source = new DatabaseSync(sourcePath);
    source.exec(`
      CREATE TABLE data(key INTEGER PRIMARY KEY, value TEXT) STRICT;
      INSERT INTO data (key, value) VALUES (1, 'one');
    `);
    const pages = await backup(source, targetPath, { rate: 1 });
    source.close();
    expect(pages).toBeGreaterThan(0);
    const restored = new DatabaseSync(targetPath);
    expect(restored.prepare("SELECT * FROM data").get()).toEqual(
      nullProto({ key: 1, value: "one" })
    );
    restored.close();
  });

  it("propagates exceptions from options getters", async () => {
    const source = new DatabaseSync(":memory:");
    source.exec("CREATE TABLE t(x INTEGER);");
    const targetPath = path.join(
      os.tmpdir(),
      `lexe-sqlite-backup-getter-${process.pid}-${Date.now()}.db`
    );
    let message = "";
    try {
      await backup(source, targetPath, {
        get rate() {
          throw new Error("getter-sentinel");
        },
      });
    } catch (err: any) {
      message = err.message;
    } finally {
      source.close();
    }
    expect(message).toBe("getter-sentinel");
  });
});

describe("SQLTagStore and createTagStore", () => {
  it("executes tagged template literals safely with LRU eviction", () => {
    const db = new DatabaseSync(":memory:");
    try {
      db.exec("CREATE TABLE items(id INTEGER PRIMARY KEY, name TEXT) STRICT;");
      const store = db.createTagStore(2);
      expect(store.capacity).toBe(2);
      expect(store.size).toBe(0);
      expect(store.db).toBe(db);

      const SQLTagStoreCtor = Object.getPrototypeOf(store).constructor;
      let illegErr: any;
      try {
        new SQLTagStoreCtor();
      } catch (e) {
        illegErr = e;
      }
      expect(illegErr?.code).toBe("ERR_ILLEGAL_CONSTRUCTOR");

      const ins = store.run`INSERT INTO items (id, name) VALUES (${1}, ${"one'; DROP TABLE items; --"})`;
      expect(ins).toEqual({ changes: 1, lastInsertRowid: 1 });
      expect(store.size).toBe(1);

      store.run`INSERT INTO items (id, name) VALUES (${2}, ${"two"})`;
      expect(store.get`SELECT name FROM items WHERE id = ${1}`).toEqual(
        nullProto({ name: "one'; DROP TABLE items; --" })
      );
      expect(store.size).toBe(2);

      // Adding a 3rd distinct SQL evicts the LRU entry; total size remains capped at capacity
      expect(store.all`SELECT * FROM items ORDER BY id`).toEqual([
        nullProto({ id: 1, name: "one'; DROP TABLE items; --" }),
        nullProto({ id: 2, name: "two" }),
      ]);
      expect(store.size).toBe(2);

      const rows: unknown[] = [];
      for (const row of store.iterate`SELECT id FROM items ORDER BY id`) {
        rows.push(row);
      }
      expect(rows).toEqual([nullProto({ id: 1 }), nullProto({ id: 2 })]);

      // Interpolated objects are treated as positional values (never named params)
      expect(() => store.get`SELECT ${({}) as any}`).toThrow();

      store.clear();
      expect(store.size).toBe(0);
    } finally {
      db.close();
    }
  });
});

describe("setAuthorizer", () => {
  it("exposes authorizer constants and controls SQL compilation", () => {
    expect(constants.SQLITE_OK).toBe(0);
    expect(constants.SQLITE_DENY).toBe(1);
    expect(constants.SQLITE_IGNORE).toBe(2);
    expect(constants.SQLITE_SELECT).toBe(21);
    expect(constants.SQLITE_READ).toBe(20);
    const desc = Object.getOwnPropertyDescriptor(constants, "SQLITE_DENY");
    expect(desc?.writable).toBe(false);
    expect(desc?.configurable).toBe(false);
    expect(desc?.enumerable).toBe(true);

    const db = new DatabaseSync(":memory:");
    try {
      db.exec("CREATE TABLE secrets(id INTEGER PRIMARY KEY, pub TEXT, priv TEXT);");
      db.exec("INSERT INTO secrets VALUES (1, 'hello', 'topsecret');");

      const seen: Array<[number, string | null, string | null, string | null, string | null]> = [];
      db.setAuthorizer((action, a1, a2, a3, a4) => {
        seen.push([action, a1, a2, a3, a4]);
        if (action === constants.SQLITE_READ && a2 === "priv") {
          return constants.SQLITE_IGNORE;
        }
        if (action === constants.SQLITE_DELETE) {
          return constants.SQLITE_DENY;
        }
        return constants.SQLITE_OK;
      });

      const row = db.prepare("SELECT pub, priv FROM secrets").get();
      expect(row).toEqual(nullProto({ pub: "hello", priv: null }));
      expect(seen.some(([action, a1, a2, a3]) => action === constants.SQLITE_READ && a1 === "secrets" && a2 === "priv" && a3 === "main")).toBe(true);

      let denyErr: any;
      try {
        db.exec("DELETE FROM secrets");
      } catch (e) {
        denyErr = e;
      }
      expect(denyErr?.code).toBe("ERR_SQLITE_ERROR");

      // Clearing authorizer re-allows DELETE
      db.setAuthorizer(null);
      db.exec("DELETE FROM secrets");
      expect(db.prepare("SELECT COUNT(*) AS c FROM secrets").get()).toEqual(nullProto({ c: 0 }));

      // Re-entrancy into db methods from inside authorizer throws ERR_INVALID_STATE
      let innerErrCode = "";
      db.setAuthorizer(() => {
        try {
          db.exec("SELECT 1");
        } catch (e: any) {
          innerErrCode = e.code;
        }
        return constants.SQLITE_OK;
      });
      db.prepare("SELECT 1").get();
      expect(innerErrCode).toBe("ERR_INVALID_STATE");

      // Invalid return value from authorizer throws ERR_INVALID_RETURN_VALUE
      db.setAuthorizer(() => "bad" as any);
      let retErr: any;
      try {
        db.prepare("SELECT 1");
      } catch (e) {
        retErr = e;
      }
      expect(retErr?.code).toBe("ERR_INVALID_RETURN_VALUE");
    } finally {
      db.close();
    }
  });
});

describe("enableDefensive and limits", () => {
  it("enables defensive mode by default and allows toggling it", () => {
    const db = new DatabaseSync(":memory:");
    try {
      db.exec("CREATE TABLE t(x INTEGER);");
      db.exec("PRAGMA writable_schema = ON;");
      expect(() =>
        db.exec("UPDATE sqlite_schema SET name = 't2' WHERE name = 't'")
      ).toThrow();

      db.enableDefensive(false);
      db.exec("PRAGMA writable_schema = ON;");
      db.exec("UPDATE sqlite_schema SET name = 't2' WHERE name = 't'");
      db.exec("PRAGMA writable_schema = RESET;");
      db.enableDefensive(true);
      db.exec("PRAGMA writable_schema = ON;");
      expect(() =>
        db.exec("UPDATE sqlite_schema SET name = 't3' WHERE name = 't2'")
      ).toThrow();
    } finally {
      db.close();
    }

    const dbNonDef = new DatabaseSync(":memory:", { defensive: false });
    try {
      dbNonDef.exec("CREATE TABLE t(x INTEGER);");
      dbNonDef.exec("PRAGMA writable_schema = ON;");
      expect(() =>
        dbNonDef.exec("UPDATE sqlite_schema SET name = 't2' WHERE name = 't'")
      ).not.toThrow();
    } finally {
      dbNonDef.close();
    }
  });

  it("supports limits constructor option and dynamic limits property", () => {
    const db = new DatabaseSync(":memory:", { limits: { sqlLength: 100 } });
    try {
      expect(db.limits.sqlLength).toBe(100);
      expect(() => db.exec(`SELECT '${"a".repeat(200)}'`)).toThrow();

      db.limits.sqlLength = Infinity;
      expect(db.limits.sqlLength).toBeGreaterThan(1000);
      expect(db.prepare(`SELECT '${"a".repeat(200)}' AS s`).get()).toEqual(
        nullProto({ s: "a".repeat(200) })
      );

      for (const k of [
        "length",
        "sqlLength",
        "column",
        "exprDepth",
        "compoundSelect",
        "vdbeOp",
        "functionArg",
        "attach",
        "likePatternLength",
        "variableNumber",
        "triggerDepth",
      ]) {
        expect(typeof (db.limits as any)[k]).toBe("number");
      }

      db.limits.variableNumber = 2;
      expect(() => db.prepare("SELECT ?, ?, ?")).toThrow();

      expect(() => {
        db.limits.sqlLength = -1;
      }).toThrow(RangeError);
      expect(() => {
        (db.limits as any).sqlLength = "10";
      }).toThrow(TypeError);
    } finally {
      db.close();
    }
  });
});
