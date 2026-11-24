"use strict";

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

function getConfig() {
  return {
    host: process.env.MYSQL_HOST || "127.0.0.1",
    port: Number(process.env.MYSQL_PORT || 3306),
    database: process.env.MYSQL_DATABASE || "lexe_compat",
    user: process.env.MYSQL_USER || "lexe",
    password: process.env.MYSQL_PASSWORD || "lexe",
  };
}

// Case 1: module entry loading
const mysql = require("mysql2");
const mysqlPromise = require("mysql2/promise");

assert(
  typeof mysql.createConnection === "function",
  "mysql.createConnection should be a function"
);
assert(
  typeof mysql.createPool === "function",
  "mysql.createPool should be a function"
);
assert(
  typeof mysqlPromise.createConnection === "function",
  "mysqlPromise.createConnection should be a function"
);
assert(
  typeof mysqlPromise.createPool === "function",
  "mysqlPromise.createPool should be a function"
);

async function main() {
  const config = getConfig();

  // Case 2: Promise connection and basic query
  const connection = await mysqlPromise.createConnection(config);
  try {
    const [rows, fields] = await connection.execute("SELECT 1 AS value");
    assert(
      Array.isArray(rows) && rows.length === 1,
      "SELECT 1 should return one row"
    );
    assert(rows[0].value === 1, "value should be 1");
    assert(Array.isArray(fields), "fields should be an array");
    assert(fields[0].name === "value", 'field name should be "value"');

    // Case 3: prepare test table
    await connection.execute("DROP TABLE IF EXISTS lexe_mysql2_compat");
    await connection.execute(`
      CREATE TABLE lexe_mysql2_compat (
        id INT NOT NULL AUTO_INCREMENT PRIMARY KEY,
        name VARCHAR(64) NOT NULL UNIQUE,
        score INT NOT NULL
      )
    `);

    // Case 4: parameterized execute()
    const [insertResult] = await connection.execute(
      "INSERT INTO lexe_mysql2_compat (name, score) VALUES (?, ?)",
      ["alpha", 7]
    );
    assert(insertResult.affectedRows === 1, "insert affectedRows should be 1");
    assert(insertResult.insertId === 1, "insertId should be 1");

    const [alphaRows] = await connection.execute(
      "SELECT id, name, score FROM lexe_mysql2_compat WHERE id = ?",
      [insertResult.insertId]
    );
    assert(alphaRows.length === 1, "should find one row by id");
    assert(
      alphaRows[0].id === 1 &&
        alphaRows[0].name === "alpha" &&
        alphaRows[0].score === 7,
      "alpha row mismatch"
    );

    // Case 5: transaction commit
    await connection.beginTransaction();
    try {
      await connection.execute(
        "INSERT INTO lexe_mysql2_compat (name, score) VALUES (?, ?)",
        ["beta", 8]
      );
      await connection.commit();
    } catch (err) {
      await connection.rollback();
      throw err;
    }

    const [betaRows] = await connection.execute(
      "SELECT name FROM lexe_mysql2_compat WHERE name = ?",
      ["beta"]
    );
    assert(
      betaRows.length === 1 && betaRows[0].name === "beta",
      "beta should be readable after commit"
    );

    const [countAfterCommit] = await connection.execute(
      "SELECT COUNT(*) AS c FROM lexe_mysql2_compat"
    );
    assert(countAfterCommit[0].c === 2, "should have 2 rows after commit");

    // Case 6: transaction rollback
    await connection.beginTransaction();
    try {
      await connection.execute(
        "INSERT INTO lexe_mysql2_compat (name, score) VALUES (?, ?)",
        ["rollback-row", 9]
      );
      await connection.rollback();
    } catch (err) {
      await connection.rollback();
      throw err;
    }

    const [rollbackRows] = await connection.execute(
      "SELECT name FROM lexe_mysql2_compat WHERE name = ?",
      ["rollback-row"]
    );
    assert(rollbackRows.length === 0, "rollback row should not be visible");

    const [countAfterRollback] = await connection.execute(
      "SELECT COUNT(*) AS c FROM lexe_mysql2_compat"
    );
    assert(
      countAfterRollback[0].c === 2,
      "should still have 2 rows after rollback"
    );

    // Case 7: Promise error propagation
    let sawError = false;
    try {
      await connection.execute("SELECT * FROM lexe_mysql2_missing_table");
    } catch (error) {
      sawError = true;
      assert(error instanceof Error, "error should be an Error");
      assert(
        error.code === "ER_NO_SUCH_TABLE",
        `error.code should be ER_NO_SUCH_TABLE, got ${error.code}`
      );
      assert(
        error.errno === 1146,
        `error.errno should be 1146, got ${error.errno}`
      );
    }
    assert(sawError, "missing table query should reject");
  } finally {
    await connection.end();
  }

  // Case 8: callback API
  const callbackConnection = mysql.createConnection(config);
  let callbackClosed = false;
  try {
    await new Promise((resolve, reject) => {
      callbackConnection.connect((error) => {
        if (error) {
          reject(error);
          return;
        }
        resolve();
      });
    });

    await new Promise((resolve, reject) => {
      callbackConnection.query(
        "SELECT name, score FROM lexe_mysql2_compat ORDER BY id",
        (error, rows, fields) => {
          if (error) {
            reject(error);
            return;
          }
          try {
            assert(error === null, "callback error should be null");
            assert(rows.length === 2, "callback query should return 2 rows");
            assert(
              rows[0].name === "alpha" && rows[0].score === 7,
              "first row should be alpha/7"
            );
            assert(
              rows[1].name === "beta" && rows[1].score === 8,
              "second row should be beta/8"
            );
            assert(fields[0].name === "name", "first field should be name");
            assert(fields[1].name === "score", "second field should be score");
            resolve();
          } catch (err) {
            reject(err);
          }
        }
      );
    });

    await new Promise((resolve, reject) => {
      callbackConnection.end((error) => {
        if (error) {
          reject(error);
          return;
        }
        callbackClosed = true;
        resolve();
      });
    });
  } finally {
    if (!callbackClosed) {
      callbackConnection.destroy();
    }
  }

  // Cases 9-10: pool
  const pool = mysqlPromise.createPool({
    ...config,
    connectionLimit: 2,
    waitForConnections: true,
    queueLimit: 0,
  });
  try {
    const [conn1, conn2] = await Promise.all([
      pool.getConnection(),
      pool.getConnection(),
    ]);
    try {
      const [[id1Rows], [id2Rows]] = await Promise.all([
        conn1.execute("SELECT CONNECTION_ID() AS id"),
        conn2.execute("SELECT CONNECTION_ID() AS id"),
      ]);
      const id1 = id1Rows[0].id;
      const id2 = id2Rows[0].id;
      assert(id1 > 0 && id2 > 0, "connection IDs should be positive");
      assert(id1 !== id2, "connection IDs should differ");
    } finally {
      conn1.release();
      conn2.release();
    }

    const [gammaResult, deltaResult] = await Promise.all([
      pool.execute(
        "INSERT INTO lexe_mysql2_compat (name, score) VALUES (?, ?)",
        ["gamma", 10]
      ),
      pool.execute(
        "INSERT INTO lexe_mysql2_compat (name, score) VALUES (?, ?)",
        ["delta", 11]
      ),
    ]);
    assert(
      gammaResult[0].affectedRows === 1,
      "gamma insert affectedRows should be 1"
    );
    assert(
      deltaResult[0].affectedRows === 1,
      "delta insert affectedRows should be 1"
    );

    const [countRows] = await pool.query(
      "SELECT COUNT(*) AS c FROM lexe_mysql2_compat"
    );
    assert(countRows[0].c === 4, "should have 4 rows total");

    const [nameRows] = await pool.query(
      "SELECT name FROM lexe_mysql2_compat ORDER BY name"
    );
    const names = nameRows.map((row) => row.name);
    assert(
      names.length === 4 &&
        names[0] === "alpha" &&
        names[1] === "beta" &&
        names[2] === "delta" &&
        names[3] === "gamma",
      `names should be alpha,beta,delta,gamma, got ${names.join(",")}`
    );

    await pool.query("DROP TABLE IF EXISTS lexe_mysql2_compat");
  } finally {
    await pool.end();
  }

  console.log("mysql2 compat OK");
}

main().catch((error) => {
  console.error(error?.stack || error);
  process.exitCode = 1;
});
