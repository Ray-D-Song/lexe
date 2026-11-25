// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
mod access;
mod chmod;
mod constants;
mod errors;
pub mod fd_async;
pub mod fd_sync;
mod file_handle;
mod fs_extra;
mod mkdir;
mod open;
mod read_dir;
mod read_file;
mod read_stream;
mod realpath;
mod rename;
mod rm;
mod stats;
mod symlink;
mod watch;
mod write_file;
mod write_stream;

use std::collections::HashSet;

use lexe_events::Emitter;
use lexe_utils::module::{export_default, ModuleInfo};
#[cfg(unix)]
use lexe_utils::result::ResultExt;
use rquickjs::{
    atom::PredefinedAtom,
    module::{Declarations, Exports, ModuleDef},
    prelude::{Async, Func, Opt},
    Array, BigInt, Class, Ctx, Exception, Function, IntoJs, Object, Result, Value,
};

use self::access::{access, access_callback, access_sync};
use self::chmod::{chmod, chmod_sync, lchmod, lchmod_sync};
use self::fd_async::{
    fchmod_callback, fchown_callback, fdatasync_callback, fstat_callback, fsync_callback,
    ftruncate_callback, futimes_callback, read_callback, readv_callback, write_callback,
    writev_callback,
};
use self::fd_sync::{
    close_sync, fchmod_sync, fchown_sync, fdatasync_sync, fstat_sync, fsync_sync, ftruncate_sync,
    futimes_sync, open_sync, read_sync, readv_sync, write_sync, writev_sync,
};
use self::file_handle::FileHandle;
use self::mkdir::{mkdir, mkdir_sync, mkdtemp, mkdtemp_sync};
use self::open::open;
use self::read_dir::{read_dir, read_dir_sync, readdir_callback, Dirent};
use self::read_file::{read_file, read_file_sync};
use self::read_stream::create_create_read_stream;
use self::realpath::{path_from_value, realpath, realpath_promises, realpath_sync};
use self::rename::{rename, rename_sync};
use self::rm::{rmdir, rmdir_sync, rmfile, rmfile_sync, unlink, unlink_sync};
use self::stats::{
    lstat_callback, lstat_fn, lstat_fn_sync, stat_callback, stat_fn, stat_fn_sync, zeroed_stats,
    BigIntStats, Stats,
};
use self::symlink::{symlink, symlink_sync};
use self::watch::{watch, FSWatcher};
use self::write_file::{append_file, append_file_sync, write_file, write_file_sync};
use self::write_stream::create_create_write_stream;

pub const CONSTANT_F_OK: u32 = 0;
pub const CONSTANT_R_OK: u32 = 4;
pub const CONSTANT_W_OK: u32 = 2;
pub const CONSTANT_X_OK: u32 = 1;

/// Pure-JS base classes and helpers that have no external `require()` dependency
/// at registration time (`Dir` and lazy `FileHandle` stream methods).
const FS_BASE_CLASSES_JS: &str = r#"(function () {
  "use strict";

  function makeDirClosedError() {
    const err = new Error("Directory handle was closed");
    err.code = "ERR_DIR_CLOSED";
    return err;
  }

  class Dir {
    constructor(path, entries) {
      this.path = path;
      this._entries = entries || [];
      this._index = 0;
      this._closed = false;
    }

    readSync() {
      if (this._closed) throw makeDirClosedError();
      if (this._index >= this._entries.length) return null;
      return this._entries[this._index++];
    }

    read(cb) {
      if (typeof cb === "function") {
        queueMicrotask(() => {
          if (this._closed) {
            cb(makeDirClosedError());
            return;
          }
          if (this._index >= this._entries.length) {
            cb(null, null);
            return;
          }
          cb(null, this._entries[this._index++]);
        });
        return;
      }
      if (this._closed) return Promise.reject(makeDirClosedError());
      if (this._index >= this._entries.length) return Promise.resolve(null);
      return Promise.resolve(this._entries[this._index++]);
    }

    close(cb) {
      this._closed = true;
      if (typeof cb === "function") {
        queueMicrotask(() => cb(null));
        return;
      }
      return Promise.resolve();
    }

    closeSync() {
      this._closed = true;
    }

    async *[Symbol.asyncIterator]() {
      for (;;) {
        const entry = await this.read();
        if (entry === null) return;
        yield entry;
      }
    }
  }

  if (typeof Symbol.dispose === "symbol") {
    Dir.prototype[Symbol.dispose] = function () {
      this.closeSync();
    };
  }
  if (typeof Symbol.asyncDispose === "symbol") {
    Dir.prototype[Symbol.asyncDispose] = function () {
      return this.close();
    };
  }

  if (typeof globalThis.__lexeFsDefer2 !== "function") {
    Object.defineProperty(globalThis, "__lexeFsDefer2", {
      value(cb, a, b) {
        queueMicrotask(() => cb(null, a, b));
      },
      configurable: true,
    });
  }

  return { Dir };
})()"#;

/// Stream subclasses (`ReadStream`, `WriteStream`) registered when `node:fs` loads.
const FS_STREAM_CLASSES_JS: &str = r#"(function () {
  "use strict";
  const { Readable, Writable } = require("node:stream");

  class ReadStream extends Readable {
    constructor(path, options) {
      const factory = globalThis.__lexeFsCreateReadStream;
      if (typeof factory === "function") {
        return factory(path, options);
      }
      options = options || {};
      super(options);
      this.path = path;
      this.pending = true;
      this.fd = options.fd == null ? null : options.fd;
      this.flags = options.flags == null ? "r" : options.flags;
      this.mode = options.mode == null ? 0o666 : options.mode;
      this.start = options.start;
      this.end = options.end;
      this.pos = this.start;
      this.bytesRead = 0;
    }
  }

  class WriteStream extends Writable {
    constructor(path, options) {
      const factory = globalThis.__lexeFsCreateWriteStream;
      if (typeof factory === "function") {
        return factory(path, options);
      }
      options = options || {};
      super(options);
      this.path = path;
      this.pending = true;
      this.fd = options.fd == null ? null : options.fd;
      this.flags = options.flags == null ? "w" : options.flags;
      this.mode = options.mode == null ? 0o666 : options.mode;
      this.start = options.start;
      this.pos = this.start;
      this.bytesWritten = 0;
    }
  }

  return { ReadStream, WriteStream };
})()"#;

