#!/usr/bin/env node
/**
 * Structured parity probe for node:sqlite differential testing.
 * Prints a single JSON document to stdout.
 */
import fs from "node:fs";
import assert from "node:assert/strict";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { DatabaseSync, StatementSync, backup, constants } =
  await import("node:sqlite");

const tmpRoot =
  process.env.PARITY_TMP_ROOT ??
  fs.mkdtempSync(path.join(os.tmpdir(), "lexe-node-sqlite-"));
const lifecycleLoops = Number.parseInt(process.env.LIFECYCLE_LOOPS ?? "1", 10);
const backupLoops = Number.parseInt(process.env.BACKUP_LOOPS ?? "1", 10);
const extensionPath = process.env.SQLITE_EXTENSION_PATH ?? "";

const results = [];

async function record(name, fn) {
  try {
    const value = await fn();
    results.push({ name, ok: true, value });
  } catch (err) {
    results.push({
      name,
      ok: false,
      error: serializeError(err),
    });
  }
}

function serializeError(err) {
  if (!err || typeof err !== "object") {
    return { message: String(err) };
  }
  const out = {
    name: err.name,
    message: err.message,
    code: err.code,
  };
  if ("errcode" in err) out.errcode = err.errcode;
  if ("errstr" in err) out.errstr = err.errstr;
  return out;
}

function nextDb(name = "db") {
  return path.join(tmpRoot, `${name}-${results.length}.db`);
}

await record("module.require_node_sqlite", async () => {
  const mod = require("node:sqlite");
  return {
    hasDatabaseSync: typeof mod.DatabaseSync === "function",
    hasStatementSync: typeof mod.StatementSync === "function",
    hasBackup: typeof mod.backup === "function",
    changesetOmit: mod.constants.SQLITE_CHANGESET_OMIT,
    sessionExported: "Session" in mod,
  };
});

await record("module.require_sqlite_fails", async () => {
  try {
    require("sqlite");
    return { threw: false };
  } catch (err) {
    return {
      threw: true,
      code: err.code,
    };
  }
});

await record("module.isBuiltin", async () => {
  const Module = require("node:module").default ?? require("node:module");
  return {
    bare: Module.isBuiltin("sqlite"),
    nodeScheme: Module.isBuiltin("node:sqlite"),
    listed: Module.builtinModules.includes("node:sqlite"),
  };
});

await record("process.versions.sqlite", async () => ({
  version: process.versions.sqlite,
}));

await record("constants", async () => ({
  omit: constants.SQLITE_CHANGESET_OMIT,
  replace: constants.SQLITE_CHANGESET_REPLACE,
  abort: constants.SQLITE_CHANGESET_ABORT,
}));

await record("database.in_memory_exec_prepare", async () => {
  const db = new DatabaseSync(":memory:");
  try {
    db.exec(`
      CREATE TABLE data(key INTEGER PRIMARY KEY, val INTEGER) STRICT;
      INSERT INTO data (key, val) VALUES (1, 2);
    `);
    const stmt = db.prepare("SELECT * FROM data");
    return {
      get: stmt.get(),
      all: stmt.all(),
      statementType: stmt instanceof StatementSync,
    };
  } finally {
    db.close();
  }
});

await record("database.err_sqlite_error_metadata", async () => {
  const db = new DatabaseSync(":memory:");
  try {
    db.exec(`
      CREATE TABLE test(key INTEGER PRIMARY KEY) STRICT;
    `);
    const stmt = db.prepare("INSERT INTO test (key) VALUES (?)");
    stmt.run(1);
    try {
      stmt.run(1);
      return { threw: false };
    } catch (err) {
      return {
        threw: true,
        code: err.code,
        message: err.message,
        errcode: err.errcode,
        errstr: err.errstr,
      };
    }
  } finally {
    db.close();
  }
});

await record("database.named_parameters", async () => {
  const db = new DatabaseSync(":memory:");
  try {
    db.exec(
      "CREATE TABLE data(key INTEGER PRIMARY KEY, val INTEGER) STRICT;"
    );
    const stmt = db.prepare("INSERT INTO data (key, val) VALUES ($k, $v)");
    stmt.run({ k: 1, v: 9 });
    return db.prepare("SELECT * FROM data").get();
  } finally {
    db.close();
  }
});

await record("database.file_buffer_url", async () => {
  const dbPath = nextDb("file");
  const db = new DatabaseSync(dbPath);
  db.exec("CREATE TABLE data(key INTEGER PRIMARY KEY); INSERT INTO data (key) VALUES (1);");
  db.close();

  const fromBuffer = new DatabaseSync(Buffer.from(dbPath));
  const fromUrl = new DatabaseSync(pathToFileURL(dbPath));
  try {
    const location = fromUrl.location();
    assert.equal(path.basename(location), path.basename(dbPath));
    return {
      buffer: fromBuffer.prepare("SELECT * FROM data").all(),
      url: fromUrl.prepare("SELECT * FROM data").all(),
      location: {
        absolute: path.isAbsolute(location),
        basename: path.basename(location),
      },
    };
  } finally {
    fromBuffer.close();
    fromUrl.close();
  }
});

