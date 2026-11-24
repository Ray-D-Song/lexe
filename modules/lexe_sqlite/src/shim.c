#include "sqlite3.h"

#if defined(__GNUC__)
#define LEXE_SQLITE_EXPORT __attribute__((visibility("default")))
#else
#define LEXE_SQLITE_EXPORT
#endif

LEXE_SQLITE_EXPORT int lexe_sqlite3_db_config_enable(sqlite3 *db, int op,
                                                        int value) {
  return sqlite3_db_config(db, op, value, 0);
}

LEXE_SQLITE_EXPORT int lexe_sqlite3_bind_text_transient(sqlite3_stmt *stmt,
                                                            int index,
                                                            const char *data,
                                                            int length) {
  return sqlite3_bind_text(stmt, index, data, length, SQLITE_TRANSIENT);
}

LEXE_SQLITE_EXPORT int lexe_sqlite3_bind_blob_transient(sqlite3_stmt *stmt,
                                                            int index,
                                                            const void *data,
                                                            int length) {
  return sqlite3_bind_blob(stmt, index, data, length, SQLITE_TRANSIENT);
}

LEXE_SQLITE_EXPORT void lexe_sqlite3_result_text_transient(
    sqlite3_context *context, const char *data, int length) {
  sqlite3_result_text(context, data, length, SQLITE_TRANSIENT);
}

LEXE_SQLITE_EXPORT void lexe_sqlite3_result_blob_transient(
    sqlite3_context *context, const void *data, int length) {
  sqlite3_result_blob(context, data, length, SQLITE_TRANSIENT);
}