/// Full `cp` / `cpSync` JS helpers supporting `recursive`, `force`,
/// `errorOnExist`, `preserveTimestamps`, `dereference`, `verbatimSymlinks`, and `filter`.
const FS_CP_HELPER_JS: &str = r#"(function (api) {
  "use strict";

  function joinPath(a, b) {
    if (a.endsWith("/")) return a + b;
    return a + "/" + b;
  }

  function dirname(p) {
    const idx = p.lastIndexOf("/");
    if (idx <= 0) return idx === 0 ? "/" : ".";
    return p.slice(0, idx);
  }

  function resolveRelative(fromDir, target) {
    if (target.startsWith("/")) return target;
    return joinPath(fromDir, target);
  }

  function makeCpError(code, message) {
    const err = new Error(message);
    err.code = code;
    return err;
  }

  function normalizeCpOptions(options) {
    const opts = options || {};
    return {
      dereference: Boolean(opts.dereference),
      errorOnExist: Boolean(opts.errorOnExist),
      filter: typeof opts.filter === "function" ? opts.filter : null,
      force: opts.force === undefined ? true : Boolean(opts.force),
      mode: typeof opts.mode === "number" ? opts.mode : 0,
      preserveTimestamps: Boolean(opts.preserveTimestamps),
      recursive: Boolean(opts.recursive),
      verbatimSymlinks: Boolean(opts.verbatimSymlinks),
    };
  }

  function existsSyncNoThrow(p) {
    return api.statSync(p, { throwIfNoEntry: false }) !== undefined ||
      api.lstatSync(p, { throwIfNoEntry: false }) !== undefined;
  }

  async function existsAsyncNoThrow(p) {
    try {
      await api.lstat(p);
      return true;
    } catch (_) {
      return false;
    }
  }

  async function tryLstatAsync(p) {
    try {
      return await api.lstat(p);
    } catch (e) {
      if (e && e.code === "ENOENT") return null;
      throw e;
    }
  }

  function copySync(src, dest, options) {
    const opts = normalizeCpOptions(options);
    const statFn = opts.dereference ? api.statSync : api.lstatSync;
    const srcStat = statFn(src);

    function runFilter(s, d) {
      if (!opts.filter) return true;
      const res = opts.filter(s, d);
      if (res && (typeof res === "object" || typeof res === "function") && typeof res.then === "function") {
        const err = new TypeError("The filter function must not return a Promise for cpSync");
        err.code = "ERR_INVALID_RETURN_VALUE";
        throw err;
      }
      return Boolean(res);
    }

    function copyEntry(curSrc, curDest, curStat, isRoot) {
      if (!runFilter(curSrc, curDest)) return;
      const destLstat = api.lstatSync(curDest, { throwIfNoEntry: false });

      if (curStat.isSymbolicLink()) {
        if (destLstat !== undefined) {
          if (!opts.force) {
            if (opts.errorOnExist) {
              throw makeCpError("ERR_FS_CP_EEXIST", `Destination already exists: ${curDest}`);
            }
            return;
          }
          api.rmSync(curDest, { recursive: true, force: true });
        }
        let linkTarget = api.readlinkSync(curSrc);
        if (!opts.verbatimSymlinks) {
          linkTarget = resolveRelative(dirname(curSrc), linkTarget);
        }
        api.symlinkSync(linkTarget, curDest);
        if (opts.preserveTimestamps) {
          api.lutimesSync(curDest, curStat.atime, curStat.mtime);
        }
        return;
      }

      if (curStat.isDirectory()) {
        if (!opts.recursive) {
          throw makeCpError("ERR_FS_EISDIR", `${curSrc} is a directory (use recursive: true)`);
        }
        if (destLstat !== undefined && !destLstat.isDirectory()) {
          if (!opts.force) {
            if (opts.errorOnExist) {
              throw makeCpError("ERR_FS_CP_EEXIST", `Destination already exists: ${curDest}`);
            }
            return;
          }
          api.rmSync(curDest, { recursive: true, force: true });
        }
        if (destLstat === undefined || !destLstat.isDirectory()) {
          api.mkdirSync(curDest, { recursive: true });
        }
        const names = api.readdirSync(curSrc);
        for (const name of names) {
          const childSrc = joinPath(curSrc, name);
          const childDest = joinPath(curDest, name);
          const childStat = statFn(childSrc);
          copyEntry(childSrc, childDest, childStat, false);
        }
        if (opts.preserveTimestamps) {
          api.utimesSync(curDest, curStat.atime, curStat.mtime);
        }
        return;
      }

      if (destLstat !== undefined) {
        if (destLstat.isDirectory()) {
          throw makeCpError("ERR_FS_CP_DIR_TO_NON_DIR", `Cannot overwrite directory ${curDest} with non-directory`);
        }
        if (!opts.force) {
          if (opts.errorOnExist) {
            throw makeCpError("ERR_FS_CP_EEXIST", `Destination already exists: ${curDest}`);
          }
          return;
        }
        if (destLstat.isSymbolicLink()) {
          api.unlinkSync(curDest);
        }
      } else if (isRoot) {
        const parent = dirname(curDest);
        if (parent && parent !== "." && !existsSyncNoThrow(parent)) {
          api.mkdirSync(parent, { recursive: true });
        }
      }
      api.copyFileSync(curSrc, curDest, opts.mode);
      if (opts.preserveTimestamps) {
        api.utimesSync(curDest, curStat.atime, curStat.mtime);
      }
    }

    copyEntry(src, dest, srcStat, true);
  }

  async function copyAsync(src, dest, options) {
    const opts = normalizeCpOptions(options);
    const statFn = opts.dereference ? api.stat : api.lstat;
    const srcStat = await statFn(src);

    async function copyEntry(curSrc, curDest, curStat, isRoot) {
      if (opts.filter) {
        const keep = await opts.filter(curSrc, curDest);
        if (!keep) return;
      }
      const destLstat = await tryLstatAsync(curDest);

      if (curStat.isSymbolicLink()) {
        if (destLstat !== null) {
          if (!opts.force) {
            if (opts.errorOnExist) {
              throw makeCpError("ERR_FS_CP_EEXIST", `Destination already exists: ${curDest}`);
            }
            return;
          }
          await api.rm(curDest, { recursive: true, force: true });
        }
        let linkTarget = await api.readlink(curSrc);
        if (!opts.verbatimSymlinks) {
          linkTarget = resolveRelative(dirname(curSrc), linkTarget);
        }
        await api.symlink(linkTarget, curDest);
        if (opts.preserveTimestamps) {
          await api.lutimes(curDest, curStat.atime, curStat.mtime);
        }
        return;
      }

      if (curStat.isDirectory()) {
        if (!opts.recursive) {
          throw makeCpError("ERR_FS_EISDIR", `${curSrc} is a directory (use recursive: true)`);
        }
        if (destLstat !== null && !destLstat.isDirectory()) {
          if (!opts.force) {
            if (opts.errorOnExist) {
              throw makeCpError("ERR_FS_CP_EEXIST", `Destination already exists: ${curDest}`);
            }
            return;
          }
          await api.rm(curDest, { recursive: true, force: true });
        }
        if (destLstat === null || !destLstat.isDirectory()) {
          await api.mkdir(curDest, { recursive: true });
        }
        const names = await api.readdir(curSrc);
        for (const name of names) {
          const childSrc = joinPath(curSrc, name);
          const childDest = joinPath(curDest, name);
          const childStat = await statFn(childSrc);
          await copyEntry(childSrc, childDest, childStat, false);
        }
        if (opts.preserveTimestamps) {
          await api.utimes(curDest, curStat.atime, curStat.mtime);
        }
        return;
      }

      if (destLstat !== null) {
        if (destLstat.isDirectory()) {
          throw makeCpError("ERR_FS_CP_DIR_TO_NON_DIR", `Cannot overwrite directory ${curDest} with non-directory`);
        }
        if (!opts.force) {
          if (opts.errorOnExist) {
            throw makeCpError("ERR_FS_CP_EEXIST", `Destination already exists: ${curDest}`);
          }
          return;
        }
        if (destLstat.isSymbolicLink()) {
          await api.unlink(curDest);
        }
      } else if (isRoot) {
        const parent = dirname(curDest);
        if (parent && parent !== "." && !(await existsAsyncNoThrow(parent))) {
          await api.mkdir(parent, { recursive: true });
        }
      }
      await api.copyFile(curSrc, curDest, opts.mode);
      if (opts.preserveTimestamps) {
        await api.utimes(curDest, curStat.atime, curStat.mtime);
      }
    }

    await copyEntry(src, dest, srcStat, true);
  }

  return { copySync, copyAsync };
})"#;