await record("database.udf_varargs", async () => {
  const db = new DatabaseSync(":memory:");
  try {
    db.function("args", { varargs: true }, (...args) => JSON.stringify(args));
    return db.prepare("SELECT args(1, 2) AS v").get();
  } finally {
    db.close();
  }
});

await record("database.aggregate_sum", async () => {
  const db = new DatabaseSync(":memory:");
  try {
    db.aggregate("agg", {
      start: 0,
      step: (acc, value) => acc + value,
    });
    return db
      .prepare(
        `SELECT agg(x) AS v FROM (SELECT 1 x UNION ALL SELECT 2 UNION ALL SELECT 3)`
      )
      .get();
  } finally {
    db.close();
  }
});

await record("database.udf", async () => {
  const db = new DatabaseSync(":memory:");
  try {
    db.function("twice", (x) => x * 2);
    return db.prepare("SELECT twice(21) AS v").get();
  } finally {
    db.close();
  }
});

await record("database.udf_text_nul", async () => {
  const db = new DatabaseSync(":memory:");
  try {
    db.function("text", () => "a\0b");
    const row = db.prepare("SELECT text() AS v").get();
    return { len: row.v.length, bytes: [...row.v].map((c) => c.charCodeAt(0)) };
  } finally {
    db.close();
  }
});

await record("database.udf_blob", async () => {
  const db = new DatabaseSync(":memory:");
  try {
    db.function("blob", () => new Uint8Array([0, 1, 2]));
    const row = db.prepare("SELECT blob() AS v").get();
    return Array.from(row.v);
  } finally {
    db.close();
  }
});

await record("database.aggregate_collect", async () => {
  const db = new DatabaseSync(":memory:");
  try {
    db.aggregate("collect", {
      start: () => [],
      step: (acc, value) => [...acc, value],
      result: (acc) => JSON.stringify(acc),
    });
    db.exec(`
      CREATE TABLE t(x INTEGER);
      INSERT INTO t(x) VALUES (1), (2), (3);
    `);
    return db.prepare("SELECT collect(x) AS v FROM t").get();
  } finally {
    db.close();
  }
});

await record("statement.source_and_expanded_sql", async () => {
  const db = new DatabaseSync(":memory:");
  try {
    const stmt = db.prepare("SELECT ? AS x");
    return {
      sourceSQL: stmt.sourceSQL,
      hasSourceSql: "sourceSql" in stmt,
      expandedBefore: stmt.expandedSQL,
      expandedAfter: (stmt.get(42), stmt.expandedSQL),
      hasExpandedSql: "expandedSql" in stmt,
    };
  } finally {
    db.close();
  }
});

await record("statement.columns_metadata", async () => {
  const db = new DatabaseSync(":memory:");
  try {
    db.exec("CREATE TABLE test (value INTEGER)");
    const stmt = db.prepare("SELECT value AS foo FROM test");
    return stmt.columns();
  } finally {
    db.close();
  }
});

await record("changeset.apply_basic", async () => {
  const create = () => {
    const db = new DatabaseSync(":memory:");
    db.exec(`
      CREATE TABLE data(key INTEGER PRIMARY KEY, value TEXT) STRICT;
    `);
    return db;
  };
  const from = create();
  const session = from.createSession();
  from.prepare("INSERT INTO data (key, value) VALUES (?, ?)").run(1, "hello");
  from.prepare("INSERT INTO data (key, value) VALUES (?, ?)").run(2, "world");
  const to = create();
  const applied = to.applyChangeset(session.changeset());
  return {
    applied,
    rows: to.prepare("SELECT * FROM data ORDER BY key").all(),
  };
});

function prepareConflict() {
  const database1 = new DatabaseSync(":memory:");
  const database2 = new DatabaseSync(":memory:");
  const sql = `CREATE TABLE data (
    key INTEGER PRIMARY KEY,
    value TEXT UNIQUE
  ) STRICT`;
  database1.exec(sql);
  database2.exec(sql);
  const insertSql = "INSERT INTO data (key, value) VALUES (?, ?)";
  const session = database1.createSession();
  database1.prepare(insertSql).run(1, "hello");
  database1.prepare(insertSql).run(2, "foo");
  database2.prepare(insertSql).run(1, "world");
  return { database2, changeset: session.changeset() };
}

await record("changeset.conflict_default_abort", async () => {
  const { database2, changeset } = prepareConflict();
  try {
    return {
      result: database2.applyChangeset(changeset),
      rows: database2.prepare("SELECT value FROM data").all(),
    };
  } finally {
    database2.close();
  }
});

