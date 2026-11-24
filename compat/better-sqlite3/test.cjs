"use strict";

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const Database = require("better-sqlite3");

function assert(condition, message) {
  if (!condition) {
    console.error(message);
    process.exit(1);
  }
}

// In-memory database: prepare / run / get / all
const memory = new Database(":memory:");
memory.exec(
  "CREATE TABLE users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)"
);

const insert = memory.prepare("INSERT INTO users (name) VALUES (?)");
const insertInfo = insert.run("alice");
assert(insertInfo.changes === 1, "insert.run should report one change");

const selectById = memory.prepare("SELECT id, name FROM users WHERE id = ?");
const alice = selectById.get(1);
assert(alice && alice.name === "alice", "select.get should return inserted row");

const allUsers = memory.prepare("SELECT name FROM users").all();
assert(allUsers.length === 1 && allUsers[0].name === "alice", "select.all mismatch");

// Transactions: commit and rollback
const insertMany = memory.transaction((names) => {
  for (const name of names) {
    insert.run(name);
  }
});
insertMany(["bob", "carol"]);
assert(
  memory.prepare("SELECT COUNT(*) AS c FROM users").get().c === 3,
  "transaction commit should persist rows"
);

const countBeforeRollback = memory
  .prepare("SELECT COUNT(*) AS c FROM users")
  .get().c;
try {
  memory.transaction(() => {
    insert.run("should-not-persist");
    throw new Error("rollback");
  })();
} catch {
  // expected
}
assert(
  memory.prepare("SELECT COUNT(*) AS c FROM users").get().c ===
    countBeforeRollback,
  "transaction rollback should not persist rows"
);

memory.close();

// File-backed database with pragma
const dbPath = path.join(os.tmpdir(), `lexe-better-sqlite3-${process.pid}.db`);
if (fs.existsSync(dbPath)) {
  fs.unlinkSync(dbPath);
}

const fileDb = new Database(dbPath);
fileDb.pragma("journal_mode = WAL");
fileDb.exec("CREATE TABLE kv (k TEXT PRIMARY KEY, v TEXT NOT NULL)");
fileDb.prepare("INSERT INTO kv (k, v) VALUES (?, ?)").run("key", "value");
assert(
  fileDb.prepare("SELECT v FROM kv WHERE k = ?").get("key").v === "value",
  "file database read/write failed"
);
fileDb.close();
fs.unlinkSync(dbPath);

// Exit without explicit close: environment cleanup hooks should release native handles.
const implicit = new Database(":memory:");
implicit.exec("CREATE TABLE implicit_kv (k TEXT PRIMARY KEY, v TEXT NOT NULL)");
implicit.prepare("INSERT INTO implicit_kv (k, v) VALUES (?, ?)").run("k", "v");

console.log("better-sqlite3 compat OK");