/// JS helper creating `fs/promises.watch(filename[, options])` AsyncIterable.
const FS_PROMISES_WATCH_JS: &str = r#"(function (watchFn) {
  "use strict";

  function makeAbortError(signal) {
    const err = new Error("The operation was aborted", { cause: signal.reason });
    err.name = "AbortError";
    err.code = "ABORT_ERR";
    return err;
  }

  return function watchAsyncIterable(filename, options) {
    let opts = {};
    if (typeof options === "string") {
      opts = { encoding: options };
    } else if (options != null) {
      if (typeof options !== "object") {
        throw new TypeError('The "options" argument must be of type object. Received type ' + typeof options);
      }
      opts = options;
    }

    const signal = opts.signal || null;

    return {
      [Symbol.asyncIterator]() {
        if (signal && signal.aborted) {
          let rejectedOnce = false;
          return {
            next() {
              if (!rejectedOnce) {
                rejectedOnce = true;
                return Promise.reject(makeAbortError(signal));
              }
              return Promise.resolve({ value: undefined, done: true });
            },
            return() {
              return Promise.resolve({ value: undefined, done: true });
            },
          };
        }

        const queue = [];
        const waiters = [];
        let closed = false;
        let pendingError = null;
        const watcher = watchFn(filename, opts);

        function flushError(err) {
          if (closed) return;
          closed = true;
          queue.length = 0;
          try { watcher.close(); } catch (_) {}
          if (waiters.length > 0) {
            const w = waiters.shift();
            w.reject(err);
          } else {
            pendingError = err;
          }
          while (waiters.length > 0) {
            waiters.shift().resolve({ value: undefined, done: true });
          }
        }

        watcher.on("change", (eventType, changedName) => {
          if (closed) return;
          const item = { eventType, filename: changedName };
          if (waiters.length > 0) {
            waiters.shift().resolve({ value: item, done: false });
          } else {
            queue.push(item);
          }
        });

        watcher.on("error", (err) => {
          flushError(err);
        });

        watcher.on("close", () => {
          if (closed) return;
          closed = true;
          while (waiters.length > 0) {
            waiters.shift().resolve({ value: undefined, done: true });
          }
        });

        return {
          next() {
            if (pendingError !== null) {
              const err = pendingError;
              pendingError = null;
              return Promise.reject(err);
            }
            if (queue.length > 0) {
              return Promise.resolve({ value: queue.shift(), done: false });
            }
            if (closed) {
              return Promise.resolve({ value: undefined, done: true });
            }
            return new Promise((resolve, reject) => {
              waiters.push({ resolve, reject });
            });
          },
          return() {
            if (!closed) {
              closed = true;
              try { watcher.close(); } catch (_) {}
              while (waiters.length > 0) {
                waiters.shift().resolve({ value: undefined, done: true });
              }
            }
            return Promise.resolve({ value: undefined, done: true });
          },
        };
      },
    };
  };
})"#;

/// JS implementation of `fs.watchFile` / `fs.unwatchFile` (`StatWatcher`).
const FS_WATCH_FILE_JS: &str = r#"(function (statSync, zeroedStats) {
  "use strict";
  const EventEmitter = require("node:events");
  const pathMod = require("node:path");

  class StatWatcher extends EventEmitter {
    constructor(bigint) {
      super();
      this._bigint = Boolean(bigint);
      this._timer = null;
      this._prev = null;
      this._stopped = false;
    }

    ref() {
      if (this._timer && typeof this._timer.ref === "function") {
        this._timer.ref();
      }
      return this;
    }

    unref() {
      if (this._timer && typeof this._timer.unref === "function") {
        this._timer.unref();
      }
      return this;
    }
  }

  const statWatchers = new Map();

  function safeStat(filename, bigint) {
    const res = statSync(filename, { bigint: Boolean(bigint), throwIfNoEntry: false });
    if (res !== undefined) return res;
    return zeroedStats(Boolean(bigint));
  }

  function statsChanged(a, b) {
    return (
      a.mtimeMs !== b.mtimeMs ||
      a.ctimeMs !== b.ctimeMs ||
      a.size !== b.size ||
      a.ino !== b.ino ||
      a.mode !== b.mode ||
      a.nlink !== b.nlink
    );
  }

  function watchFile(filename, options, listener) {
    if (typeof options === "function") {
      listener = options;
      options = {};
    } else if (options == null) {
      options = {};
    } else if (typeof options !== "object") {
      throw new TypeError('The "options" argument must be of type object');
    }

    if (typeof listener !== "function") {
      const err = new TypeError('The "listener" argument must be of type function');
      err.code = "ERR_INVALID_ARG_TYPE";
      throw err;
    }

    const persistent = options.persistent === undefined ? true : Boolean(options.persistent);
    const interval = typeof options.interval === "number" ? options.interval : 5007;
    const bigint = Boolean(options.bigint);
    const resolved = typeof filename === "string" ? pathMod.resolve(filename) : String(filename);

    let watcher = statWatchers.get(resolved);
    if (!watcher) {
      watcher = new StatWatcher(bigint);
      watcher._prev = safeStat(resolved, bigint);
      watcher._timer = setInterval(() => {
        if (watcher._stopped) return;
        const curr = safeStat(resolved, watcher._bigint);
        const prev = watcher._prev;
        if (statsChanged(curr, prev)) {
          watcher._prev = curr;
          watcher.emit("change", curr, prev);
        }
      }, interval);
      if (!persistent && watcher._timer && typeof watcher._timer.unref === "function") {
        watcher._timer.unref();
      }
      statWatchers.set(resolved, watcher);
    }

    watcher.on("change", listener);
    return watcher;
  }

  function unwatchFile(filename, listener) {
    const resolved = typeof filename === "string" ? pathMod.resolve(filename) : String(filename);
    const watcher = statWatchers.get(resolved);
    if (!watcher) return;

    if (typeof listener === "function") {
      watcher.removeListener("change", listener);
    } else {
      watcher.removeAllListeners("change");
    }

    if (watcher.listenerCount("change") === 0) {
      watcher._stopped = true;
      if (watcher._timer) {
        clearInterval(watcher._timer);
        watcher._timer = null;
      }
      statWatchers.delete(resolved);
      watcher.emit("stop");
    }
  }

  return { StatWatcher, watchFile, unwatchFile };
})"#;

fn ensure_fs_base_classes_registered<'js>(ctx: &Ctx<'js>) -> Result<()> {
    let globals = ctx.globals();

    Class::<Dirent>::define(&globals)?;
    Class::<FileHandle>::define(&globals)?;
    Class::<Stats>::define(&globals)?;
    Class::<BigIntStats>::define(&globals)?;

    if !globals.contains_key("FSWatcher")? {
        let fs_watcher_ctor = Class::<FSWatcher>::create_constructor(ctx)?
            .expect("Can't create FSWatcher constructor");
        globals.set("FSWatcher", fs_watcher_ctor)?;
        FSWatcher::add_event_emitter_prototype(ctx)?;
    }

    if !globals.contains_key("__lexeFsDirClass")? {
        let base_classes: Object = ctx.eval(FS_BASE_CLASSES_JS)?;
        let dir_ctor: Function = base_classes.get("Dir")?;
        globals.set("__lexeFsDirClass", dir_ctor)?;
    }

    if let Some(fh_proto) = Class::<FileHandle>::prototype(ctx)? {
        if !fh_proto.contains_key("createReadStream")? {
            let attach: Function = ctx.eval(
                r#"(function(proto) {
  proto.createReadStream = function(options) {
    return require("node:fs").createReadStream(null, Object.assign({}, options, { fd: this }));
  };
  proto.createWriteStream = function(options) {
    return require("node:fs").createWriteStream(null, Object.assign({}, options, { fd: this }));
  };
})"#,
            )?;
            attach.call::<_, ()>((fh_proto,))?;
        }
    }

    Ok(())
}

pub struct FsPromisesModule;