await record("changeset.conflict_replace", async () => {
  const database1 = new DatabaseSync(":memory:");
  const database2 = new DatabaseSync(":memory:");
  const sql = `CREATE TABLE data (
    key INTEGER PRIMARY KEY,
    value TEXT UNIQUE
  ) STRICT`;
  database1.exec(sql);
  database2.exec(sql);
  database1.prepare("INSERT INTO data (key, value) VALUES (?, ?)").run(1, "hello");
  database2.prepare("INSERT INTO data (key, value) VALUES (?, ?)").run(1, "othervalue");
  const session = database1.createSession();
  database1.prepare("UPDATE data SET value = ? WHERE key = ?").run("foo", 1);
  const changeset = session.changeset();
  let conflictType = null;
  const result = database2.applyChangeset(changeset, {
    onConflict: (type) => {
      conflictType = type;
      return constants.SQLITE_CHANGESET_REPLACE;
    },
  });
  return {
    result,
    conflictType,
    rows: database2.prepare("SELECT value FROM data ORDER BY key").all(),
  };
});

await record("changeset.filter_truthy", async () => {
  const database1 = new DatabaseSync(":memory:");
  const database2 = new DatabaseSync(":memory:");
  database1.exec(`
    CREATE TABLE data1(key INTEGER PRIMARY KEY, value TEXT) STRICT;
    CREATE TABLE data2(key INTEGER PRIMARY KEY, value TEXT) STRICT;
  `);
  database2.exec(`
    CREATE TABLE data1(key INTEGER PRIMARY KEY, value TEXT) STRICT;
    CREATE TABLE data2(key INTEGER PRIMARY KEY, value TEXT) STRICT;
  `);
  const session = database1.createSession({ table: "data1" });
  database1.prepare("INSERT INTO data1 (key, value) VALUES (?, ?)").run(1, "a");
  database1.prepare("INSERT INTO data2 (key, value) VALUES (?, ?)").run(1, "b");
  const changeset = session.changeset();
  const result = database2.applyChangeset(changeset, {
    filter: (table) => table === "data1" || 1,
  });
  return {
    result,
    data1: database2.prepare("SELECT * FROM data1").all(),
    data2: database2.prepare("SELECT * FROM data2").all(),
  };
});

await record("changeset.filter_bigint_zero", async () => {
  const database1 = new DatabaseSync(":memory:");
  const database2 = new DatabaseSync(":memory:");
  database1.exec("CREATE TABLE data(key INTEGER PRIMARY KEY, value TEXT) STRICT;");
  database2.exec("CREATE TABLE data(key INTEGER PRIMARY KEY, value TEXT) STRICT;");
  const session = database1.createSession();
  database1.prepare("INSERT INTO data (key, value) VALUES (?, ?)").run(1, "a");
  database2.applyChangeset(session.changeset(), {
    filter: () => 0n,
  });
  return {
    count: database2.prepare("SELECT COUNT(*) AS c FROM data").get().c,
  };
});

await record("changeset.on_conflict_non_integer", async () => {
  const database1 = new DatabaseSync(":memory:");
  const database2 = new DatabaseSync(":memory:");
  const sql = `CREATE TABLE data (
    key INTEGER PRIMARY KEY,
    value TEXT UNIQUE
  ) STRICT`;
  database1.exec(sql);
  database2.exec(sql);
  database1.prepare("INSERT INTO data (key, value) VALUES (?, ?)").run(1, "hello");
  database2.prepare("INSERT INTO data (key, value) VALUES (?, ?)").run(1, "other");
  const session = database1.createSession();
  database1.prepare("UPDATE data SET value = ? WHERE key = ?").run("foo", 1);
  try {
    database2.applyChangeset(session.changeset(), {
      onConflict: () => 1.5,
    });
    return { threw: false };
  } catch (err) {
    return {
      threw: true,
      code: err.code,
      errcode: err.errcode,
    };
  }
});

await record("changeset.options_unrelated_getter", async () => {
  const database1 = new DatabaseSync(":memory:");
  const database2 = new DatabaseSync(":memory:");
  database1.exec("CREATE TABLE data(key INTEGER PRIMARY KEY, value TEXT) STRICT;");
  database2.exec("CREATE TABLE data(key INTEGER PRIMARY KEY, value TEXT) STRICT;");
  const session = database1.createSession();
  database1.prepare("INSERT INTO data (key, value) VALUES (?, ?)").run(1, "a");
  const result = database2.applyChangeset(session.changeset(), {
    get unrelated() {
      throw new Error("sentinel");
    },
  });
  return { result };
});