impl ModuleDef for FsPromisesModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("access")?;
        declare.declare("appendFile")?;
        declare.declare("chmod")?;
        declare.declare("chown")?;
        declare.declare("constants")?;
        declare.declare("copyFile")?;
        declare.declare("cp")?;
        declare.declare("glob")?;
        declare.declare("lchmod")?;
        declare.declare("lchown")?;
        declare.declare("link")?;
        declare.declare("lstat")?;
        declare.declare("lutimes")?;
        declare.declare("mkdir")?;
        declare.declare("mkdtemp")?;
        declare.declare("open")?;
        declare.declare("opendir")?;
        declare.declare("readFile")?;
        declare.declare("readdir")?;
        declare.declare("readlink")?;
        declare.declare("realpath")?;
        declare.declare("rename")?;
        declare.declare("rm")?;
        declare.declare("rmdir")?;
        declare.declare("stat")?;
        declare.declare("statfs")?;
        declare.declare("symlink")?;
        declare.declare("truncate")?;
        declare.declare("unlink")?;
        declare.declare("utimes")?;
        declare.declare("watch")?;
        declare.declare("writeFile")?;

        declare.declare("default")?;

        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        ensure_fs_base_classes_registered(ctx)?;

        export_default(ctx, exports, |default| {
            export_promises(ctx, default)?;
            Ok(())
        })
    }
}

impl From<FsPromisesModule> for ModuleInfo<FsPromisesModule> {
    fn from(val: FsPromisesModule) -> Self {
        ModuleInfo {
            name: "fs/promises",
            module: val,
        }
    }
}

pub struct FsModule;

impl ModuleDef for FsModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("promises")?;
        declare.declare("access")?;
        declare.declare("accessSync")?;
        declare.declare("appendFile")?;
        declare.declare("appendFileSync")?;
        declare.declare("chmod")?;
        declare.declare("chmodSync")?;
        declare.declare("chown")?;
        declare.declare("chownSync")?;
        declare.declare("close")?;
        declare.declare("closeSync")?;
        declare.declare("constants")?;
        declare.declare("copyFile")?;
        declare.declare("copyFileSync")?;
        declare.declare("cp")?;
        declare.declare("cpSync")?;
        declare.declare("createReadStream")?;
        declare.declare("createWriteStream")?;
        declare.declare("exists")?;
        declare.declare("existsSync")?;
        declare.declare("fchmod")?;
        declare.declare("fchmodSync")?;
        declare.declare("fchown")?;
        declare.declare("fchownSync")?;
        declare.declare("fdatasync")?;
        declare.declare("fdatasyncSync")?;
        declare.declare("fstat")?;
        declare.declare("fstatSync")?;
        declare.declare("fsync")?;
        declare.declare("fsyncSync")?;
        declare.declare("ftruncate")?;
        declare.declare("ftruncateSync")?;
        declare.declare("futimes")?;
        declare.declare("futimesSync")?;
        declare.declare("glob")?;
        declare.declare("globSync")?;
        declare.declare("lchmod")?;
        declare.declare("lchmodSync")?;
        declare.declare("lchown")?;
        declare.declare("lchownSync")?;
        declare.declare("link")?;
        declare.declare("linkSync")?;
        declare.declare("lstat")?;
        declare.declare("lstatSync")?;
        declare.declare("lutimes")?;
        declare.declare("lutimesSync")?;
        declare.declare("mkdir")?;
        declare.declare("mkdirSync")?;
        declare.declare("mkdtemp")?;
        declare.declare("mkdtempSync")?;
        declare.declare("open")?;
        declare.declare("openAsBlob")?;
        declare.declare("openSync")?;
        declare.declare("opendir")?;
        declare.declare("opendirSync")?;
        declare.declare("read")?;
        declare.declare("readFile")?;
        declare.declare("readFileSync")?;
        declare.declare("readSync")?;
        declare.declare("readdir")?;
        declare.declare("readdirSync")?;
        declare.declare("readlink")?;
        declare.declare("readlinkSync")?;
        declare.declare("readv")?;
        declare.declare("readvSync")?;
        declare.declare("realpath")?;
        declare.declare("realpathSync")?;
        declare.declare("rename")?;
        declare.declare("renameSync")?;
        declare.declare("rm")?;
        declare.declare("rmSync")?;
        declare.declare("rmdir")?;
        declare.declare("rmdirSync")?;
        declare.declare("stat")?;
        declare.declare("statSync")?;
        declare.declare("statfs")?;
        declare.declare("statfsSync")?;
        declare.declare("symlink")?;
        declare.declare("symlinkSync")?;
        declare.declare("truncate")?;
        declare.declare("truncateSync")?;
        declare.declare("unlink")?;
        declare.declare("unlinkSync")?;
        declare.declare("unwatchFile")?;
        declare.declare("utimes")?;
        declare.declare("utimesSync")?;
        declare.declare("watch")?;
        declare.declare("watchFile")?;
        declare.declare("write")?;
        declare.declare("writeFile")?;
        declare.declare("writeFileSync")?;
        declare.declare("writeSync")?;
        declare.declare("writev")?;
        declare.declare("writevSync")?;

        declare.declare("Stats")?;
        declare.declare("BigIntStats")?;
        declare.declare("Dirent")?;
        declare.declare("FileHandle")?;
        declare.declare("FSWatcher")?;
        declare.declare("ReadStream")?;
        declare.declare("WriteStream")?;
        declare.declare("FileReadStream")?;
        declare.declare("FileWriteStream")?;
        declare.declare("Dir")?;
        declare.declare("_toUnixTimestamp")?;

        declare.declare("default")?;

        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        ensure_fs_base_classes_registered(ctx)?;
        let globals = ctx.globals();

        let fs_watcher_ctor: Function = globals.get("FSWatcher")?;
        let stats_ctor = Class::<Stats>::prototype(ctx)?
            .and_then(|p| p.get::<_, Value>("constructor").ok())
            .and_then(|c| c.as_function().cloned());
        let bigint_stats_ctor = Class::<BigIntStats>::prototype(ctx)?
            .and_then(|p| p.get::<_, Value>("constructor").ok())
            .and_then(|c| c.as_function().cloned());
        let dirent_ctor = Class::<Dirent>::prototype(ctx)?
            .and_then(|p| p.get::<_, Value>("constructor").ok())
            .and_then(|c| c.as_function().cloned());
        let file_handle_ctor = Class::<FileHandle>::prototype(ctx)?
            .and_then(|p| p.get::<_, Value>("constructor").ok())
            .and_then(|c| c.as_function().cloned());

        export_default(ctx, exports, |default| {
            let promises = Object::new(ctx.clone())?;
            export_promises(ctx, &promises)?;
            export_constants(ctx, default)?;

            default.set("promises", promises.clone())?;

            default.set("access", Function::new(ctx.clone(), access_callback)?)?;
            default.set("accessSync", Func::from(access_sync))?;
            default.set("mkdirSync", Func::from(mkdir_sync))?;
            default.set("mkdtempSync", Func::from(mkdtemp_sync))?;
            default.set("readdir", Function::new(ctx.clone(), readdir_callback)?)?;
            default.set("readdirSync", Func::from(read_dir_sync))?;

            default.set("openSync", Func::from(open_sync))?;
            default.set("closeSync", Func::from(close_sync))?;
            default.set("writeSync", Func::from(write_sync))?;
            default.set("readSync", Func::from(read_sync))?;
            default.set("fstatSync", Func::from(fstat_sync))?;
            default.set("fchmodSync", Func::from(fchmod_sync))?;
            default.set("fchownSync", Func::from(fchown_sync))?;
            default.set("fdatasyncSync", Func::from(fdatasync_sync))?;
            default.set("fsyncSync", Func::from(fsync_sync))?;
            default.set("ftruncateSync", Func::from(ftruncate_sync))?;
            default.set("futimesSync", Func::from(futimes_sync))?;
            default.set("readvSync", Func::from(readv_sync))?;
            default.set("writevSync", Func::from(writev_sync))?;

            default.set("read", Function::new(ctx.clone(), read_callback)?)?;
            default.set("write", Function::new(ctx.clone(), write_callback)?)?;
            default.set("fstat", Func::from(fstat_callback))?;
            default.set("ftruncate", Func::from(ftruncate_callback))?;
            default.set("fchmod", Func::from(fchmod_callback))?;
            default.set("fchown", Func::from(fchown_callback))?;
            default.set("fsync", Func::from(fsync_callback))?;
            default.set("fdatasync", Func::from(fdatasync_callback))?;
            default.set("futimes", Func::from(futimes_callback))?;
            default.set("readv", Func::from(readv_callback))?;
            default.set("writev", Func::from(writev_callback))?;

            default.set("readFileSync", Func::from(read_file_sync))?;
            default.set("writeFileSync", Func::from(write_file_sync))?;
            default.set("appendFileSync", Func::from(append_file_sync))?;
            default.set("rmdirSync", Func::from(rmdir_sync))?;
            default.set("rmSync", Func::from(rmfile_sync))?;
            default.set("unlinkSync", Func::from(unlink_sync))?;
            default.set("copyFileSync", Func::from(fs_extra::copy_file_sync))?;
            default.set("opendirSync", Func::from(opendir_sync))?;
            default.set("statfsSync", Func::from(statfs_sync))?;
            default.set("globSync", Func::from(glob_sync))?;

            default.set("stat", Function::new(ctx.clone(), stat_callback)?)?;
            default.set("statSync", Func::from(stat_fn_sync))?;
            default.set("lstat", Function::new(ctx.clone(), lstat_callback)?)?;
            default.set("lstatSync", Func::from(lstat_fn_sync))?;
            default.set("watch", Function::new(ctx.clone(), watch)?)?;
            default.set("FSWatcher", fs_watcher_ctor)?;
            default.set("chmodSync", Func::from(chmod_sync))?;
            default.set("lchmodSync", Func::from(lchmod_sync))?;
            default.set("renameSync", Func::from(rename_sync))?;
            default.set("symlinkSync", Func::from(symlink_sync))?;
            default.set("existsSync", Func::from(exists_sync))?;
            default.set("openAsBlob", Func::from(Async(open_as_blob)))?;

            default.set("linkSync", Func::from(fs_extra::link_sync))?;
            default.set("readlinkSync", Func::from(readlink_sync_call))?;
            default.set("truncateSync", Func::from(fs_extra::truncate_sync))?;
            default.set("chownSync", Func::from(fs_extra::chown_sync))?;
            default.set("lchownSync", Func::from(fs_extra::lchown_sync))?;
            default.set("utimesSync", Func::from(utimes_sync))?;
            default.set("lutimesSync", Func::from(lutimes_sync))?;

            let realpath_sync_fn = Function::new(ctx.clone(), realpath_sync)?;
            let realpath_sync_native = Function::new(ctx.clone(), realpath_sync)?;
            realpath_sync_fn.set("native", realpath_sync_native)?;
            default.set("realpathSync", realpath_sync_fn)?;

            let realpath_fn = Function::new(ctx.clone(), realpath)?;
            let realpath_native = Function::new(ctx.clone(), realpath)?;
            realpath_fn.set("native", realpath_native)?;
            default.set("realpath", realpath_fn)?;

            if let Some(ctor) = stats_ctor {
                default.set("Stats", ctor)?;
            }
            if let Some(ctor) = bigint_stats_ctor {
                default.set("BigIntStats", ctor)?;
            }
            if let Some(ctor) = dirent_ctor {
                default.set("Dirent", ctor)?;
            }
            if let Some(ctor) = file_handle_ctor {
                default.set("FileHandle", ctor)?;
            }

            let dir_ctor: Function = globals.get("__lexeFsDirClass")?;
            default.set("Dir", dir_ctor)?;

            default.set("createReadStream", create_create_read_stream(ctx)?)?;
            default.set("createWriteStream", create_create_write_stream(ctx)?)?;
            globals.set("__lexeFsCreateReadStream", create_create_read_stream(ctx)?)?;
            globals.set(
                "__lexeFsCreateWriteStream",
                create_create_write_stream(ctx)?,
            )?;

            let fs_stream_classes: Object = ctx.eval(FS_STREAM_CLASSES_JS)?;
            let read_stream_ctor: Function = fs_stream_classes.get("ReadStream")?;
            let write_stream_ctor: Function = fs_stream_classes.get("WriteStream")?;
            default.set("ReadStream", read_stream_ctor.clone())?;
            default.set("WriteStream", write_stream_ctor.clone())?;
            default.set("FileReadStream", read_stream_ctor.clone())?;
            default.set("FileWriteStream", write_stream_ctor.clone())?;
            globals.set("__lexeFsReadStreamClass", read_stream_ctor)?;
            globals.set("__lexeFsWriteStreamClass", write_stream_ctor)?;

            let cp_factory: Function = ctx.eval(FS_CP_HELPER_JS)?;
            let cp_fns: Object = cp_factory.call((default.clone(),))?;
            let cp_sync_fn: Value = cp_fns.get("copySync")?;
            default.set("cpSync", cp_sync_fn)?;

            let zeroed_fn = Function::new(ctx.clone(), zeroed_stats_js)?;
            let stat_sync_val: Value = default.get("statSync")?;
            let watch_file_factory: Function = ctx.eval(FS_WATCH_FILE_JS)?;
            let watch_file_fns: Object = watch_file_factory.call((stat_sync_val, zeroed_fn))?;
            default.set("watchFile", watch_file_fns.get::<_, Value>("watchFile")?)?;
            default.set(
                "unwatchFile",
                watch_file_fns.get::<_, Value>("unwatchFile")?,
            )?;

            // Callback helpers (open, close, readFile, writeFile, appendFile, exists,
            // glob, and wrappers around all `promises.*` operations).
            let cb_factory: Function = ctx.eval(
                r#"(function(fsp, closeSyncFn, existsSyncFn, globSyncFn){
  const handles = new Map();

  function validateCallback(callback) {
    if (typeof callback !== "function") {
      throw new TypeError('The "cb" argument must be of type function');
    }
  }

  function onceCallback(callback) {
    let called = false;
    return function(...args) {
      if (called) return;
      called = true;
      callback(...args);
    };
  }

  function createEBADF(fd) {
    const err = new Error("EBADF: bad file descriptor, close");
    err.code = "EBADF";
    err.errno = -9;
    err.syscall = "close";
    err.path = undefined;
    err.fd = fd;
    return err;
  }

  function open(path, flags, mode, callback) {
    if (typeof flags === "function") {
      callback = flags;
      flags = "r";
      mode = undefined;
    } else if (typeof mode === "function") {
      callback = mode;
      mode = undefined;
    }
    validateCallback(callback);
    const cb = onceCallback(callback);
    fsp.open(path, flags, mode).then(
      (handle) => {
        const fd = handle.fd;
        if (typeof fd !== "number" || !Number.isFinite(fd)) {
          const err = new Error("internal error: FileHandle.fd is not a number");
          handle.close().then(() => cb(err), () => cb(err));
          return;
        }
        if (handles.has(fd)) {
          const err = new Error("internal error: fd collision in fs.open registry");
          handle.close().then(() => cb(err), () => cb(err));
          return;
        }
        handles.set(fd, handle);
        cb(null, fd);
      },
      (error) => cb(error)
    );
  }

  function close(fd, callback) {
    validateCallback(callback);
    const cb = onceCallback(callback);
    const handle = handles.get(fd);
    if (!handle) {
      queueMicrotask(() => {
        try {
          closeSyncFn(fd);
          cb(null);
        } catch (err) {
          cb(err && err.code ? err : createEBADF(fd));
        }
      });
      return;
    }
    handles.delete(fd);
    handle.close().then(() => cb(null), (e) => cb(e));
  }

  function wrap(promiseFn) {
    return function(...args) {
      const callback = args[args.length - 1];
      validateCallback(callback);
      args.pop();
      const cb = onceCallback(callback);
      promiseFn(...args).then(
        (v) => v === undefined ? cb(null) : cb(null, v),
        (e) => cb(e)
      );
    };
  }

  function exists(path, callback) {
    validateCallback(callback);
    queueMicrotask(() => callback(existsSyncFn(path)));
  }

  function glob(pattern, options, callback) {
    if (typeof options === "function") {
      callback = options;
      options = undefined;
    }
    validateCallback(callback);
    const cb = onceCallback(callback);
    queueMicrotask(() => {
      try {
        const result = options === undefined ? globSyncFn(pattern) : globSyncFn(pattern, options);
        cb(null, result);
      } catch (err) {
        cb(err);
      }
    });
  }

  function _toUnixTimestamp(time) {
    if (typeof time === "number") return time;
    if (time instanceof Date) return time.getTime() / 1000;
    if (typeof time === "string" && isFinite(Number(time))) return Number(time);
    throw new TypeError("Cannot parse time: " + time);
  }

  return {
    open,
    close,
    exists,
    glob,
    _toUnixTimestamp,
    readFile: wrap((path, opts) => fsp.readFile(path, opts)),
    writeFile: wrap((path, data, opts) => fsp.writeFile(path, data, opts)),
    appendFile: wrap((path, data, opts) => fsp.appendFile(path, data, opts)),
    unlink: wrap((path) => fsp.unlink(path)),
    chmod: wrap((path, mode) => fsp.chmod(path, mode)),
    lchmod: wrap((path, mode) => fsp.lchmod(path, mode)),
    chown: wrap((path, uid, gid) => fsp.chown(path, uid, gid)),
    lchown: wrap((path, uid, gid) => fsp.lchown(path, uid, gid)),
    copyFile: wrap((src, dest, mode) => fsp.copyFile(src, dest, mode)),
    cp: wrap((src, dest, opts) => fsp.cp(src, dest, opts)),
    link: wrap((existing, target) => fsp.link(existing, target)),
    lutimes: wrap((path, a, m) => fsp.lutimes(path, a, m)),
    mkdir: wrap((path, opts) => fsp.mkdir(path, opts)),
    mkdtemp: wrap((prefix, opts) => fsp.mkdtemp(prefix, opts)),
    opendir: wrap((path, opts) => fsp.opendir(path, opts)),
    readlink: wrap((path, opts) => fsp.readlink(path, opts)),
    rename: wrap((oldPath, newPath) => fsp.rename(oldPath, newPath)),
    rm: wrap((path, opts) => fsp.rm(path, opts)),
    rmdir: wrap((path, opts) => fsp.rmdir(path, opts)),
    statfs: wrap((path, opts) => fsp.statfs(path, opts)),
    symlink: wrap((target, path, type) => fsp.symlink(target, path, type)),
    truncate: wrap((path, len) => fsp.truncate(path, len)),
    utimes: wrap((path, a, m) => fsp.utimes(path, a, m)),
  };
})"#,
            )?;
            let close_sync_val: Value = default.get("closeSync")?;
            let exists_sync_val: Value = default.get("existsSync")?;
            let glob_sync_val: Value = default.get("globSync")?;
            let cb_helpers: Object = cb_factory.call((
                promises.clone(),
                close_sync_val,
                exists_sync_val,
                glob_sync_val,
            ))?;

            for key in [
                "open",
                "close",
                "exists",
                "glob",
                "_toUnixTimestamp",
                "readFile",
                "writeFile",
                "appendFile",
                "unlink",
                "chmod",
                "lchmod",
                "chown",
                "lchown",
                "copyFile",
                "cp",
                "link",
                "lutimes",
                "mkdir",
                "mkdtemp",
                "opendir",
                "readlink",
                "rename",
                "rm",
                "rmdir",
                "statfs",
                "symlink",
                "truncate",
                "utimes",
            ] {
                default.set(key, cb_helpers.get::<_, Value>(key)?)?;
            }

            Ok(())
        })
    }
}