await record("createSession.invalid_options", async () => {
  const db = new DatabaseSync(":memory:");
  try {
    try {
      db.createSession(null);
      return { threw: false };
    } catch (err) {
      return { threw: true, code: err.code };
    }
  } finally {
    db.close();
  }
});

await record("module.cjs_own_keys", async () => {
  const mod = require("node:sqlite");
  return Object.keys(mod).sort();
});

await record("module.constants_descriptor", async () => {
  const desc = Object.getOwnPropertyDescriptor(
    constants,
    "SQLITE_CHANGESET_OMIT"
  );
  return {
    writable: desc?.writable,
    configurable: desc?.configurable,
    enumerable: desc?.enumerable,
  };
});

await record("database.session_and_iterator", async () => {
  const db = new DatabaseSync(":memory:");
  try {
    db.exec("CREATE TABLE t(key INTEGER PRIMARY KEY, val TEXT);");
    const session = db.createSession({ table: "t" });
    db.prepare("INSERT INTO t (key, val) VALUES (?, ?)").run(1, "a");
    const changeset = session.changeset();
    session.close();
    return {
      changesetBytes: changeset.byteLength,
      rowCount: db.prepare("SELECT COUNT(*) AS c FROM t").get().c,
    };
  } finally {
    db.close();
  }
});

// Extension differential coverage is tracked separately until Lexe
// loadExtension parity is complete.
if (
  extensionPath &&
  fs.existsSync(extensionPath) &&
  process.env.PARITY_INCLUDE_EXTENSION === "1"
) {
  await record("extension.load_and_query", async () => {
    const db = new DatabaseSync(":memory:", { allowExtension: true });
    try {
      db.loadExtension(extensionPath);
      db.exec("SELECT noop('hello');");
      return db.prepare("SELECT noop('world') AS result").get();
    } finally {
      db.close();
    }
  });

  await record("extension.disabled_by_default", async () => {
    const db = new DatabaseSync(":memory:", { allowExtension: false });
    try {
      try {
        db.loadExtension(extensionPath);
        return { threw: false };
      } catch (err) {
        return {
          threw: true,
          code: err.code,
          message: err.message,
        };
      }
    } finally {
      db.close();
    }
  });
}

await record("backup.basic", async () => {
  const sourcePath = nextDb("backup-source");
  const targetPath = nextDb("backup-target");
  const source = new DatabaseSync(sourcePath);
  source.exec(`
    CREATE TABLE data(key INTEGER PRIMARY KEY, value TEXT) STRICT;
    INSERT INTO data (key, value) VALUES (1, 'one');
  `);
  const pages = await backup(source, targetPath, { rate: 1 });
  source.close();
  const restored = new DatabaseSync(targetPath);
  const row = restored.prepare("SELECT * FROM data").get();
  restored.close();
  return { pages, row };
});

await record("backup.options_getter_propagates", async () => {
  const source = new DatabaseSync(":memory:");
  source.exec("CREATE TABLE t(x INTEGER);");
  try {
    await backup(source, nextDb("backup-getter-target"), {
      get rate() {
        throw new Error("getter-sentinel");
      },
    });
    return { threw: false };
  } catch (err) {
    return { threw: true, message: err.message };
  } finally {
    source.close();
  }
});

const lifecycle = [];
for (let i = 0; i < lifecycleLoops; i++) {
  const db = new DatabaseSync(":memory:");
  db.exec("CREATE TABLE t(x INTEGER);");
  db.function("id", (x) => x);
  db.prepare("INSERT INTO t(x) VALUES (?)").run(i);
  const session = db.createSession();
  const patch = session.patchset();
  session.close();
  const value = db.prepare("SELECT x FROM t").get();
  db.close();
  lifecycle.push({ i, value, patchBytes: patch.byteLength });
}
results.push({ name: "stability.lifecycle", ok: true, value: lifecycle });

const backupRuns = [];
for (let i = 0; i < backupLoops; i++) {
  const sourcePath = nextDb(`backup-loop-src-${i}`);
  const targetPath = nextDb(`backup-loop-dst-${i}`);
  const source = new DatabaseSync(sourcePath);
  source.exec("CREATE TABLE t(x INTEGER);");
  source.prepare("INSERT INTO t(x) VALUES (?)").run(i);
  const pages = await backup(source, targetPath);
  source.close();
  const target = new DatabaseSync(targetPath);
  const row = target.prepare("SELECT x FROM t").get();
  target.close();
  backupRuns.push({ i, pages, row });
}
results.push({ name: "stability.backup", ok: true, value: backupRuns });

const output = {
  parity: "node-sqlite",
  runtime: {
    pid: process.pid,
    execPath: process.execPath,
    platform: process.platform,
    arch: process.arch,
    tmpRoot,
    extensionPath: extensionPath || null,
    lifecycleLoops,
    backupLoops,
  },
  results,
};

process.stdout.write(`${JSON.stringify(output)}\n`);