fn zeroed_stats_js<'js>(ctx: Ctx<'js>, bigint: Opt<bool>) -> Result<Value<'js>> {
    zeroed_stats(&ctx, bigint.0.unwrap_or(false))
}

fn exists_sync<'js>(ctx: Ctx<'js>, path: Value<'js>) -> bool {
    let Ok(resolved) = path_from_value(&ctx, path) else {
        return false;
    };
    std::fs::metadata(resolved).is_ok()
}

fn export_promises<'js>(ctx: &Ctx<'js>, exports: &Object<'js>) -> Result<()> {
    export_constants(ctx, exports)?;

    exports.set("access", Func::from(Async(access)))?;
    exports.set("open", Func::from(Async(open)))?;
    exports.set("readFile", Func::from(Async(read_file)))?;
    exports.set("writeFile", Func::from(Async(write_file)))?;
    exports.set("appendFile", Func::from(Async(append_file)))?;
    exports.set("rename", Func::from(Async(rename)))?;
    exports.set("readdir", Func::from(Async(read_dir)))?;
    exports.set("mkdir", Func::from(Async(mkdir)))?;
    exports.set("mkdtemp", Func::from(Async(mkdtemp)))?;
    exports.set("rm", Func::from(Async(rmfile)))?;
    exports.set("rmdir", Func::from(Async(rmdir)))?;
    exports.set("unlink", Func::from(Async(unlink)))?;
    exports.set("stat", Func::from(Async(stat_fn)))?;
    exports.set("lstat", Func::from(Async(lstat_fn)))?;
    exports.set("chmod", Func::from(Async(chmod)))?;
    exports.set("lchmod", Func::from(Async(lchmod)))?;
    exports.set("symlink", Func::from(Async(symlink)))?;
    exports.set("realpath", Func::from(Async(realpath_promises)))?;

    exports.set("link", Func::from(Async(fs_extra::link)))?;
    exports.set("readlink", Func::from(Async(readlink_async)))?;
    exports.set("truncate", Func::from(Async(fs_extra::truncate)))?;
    exports.set("chown", Func::from(Async(fs_extra::chown)))?;
    exports.set("lchown", Func::from(Async(fs_extra::lchown)))?;
    exports.set("copyFile", Func::from(Async(fs_extra::copy_file)))?;
    exports.set("utimes", Func::from(Async(utimes_async)))?;
    exports.set("lutimes", Func::from(Async(lutimes_async)))?;
    exports.set("opendir", Func::from(Async(opendir)))?;
    exports.set("glob", glob_entries(ctx)?)?;
    exports.set("statfs", Func::from(Async(statfs)))?;

    let watch_fn = Function::new(ctx.clone(), watch)?;
    let watch_factory: Function = ctx.eval(FS_PROMISES_WATCH_JS)?;
    let fsp_watch: Value = watch_factory.call((watch_fn,))?;
    exports.set("watch", fsp_watch)?;

    // Build `cp` with sync + async helpers using this promises object + sync primitives.
    let cp_api = Object::new(ctx.clone())?;
    for key in [
        "stat", "lstat", "mkdir", "readdir", "copyFile", "readlink", "symlink", "utimes",
        "lutimes", "rm", "unlink",
    ] {
        cp_api.set(key, exports.get::<_, Value>(key)?)?;
    }
    let cp_factory: Function = ctx.eval(FS_CP_HELPER_JS)?;
    let cp_fns: Object = cp_factory.call((cp_api,))?;
    exports.set("cp", cp_fns.get::<_, Value>("copyAsync")?)?;

    Ok(())
}

/// `fs.openAsBlob(path[, options])` -> `Promise<Blob>`.
async fn open_as_blob<'js>(
    ctx: Ctx<'js>,
    path: Value<'js>,
    options: Opt<Object<'js>>,
) -> Result<Value<'js>> {
    let resolved = path_from_value(&ctx, path)?;
    lexe_utils::permissions::ensure_fs_read(&ctx, &resolved)?;
    let bytes = tokio::fs::read(&resolved)
        .await
        .map_err(|err| crate::errors::throw_fs_error(&ctx, err, "open", &resolved))?;
    let mime_type = options
        .0
        .and_then(|o| o.get::<_, Option<String>>("type").ok().flatten())
        .unwrap_or_default();

    let blob_ctor_val: Value = ctx.globals().get("Blob")?;
    let Some(blob_ctor) = blob_ctor_val.as_constructor().cloned() else {
        return Err(Exception::throw_message(&ctx, "Blob is not registered"));
    };
    let buf = lexe_buffer::Buffer(bytes).into_js(&ctx)?;
    let parts = Array::new(ctx.clone())?;
    parts.set(0, buf)?;
    let blob_opts = Object::new(ctx.clone())?;
    if !mime_type.is_empty() {
        blob_opts.set("type", mime_type)?;
    }
    blob_ctor.construct((parts, blob_opts))
}

/// `fs.promises.glob(pattern[, options])` — an async iterator over matches.
fn glob_entries<'js>(ctx: &Ctx<'js>) -> Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        |ctx: Ctx<'js>, pattern: Value<'js>, options: Opt<Value<'js>>| -> Result<Value<'js>> {
            glob_run(&ctx, pattern, options)
        },
    )
}

fn parse_glob_patterns<'js>(ctx: &Ctx<'js>, pattern: Value<'js>) -> Result<Vec<String>> {
    if let Some(s) = pattern.as_string() {
        return Ok(vec![s.to_string()?]);
    }
    if let Some(arr) = pattern.as_array() {
        let mut out = Vec::with_capacity(arr.len());
        for item in arr.iter::<String>() {
            out.push(item?);
        }
        return Ok(out);
    }
    Err(Exception::throw_type(
        ctx,
        "The \"pattern\" argument must be a string or string[]",
    ))
}

enum GlobExclude<'js> {
    None,
    Patterns(Vec<fs_extra::GlobMatcher>),
    Func(Function<'js>),
}

fn glob_collect<'js>(
    ctx: &Ctx<'js>,
    pattern: Value<'js>,
    options: Opt<Value<'js>>,
) -> Result<Array<'js>> {
    let patterns = parse_glob_patterns(ctx, pattern)?;
    let mut cwd_explicit = false;
    let mut cwd = None;
    let mut with_file_types = false;
    let mut exclude = GlobExclude::None;

    if let Some(opts) = options.0.as_ref().and_then(|v| v.as_object()) {
        if let Ok(Some(cwd_val)) = opts.get::<_, Option<Value>>("cwd") {
            if !cwd_val.is_null() && !cwd_val.is_undefined() {
                cwd = Some(path_from_value(ctx, cwd_val)?);
                cwd_explicit = true;
            }
        }
        if let Ok(Some(wft)) = opts.get::<_, Option<bool>>("withFileTypes") {
            with_file_types = wft;
        }
        if let Ok(Some(excl)) = opts.get::<_, Option<Value>>("exclude") {
            if let Some(f) = excl.as_function() {
                exclude = GlobExclude::Func(f.clone());
            } else if let Some(arr) = excl.as_array() {
                let mut matchers = Vec::with_capacity(arr.len());
                for item in arr.iter::<String>() {
                    matchers.push(fs_extra::glob_pattern_to_matcher(&item?));
                }
                exclude = GlobExclude::Patterns(matchers);
            } else if !excl.is_null() && !excl.is_undefined() {
                return Err(Exception::throw_type(
                    ctx,
                    "The \"options.exclude\" property must be a function or string[]",
                ));
            }
        }
    }

    let root_str = cwd
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| ".".to_string());
    let root_path = std::path::PathBuf::from(&root_str);
    let display_root = if cwd_explicit {
        root_str.clone()
    } else {
        ".".to_string()
    };

    let matchers: Vec<fs_extra::GlobMatcher> = patterns
        .iter()
        .map(|p| fs_extra::glob_pattern_to_matcher(p))
        .collect();
    let mut seen = HashSet::new();
    let mut matches: Vec<(String, std::path::PathBuf)> = Vec::new();
    collect_glob_matches(
        ctx,
        &root_path,
        &root_path,
        &display_root,
        &matchers,
        &exclude,
        with_file_types,
        &mut seen,
        &mut matches,
    )?;
    matches.sort_by(|a, b| a.0.cmp(&b.0));

    let array = Array::new(ctx.clone())?;
    for (index, (rel, full_path)) in matches.into_iter().enumerate() {
        if with_file_types {
            let meta = std::fs::symlink_metadata(&full_path)
                .map_err(|err| crate::errors::throw_fs_error(ctx, err, "lstat", &rel))?;
            let dirent = build_glob_dirent(ctx, meta, &rel, &display_root)?;
            array.set(index, dirent)?;
        } else {
            array.set(index, rel)?;
        }
    }
    Ok(array)
}

fn build_glob_dirent<'js>(
    ctx: &Ctx<'js>,
    metadata: std::fs::Metadata,
    relative: &str,
    display_root: &str,
) -> Result<Value<'js>> {
    let p = std::path::Path::new(relative);
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| relative.to_string());
    let parent_rel = p
        .parent()
        .map(|par| par.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let parent_path = if parent_rel.is_empty() {
        display_root.to_string()
    } else if display_root == "." {
        parent_rel
    } else if display_root.ends_with('/') {
        format!("{display_root}{parent_rel}")
    } else {
        format!("{display_root}/{parent_rel}")
    };

    let dirent = Class::instance(ctx.clone(), Dirent::new(metadata))?;
    dirent.set(PredefinedAtom::Name, name)?;
    dirent.set("parentPath", parent_path)?;
    dirent.into_js(ctx)
}

fn glob_sync<'js>(
    ctx: Ctx<'js>,
    pattern: Value<'js>,
    options: Opt<Value<'js>>,
) -> Result<Array<'js>> {
    glob_collect(&ctx, pattern, options)
}

fn glob_run<'js>(
    ctx: &Ctx<'js>,
    pattern: Value<'js>,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let array = glob_collect(ctx, pattern, options)?;
    // Node's `glob` returns an async-iterable `Glob`, so wrap the matches.
    let make: Function = ctx.eval(
        r#"(function (paths) {
  return {
    [Symbol.asyncIterator]() {
      let i = 0;
      return {
        next: () => Promise.resolve(
          i < paths.length
            ? { value: paths[i++], done: false }
            : { value: undefined, done: true }
        ),
      };
    },
  };
})"#,
    )?;
    let out: Value = make.call((array,))?;
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn collect_glob_matches<'js>(
    ctx: &Ctx<'js>,
    root: &std::path::Path,
    dir: &std::path::Path,
    display_root: &str,
    matchers: &[fs_extra::GlobMatcher],
    exclude: &GlobExclude<'js>,
    with_file_types: bool,
    seen: &mut HashSet<String>,
    out: &mut Vec<(String, std::path::PathBuf)>,
) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

        let excluded = match exclude {
            GlobExclude::None => false,
            GlobExclude::Patterns(pats) => pats.iter().any(|m| m.is_match(&relative)),
            GlobExclude::Func(f) => {
                if with_file_types {
                    let meta = entry
                        .metadata()
                        .or_else(|_| std::fs::symlink_metadata(&path));
                    if let Ok(m) = meta {
                        let dirent = build_glob_dirent(ctx, m, &relative, display_root)?;
                        f.call::<_, Value>((dirent,))?.as_bool().unwrap_or(false)
                    } else {
                        false
                    }
                } else {
                    f.call::<_, Value>((relative.clone(),))?
                        .as_bool()
                        .unwrap_or(false)
                }
            },
        };
        if excluded {
            continue;
        }

        if matchers.iter().any(|m| m.is_match(&relative)) && seen.insert(relative.clone()) {
            out.push((relative, path.clone()));
        }
        if is_dir {
            collect_glob_matches(
                ctx,
                root,
                &path,
                display_root,
                matchers,
                exclude,
                with_file_types,
                seen,
                out,
            )?;
        }
    }
    Ok(())
}

fn statfs_impl<'js>(ctx: &Ctx<'js>, path: &str, bigint: bool) -> Result<Object<'js>> {
    #[cfg(unix)]
    {
        let c_path =
            std::ffi::CString::new(path).or_throw_msg(ctx, "statfs: path contains a NUL byte")?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c_path` is valid and `stat` is a correctly sized out-buffer.
        let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
        if rc != 0 {
            return Err(crate::errors::throw_fs_error(
                ctx,
                std::io::Error::last_os_error(),
                "statfs",
                path,
            ));
        }
        let obj = Object::new(ctx.clone())?;
        #[allow(clippy::unnecessary_cast)]
        if bigint {
            obj.set("type", BigInt::from_u64(ctx.clone(), 0)?)?;
            obj.set("bsize", BigInt::from_u64(ctx.clone(), stat.f_bsize as u64)?)?;
            obj.set(
                "blocks",
                BigInt::from_u64(ctx.clone(), stat.f_blocks as u64)?,
            )?;
            obj.set("bfree", BigInt::from_u64(ctx.clone(), stat.f_bfree as u64)?)?;
            obj.set(
                "bavail",
                BigInt::from_u64(ctx.clone(), stat.f_bavail as u64)?,
            )?;
            obj.set("files", BigInt::from_u64(ctx.clone(), stat.f_files as u64)?)?;
            obj.set("ffree", BigInt::from_u64(ctx.clone(), stat.f_ffree as u64)?)?;
        } else {
            obj.set("type", 0.0)?;
            obj.set("bsize", stat.f_bsize as f64)?;
            obj.set("blocks", stat.f_blocks as f64)?;
            obj.set("bfree", stat.f_bfree as f64)?;
            obj.set("bavail", stat.f_bavail as f64)?;
            obj.set("files", stat.f_files as f64)?;
            obj.set("ffree", stat.f_ffree as f64)?;
        }
        Ok(obj)
    }
    #[cfg(not(unix))]
    {
        let _ = (bigint, BigInt::from_u64);
        Err(Exception::throw_message(
            ctx,
            &format!("statfs {path}: ENOSYS"),
        ))
    }
}

fn parse_statfs_bigint(options: Option<Value<'_>>) -> bool {
    options
        .as_ref()
        .and_then(|v| v.as_object())
        .and_then(|o| o.get::<_, Option<bool>>("bigint").ok().flatten())
        .unwrap_or(false)
}

/// `fs.promises.statfs(path[, options])` — filesystem statistics.
async fn statfs<'js>(ctx: Ctx<'js>, path: String, options: Opt<Value<'js>>) -> Result<Object<'js>> {
    statfs_impl(&ctx, &path, parse_statfs_bigint(options.0))
}

/// `fs.statfsSync(path[, options])`.
fn statfs_sync<'js>(ctx: Ctx<'js>, path: String, options: Opt<Value<'js>>) -> Result<Object<'js>> {
    statfs_impl(&ctx, &path, parse_statfs_bigint(options.0))
}

/// `readlink` takes `options.encoding === "buffer"` to return a Buffer.
fn readlink_async<'js>(
    ctx: Ctx<'js>,
    path: String,
    options: Opt<Value<'js>>,
) -> impl std::future::Future<Output = Result<Value<'js>>> + 'js {
    let as_buffer = options
        .0
        .as_ref()
        .and_then(|v| v.as_object())
        .and_then(|o| o.get::<_, Option<String>>("encoding").ok().flatten())
        .map(|enc| enc == "buffer")
        .unwrap_or(false);
    fs_extra::readlink(ctx, path, as_buffer)
}

fn readlink_sync_call<'js>(
    ctx: Ctx<'js>,
    path: String,
    options: Opt<Value<'js>>,
) -> Result<Value<'js>> {
    let as_buffer = options
        .0
        .as_ref()
        .and_then(|v| v.as_object())
        .and_then(|o| o.get::<_, Option<String>>("encoding").ok().flatten())
        .map(|enc| enc == "buffer")
        .unwrap_or(false);
    fs_extra::readlink_sync(ctx, path, as_buffer)
}

fn utimes_sync<'js>(
    ctx: Ctx<'js>,
    path: String,
    atime: Value<'js>,
    mtime: Value<'js>,
) -> Result<()> {
    let a = utimes_pair(&ctx, atime)?;
    let m = utimes_pair(&ctx, mtime)?;
    fs_extra::utimes_sync(ctx, path, a, m)
}

fn lutimes_sync<'js>(
    ctx: Ctx<'js>,
    path: String,
    atime: Value<'js>,
    mtime: Value<'js>,
) -> Result<()> {
    let a = utimes_pair(&ctx, atime)?;
    let m = utimes_pair(&ctx, mtime)?;
    fs_extra::lutimes_sync(ctx, path, a, m)
}

/// `utimes`/`lutimes` accept numbers (seconds), strings, or `Date` for each time.
fn utimes_pair<'js>(ctx: &Ctx<'js>, t: Value<'js>) -> Result<f64> {
    fd_sync::utimes_value_to_secs(ctx, t)
}

fn utimes_async<'js>(
    ctx: Ctx<'js>,
    path: String,
    atime: Value<'js>,
    mtime: Value<'js>,
) -> impl std::future::Future<Output = Result<()>> + 'js {
    let result = utimes_pair(&ctx, atime).and_then(|a| utimes_pair(&ctx, mtime).map(|m| (a, m)));
    async move {
        let (a, m) = result?;
        fs_extra::utimes(ctx, path, a, m).await
    }
}

fn lutimes_async<'js>(
    ctx: Ctx<'js>,
    path: String,
    atime: Value<'js>,
    mtime: Value<'js>,
) -> impl std::future::Future<Output = Result<()>> + 'js {
    let result = utimes_pair(&ctx, atime).and_then(|a| utimes_pair(&ctx, mtime).map(|m| (a, m)));
    async move {
        let (a, m) = result?;
        fs_extra::lutimes(ctx, path, a, m).await
    }
}

/// `fs.promises.opendir(path)` — a `Dir` over the directory's entries.
async fn opendir<'js>(ctx: Ctx<'js>, path: String) -> Result<Value<'js>> {
    let opts = Object::new(ctx.clone())?;
    opts.set("withFileTypes", true)?;
    let entries: Value = read_dir(ctx.clone(), path.clone(), Opt(Some(opts)))
        .await?
        .into_js(&ctx)?;
    let ctor: Value = ctx.globals().get("__lexeFsDirClass")?;
    let Some(ctor) = ctor.as_constructor().cloned() else {
        return Err(Exception::throw_message(&ctx, "fs.Dir is not registered"));
    };
    ctor.construct((path, entries))
}

/// `fs.opendirSync(path)`.
fn opendir_sync<'js>(ctx: Ctx<'js>, path: String) -> Result<Value<'js>> {
    let opts = Object::new(ctx.clone())?;
    opts.set("withFileTypes", true)?;
    let entries: Value =
        read_dir_sync(ctx.clone(), path.clone(), Opt(Some(opts)))?.into_js(&ctx)?;
    let ctor: Value = ctx.globals().get("__lexeFsDirClass")?;
    let Some(ctor) = ctor.as_constructor().cloned() else {
        return Err(Exception::throw_message(&ctx, "fs.Dir is not registered"));
    };
    ctor.construct((path, entries))
}

pub use self::constants::{create_constants, visit_fs_constants};

fn export_constants<'js>(ctx: &Ctx<'js>, exports: &Object<'js>) -> Result<()> {
    exports.set("constants", create_constants(ctx)?)?;
    Ok(())
}

impl From<FsModule> for ModuleInfo<FsModule> {
    fn from(val: FsModule) -> Self {
        ModuleInfo {
            name: "fs",
            module: val,
        }
    }
}
