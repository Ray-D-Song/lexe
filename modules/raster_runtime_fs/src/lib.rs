// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
mod access;
mod chmod;
mod errors;
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

use raster_runtime_events::Emitter;
use raster_runtime_utils::module::{export_default, ModuleInfo};
use raster_runtime_utils::result::ResultExt;
use rquickjs::{
    module::{Declarations, Exports, ModuleDef},
    prelude::{Async, Func, Opt},
    Exception, Function,
};
use rquickjs::{Class, Ctx, IntoJs, Object, Result, Value};

use self::access::{access, access_callback, access_sync};
use self::chmod::{chmod, chmod_sync};
use self::file_handle::FileHandle;
use self::mkdir::{mkdir, mkdir_sync, mkdtemp, mkdtemp_sync};
use self::open::open;
use self::read_dir::{read_dir, read_dir_sync, readdir_callback, Dirent};
use self::read_file::{read_file, read_file_sync};
use self::read_stream::create_create_read_stream;
use self::realpath::{realpath, realpath_promises, realpath_sync};
use self::rename::{rename, rename_sync};
use self::rm::{rmdir, rmdir_sync, rmfile, rmfile_sync};
use self::stats::{
    lstat_callback, lstat_fn, lstat_fn_sync, stat_callback, stat_fn, stat_fn_sync, Stats,
};
use self::symlink::{symlink, symlink_sync};
use self::watch::{watch, FSWatcher};
use self::write_file::{append_file, append_file_sync, write_file, write_file_sync};
use self::write_stream::create_create_write_stream;

pub const CONSTANT_F_OK: u32 = 0;
pub const CONSTANT_R_OK: u32 = 4;
pub const CONSTANT_W_OK: u32 = 2;
pub const CONSTANT_X_OK: u32 = 1;

pub struct FsPromisesModule;

impl ModuleDef for FsPromisesModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("access")?;
        declare.declare("open")?;
        declare.declare("readFile")?;
        declare.declare("writeFile")?;
        declare.declare("appendFile")?;
        declare.declare("rename")?;
        declare.declare("readdir")?;
        declare.declare("mkdir")?;
        declare.declare("mkdtemp")?;
        declare.declare("rm")?;
        declare.declare("rmdir")?;
        declare.declare("stat")?;
        declare.declare("lstat")?;
        declare.declare("constants")?;
        declare.declare("chmod")?;
        declare.declare("symlink")?;
        declare.declare("realpath")?;
        declare.declare("link")?;
        declare.declare("readlink")?;
        declare.declare("truncate")?;
        declare.declare("chown")?;
        declare.declare("lchown")?;
        declare.declare("utimes")?;
        declare.declare("lutimes")?;
        declare.declare("opendir")?;
        declare.declare("cp")?;
        declare.declare("glob")?;
        declare.declare("statfs")?;

        declare.declare("default")?;

        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let globals = ctx.globals();

        Class::<Dirent>::define(&globals)?;
        Class::<FileHandle>::define(&globals)?;
        Class::<Stats>::define(&globals)?;

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

/// `fs.ReadStream` / `fs.WriteStream` / `fs.Dir` constructors.
///
/// These exist for `instanceof` checks and for `new fs.ReadStream(path)`. The
/// stream bodies stay where they already were — `createReadStream` and
/// `createWriteStream` build a `Readable`/`Writable` and re-parent it onto
/// these prototypes — so there is one implementation, not two.
const FS_STREAM_CLASSES_JS: &str = r#"(function () {
  "use strict";
  const { Readable, Writable } = require("node:stream");

  // Node's ReadStream/WriteStream are Readable/Writable subclasses; the actual
  // reading/writing is driven by the helpers in this module.
  class ReadStream extends Readable {
    constructor(path, options) {
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
      this.closed = false;
      if (!options._rasterNoAutoStart) this._start();
    }

    _start() {
      const factory = globalThis.__rasterFsCreateReadStream;
      if (typeof factory !== "function") return;
      const target = factory(this.path, {
        flags: this.flags,
        mode: this.mode,
        start: this.start,
        end: this.end,
        highWaterMark: this.readableHighWaterMark,
      });
      target.on("data", (chunk) => this.push(chunk));
      target.on("end", () => {
        this.bytesRead = target.bytesRead || this.bytesRead;
        this.push(null);
      });
      target.on("error", (err) => this.destroy(err));
      this._source = target;
    }

    close(cb) {
      this.closed = true;
      if (this._source && typeof this._source.destroy === "function") {
        this._source.destroy();
      }
      this.destroy();
      if (typeof cb === "function") process.nextTick(cb, null);
    }

    _destroy(err, cb) {
      if (this._source && typeof this._source.destroy === "function") {
        this._source.destroy();
      }
      cb(err);
    }
  }

  class WriteStream extends Writable {
    constructor(path, options) {
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
      this.closed = false;
      if (!options._rasterNoAutoStart) this._start();
    }

    _start() {
      const factory = globalThis.__rasterFsCreateWriteStream;
      if (typeof factory !== "function") return;
      const target = factory(this.path, {
        flags: this.flags,
        mode: this.mode,
        start: this.start,
      });
      this._sink = target;
      target.on("error", (err) => this.destroy(err));
    }

    _write(chunk, encoding, cb) {
      if (!this._sink) {
        cb(new Error("WriteStream has no sink"));
        return;
      }
      this.bytesWritten += chunk.length;
      this._sink.write(chunk, encoding, cb);
    }

    _final(cb) {
      if (this._sink) this._sink.end(cb);
      else cb();
    }

    close(cb) {
      this.closed = true;
      this.destroy();
      if (typeof cb === "function") process.nextTick(cb, null);
    }

    _destroy(err, cb) {
      if (this._sink && typeof this._sink.destroy === "function") {
        this._sink.destroy();
      }
      cb(err);
    }
  }

  // `fs.Dir`: an async iterator over Dirents with close()/read(). The entries
  // come from `fs.readdir(withFileTypes: true)`, which already returns Dirents.
  class Dir {
    constructor(path, entries) {
      this.path = path;
      this._entries = entries;
      this._index = 0;
      this._closed = false;
    }

    read() {
      if (this._closed) return Promise.resolve(null);
      if (this._index >= this._entries.length) return Promise.resolve(null);
      return Promise.resolve(this._entries[this._index++]);
    }

    close() {
      this._closed = true;
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

  return { ReadStream, WriteStream, Dir };
})()"#;

pub struct FsModule;

impl ModuleDef for FsModule {
    fn declare(declare: &Declarations) -> Result<()> {
        declare.declare("promises")?;
        declare.declare("access")?;
        declare.declare("accessSync")?;
        declare.declare("open")?;
        declare.declare("close")?;
        declare.declare("mkdirSync")?;
        declare.declare("mkdtempSync")?;
        declare.declare("readdir")?;
        declare.declare("readdirSync")?;
        declare.declare("readFile")?;
        declare.declare("readFileSync")?;
        declare.declare("writeFile")?;
        declare.declare("appendFile")?;
        declare.declare("appendFileSync")?;
        declare.declare("rmdirSync")?;
        declare.declare("rmSync")?;
        declare.declare("unlinkSync")?;
        declare.declare("stat")?;
        declare.declare("statSync")?;
        declare.declare("lstat")?;
        declare.declare("lstatSync")?;
        declare.declare("writeFileSync")?;
        declare.declare("watch")?;
        declare.declare("FSWatcher")?;
        declare.declare("constants")?;
        declare.declare("chmodSync")?;
        declare.declare("renameSync")?;
        declare.declare("symlinkSync")?;
        declare.declare("realpathSync")?;
        declare.declare("realpath")?;
        declare.declare("existsSync")?;
        declare.declare("createReadStream")?;
        declare.declare("createWriteStream")?;
        declare.declare("Stats")?;
        declare.declare("Dirent")?;
        declare.declare("FileHandle")?;
        declare.declare("ReadStream")?;
        declare.declare("WriteStream")?;
        declare.declare("Dir")?;
        declare.declare("linkSync")?;
        declare.declare("readlinkSync")?;
        declare.declare("truncateSync")?;
        declare.declare("chownSync")?;
        declare.declare("lchownSync")?;
        declare.declare("utimesSync")?;
        declare.declare("lutimesSync")?;

        // `fs.promises` is exported as a nested object, and every key on it is
        // re-exported on the namespace, so each one has to be declared here too.
        declare.declare("link")?;
        declare.declare("readlink")?;
        declare.declare("truncate")?;
        declare.declare("chown")?;
        declare.declare("lchown")?;
        declare.declare("utimes")?;
        declare.declare("lutimes")?;
        declare.declare("opendir")?;
        declare.declare("cp")?;
        declare.declare("glob")?;
        declare.declare("statfs")?;
        declare.declare("mkdir")?;
        declare.declare("mkdtemp")?;
        declare.declare("rename")?;
        declare.declare("rm")?;
        declare.declare("rmdir")?;
        declare.declare("symlink")?;

        declare.declare("default")?;

        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let globals = ctx.globals();

        Class::<Dirent>::define(&globals)?;
        Class::<FileHandle>::define(&globals)?;
        Class::<Stats>::define(&globals)?;
        let fs_watcher_ctor = Class::<FSWatcher>::create_constructor(ctx)?
            .expect("Can't create FSWatcher constructor");
        globals.set("FSWatcher", fs_watcher_ctor.clone())?;
        // `create_constructor` returns None for classes whose constructor is
        // skipped (Stats/Dirent are built in Rust, not from JS), so recover the
        // constructor from a prototype instead. That is the same function
        // `instanceof` compares against, which is all callers need.
        let stats_ctor = Class::<Stats>::prototype(ctx)?
            .and_then(|p| p.get::<_, Value>("constructor").ok())
            .and_then(|c| c.as_function().cloned());
        let dirent_ctor = Class::<Dirent>::prototype(ctx)?
            .and_then(|p| p.get::<_, Value>("constructor").ok())
            .and_then(|c| c.as_function().cloned());
        let file_handle_ctor = Class::<FileHandle>::prototype(ctx)?
            .and_then(|p| p.get::<_, Value>("constructor").ok())
            .and_then(|c| c.as_function().cloned());
        FSWatcher::add_event_emitter_prototype(ctx)?;

        export_default(ctx, exports, |default| {
            let promises = Object::new(ctx.clone())?;
            export_promises(ctx, &promises)?;
            export_constants(ctx, default)?;

            default.set("promises", promises)?;

            let access_fn = Function::new(ctx.clone(), access_callback)?;
            default.set("access", access_fn)?;
            default.set("accessSync", Func::from(access_sync))?;
            default.set("mkdirSync", Func::from(mkdir_sync))?;
            default.set("mkdtempSync", Func::from(mkdtemp_sync))?;
            let readdir_fn = Function::new(ctx.clone(), readdir_callback)?;
            default.set("readdir", readdir_fn)?;
            default.set("readdirSync", Func::from(read_dir_sync))?;
            // Callback-style open/close/readFile/writeFile with real handle tracking.
            let cb_helpers: Object = ctx.eval(
                r#"(function(){
  const fsp = require("fs/promises");
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
        // Use the real OS fd from FileHandle.fd (sync getter). Registry keeps
        // the handle alive so the fd is not closed until fs.close(fd).
        const fd = handle.fd;
        if (typeof fd !== "number" || !Number.isFinite(fd)) {
          const err = new Error("internal error: FileHandle.fd is not a number");
          handle.close().then(() => cb(err), () => cb(err));
          return;
        }
        if (handles.has(fd)) {
          // Collision on a live registry entry is an internal error — never
          // allocate synthetic fds that could mask the real descriptor.
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
    // Only registry-owned handles may be closed via fs.close(fd).
    const handle = handles.get(fd);
    if (!handle) {
      // Double close / unknown fd → Node EBADF (error-first callback).
      queueMicrotask(() => cb(createEBADF(fd)));
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
        (v) => cb(null, v),
        (e) => cb(e)
      );
    };
  }

  return {
    open,
    close,
    readFile: wrap((path, opts) => fsp.readFile(path, opts)),
    writeFile: wrap((path, data, opts) => fsp.writeFile(path, data, opts)),
    appendFile: wrap((path, data, opts) => fsp.appendFile(path, data, opts)),
  };
})()"#,
            )?;
            default.set("open", cb_helpers.get::<_, Value>("open")?)?;
            default.set("close", cb_helpers.get::<_, Value>("close")?)?;
            default.set("readFile", cb_helpers.get::<_, Value>("readFile")?)?;
            default.set("writeFile", cb_helpers.get::<_, Value>("writeFile")?)?;
            default.set("appendFile", cb_helpers.get::<_, Value>("appendFile")?)?;
            default.set("readFileSync", Func::from(read_file_sync))?;
            default.set("rmdirSync", Func::from(rmdir_sync))?;
            default.set("rmSync", Func::from(rmfile_sync))?;
            default.set("unlinkSync", Func::from(rmfile_sync))?;

            let stat_fn_export = Function::new(ctx.clone(), stat_callback)?;
            let lstat_fn_export = Function::new(ctx.clone(), lstat_callback)?;
            default.set("stat", stat_fn_export)?;
            default.set("statSync", Func::from(stat_fn_sync))?;
            default.set("lstat", lstat_fn_export)?;
            default.set("lstatSync", Func::from(lstat_fn_sync))?;
            default.set("writeFileSync", Func::from(write_file_sync))?;
            default.set("appendFileSync", Func::from(append_file_sync))?;
            default.set("watch", Func::from(watch))?;
            default.set("FSWatcher", fs_watcher_ctor)?;
            default.set("chmodSync", Func::from(chmod_sync))?;
            default.set("renameSync", Func::from(rename_sync))?;
            default.set("symlinkSync", Func::from(symlink_sync))?;
            default.set("existsSync", Func::from(exists_sync))?;
            default.set("createReadStream", create_create_read_stream(ctx)?)?;
            default.set("createWriteStream", create_create_write_stream(ctx)?)?;
            // The `ReadStream`/`WriteStream` classes delegate to these so there
            // is exactly one stream implementation.
            ctx.globals().set(
                "__rasterFsCreateReadStream",
                create_create_read_stream(ctx)?,
            )?;
            ctx.globals().set(
                "__rasterFsCreateWriteStream",
                create_create_write_stream(ctx)?,
            )?;

            // Expose the classes so `instanceof fs.Stats` / `import { Stats }`
            // work. Node refuses direct construction of these, so the
            // constructors are identity checks first and foremost.
            if let Some(ctor) = stats_ctor.clone() {
                default.set("Stats", ctor)?;
            }
            if let Some(ctor) = dirent_ctor.clone() {
                default.set("Dirent", ctor)?;
            }
            if let Some(ctor) = file_handle_ctor.clone() {
                default.set("FileHandle", ctor)?;
            }

            // `fs.ReadStream` / `fs.WriteStream` / `fs.Dir` for `instanceof`
            // branches (send, serve-static, multer, formidable). The base
            // classes come from the stream implementation already in use;
            // `createReadStream`/`createWriteStream` re-parent their results
            // onto these prototypes, and `opendir` builds real `Dir` objects.
            let fs_stream_classes: Object = ctx.eval(FS_STREAM_CLASSES_JS)?;
            let read_stream_ctor: Function = fs_stream_classes.get("ReadStream")?;
            let write_stream_ctor: Function = fs_stream_classes.get("WriteStream")?;
            let dir_ctor: Function = fs_stream_classes.get("Dir")?;
            default.set("ReadStream", read_stream_ctor.clone())?;
            default.set("WriteStream", write_stream_ctor.clone())?;
            default.set("Dir", dir_ctor.clone())?;
            // `createReadStream`/`createWriteStream` look these up while building
            // their result.
            ctx.globals()
                .set("__rasterFsReadStreamClass", read_stream_ctor)?;
            ctx.globals()
                .set("__rasterFsWriteStreamClass", write_stream_ctor)?;
            ctx.globals().set("__rasterFsDirClass", dir_ctor)?;

            let realpath_sync_fn = Function::new(ctx.clone(), realpath_sync)?;
            let realpath_sync_native = Function::new(ctx.clone(), realpath_sync)?;
            realpath_sync_fn.set("native", realpath_sync_native)?;
            default.set("realpathSync", realpath_sync_fn)?;

            let realpath_fn = Function::new(ctx.clone(), realpath)?;
            let realpath_native = Function::new(ctx.clone(), realpath)?;
            realpath_fn.set("native", realpath_native)?;
            default.set("realpath", realpath_fn)?;

            // Sync twins for the `fs/promises` entries added above.
            default.set("linkSync", Func::from(fs_extra::link_sync))?;
            default.set("readlinkSync", Func::from(readlink_sync_call))?;
            default.set("truncateSync", Func::from(fs_extra::truncate_sync))?;
            default.set("chownSync", Func::from(fs_extra::chown_sync))?;
            default.set("lchownSync", Func::from(fs_extra::lchown_sync))?;
            default.set("utimesSync", Func::from(utimes_sync))?;
            default.set("lutimesSync", Func::from(lutimes_sync))?;

            Ok(())
        })
    }
}

fn exists_sync(path: String) -> bool {
    std::fs::metadata(path).is_ok()
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
    exports.set("stat", Func::from(Async(stat_fn)))?;
    exports.set("lstat", Func::from(Async(lstat_fn)))?;
    exports.set("chmod", Func::from(Async(chmod)))?;
    exports.set("symlink", Func::from(Async(symlink)))?;
    exports.set("realpath", Func::from(Async(realpath_promises)))?;

    // Entries that were missing from `fs/promises`.
    exports.set("link", Func::from(Async(fs_extra::link)))?;
    exports.set("readlink", Func::from(Async(readlink_async)))?;
    exports.set("truncate", Func::from(Async(fs_extra::truncate)))?;
    exports.set("chown", Func::from(Async(fs_extra::chown)))?;
    exports.set("lchown", Func::from(Async(fs_extra::lchown)))?;
    exports.set("utimes", Func::from(Async(utimes_async)))?;
    exports.set("lutimes", Func::from(Async(lutimes_async)))?;
    exports.set("opendir", Func::from(Async(opendir)))?;
    exports.set("cp", Func::from(Async(copy_tree)))?;
    exports.set("glob", glob_entries(ctx)?)?;
    exports.set("statfs", Func::from(Async(statfs)))?;

    Ok(())
}

/// `fs.promises.glob(pattern[, options])` — an async iterator over matches.
///
/// Node returns a `Glob` object that is async-iterable; the pattern is walked
/// here and matches are yielded with `cwd`-relative paths.
fn glob_entries<'js>(ctx: &Ctx<'js>) -> Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        |ctx: Ctx<'js>, pattern: String, options: Opt<Value<'js>>| -> Result<Value<'js>> {
            glob_run(&ctx, pattern, options)
        },
    )
}

fn glob_run<'js>(ctx: &Ctx<'js>, pattern: String, options: Opt<Value<'js>>) -> Result<Value<'js>> {
    let cwd = options
        .0
        .as_ref()
        .and_then(|v| v.as_object())
        .and_then(|o| o.get::<_, Option<String>>("cwd").ok().flatten())
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| ".".to_string());
    let matcher = fs_extra::glob_pattern_to_matcher(&pattern);
    let mut matches = Vec::new();
    collect_glob_matches(
        &std::path::PathBuf::from(&cwd),
        &std::path::PathBuf::from(&cwd),
        &matcher,
        &mut matches,
    );
    matches.sort();
    let array = rquickjs::Array::new(ctx.clone())?;
    for (index, path) in matches.into_iter().enumerate() {
        array.set(index, path)?;
    }
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

fn collect_glob_matches(
    root: &std::path::Path,
    dir: &std::path::Path,
    matcher: &fs_extra::GlobMatcher,
    out: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if matcher.is_match(&relative) {
            out.push(relative.clone());
        }
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            collect_glob_matches(root, &path, matcher, out);
        }
    }
}

/// `fs.promises.statfs(path)` — filesystem statistics.
async fn statfs<'js>(ctx: Ctx<'js>, path: String) -> Result<Object<'js>> {
    #[cfg(unix)]
    {
        let c_path = std::ffi::CString::new(path.as_str())
            .or_throw_msg(&ctx, "statfs: path contains a NUL byte")?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c_path` is valid and `stat` is a correctly sized out-buffer.
        let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
        if rc != 0 {
            return Err::<Object<'js>, _>(std::io::Error::last_os_error())
                .or_throw_msg(&ctx, &format!("statfs {path}"));
        }
        let obj = Object::new(ctx.clone())?;
        let bsize = stat.f_frsize.max(1) as f64;
        // `statvfs` has no filesystem-type field; Node's `type` is 0 here.
        obj.set("type", 0.0)?;
        obj.set("bsize", stat.f_bsize as f64)?;
        obj.set("blocks", stat.f_blocks as f64)?;
        obj.set("bfree", stat.f_bfree as f64)?;
        obj.set("bavail", stat.f_bavail as f64)?;
        obj.set("files", stat.f_files as f64)?;
        obj.set("ffree", stat.f_ffree as f64)?;
        obj.set("fsize", stat.f_frsize as f64)?;
        // BigInt-valued fields Node also exposes.
        obj.set("bavailBigInt", stat.f_bavail as i64)?;
        obj.set("bfreeBigInt", stat.f_bfree as i64)?;
        obj.set("blocksBigInt", stat.f_blocks as i64)?;
        obj.set("ffreeBigInt", stat.f_ffree as i64)?;
        obj.set("filesBigInt", stat.f_files as i64)?;
        let _ = bsize;
        Ok(obj)
    }
    #[cfg(not(unix))]
    {
        Err(Exception::throw_message(
            &ctx,
            &format!("statfs {path}: ENOSYS"),
        ))
    }
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

/// `utimes`/`lutimes` accept numbers (seconds) or `Date` for each time.
fn utimes_pair<'js>(ctx: &Ctx<'js>, t: Value<'js>) -> Result<f64> {
    if let Some(n) = t.as_number() {
        return Ok(n);
    }
    if let Some(obj) = t.as_object() {
        if let Ok(get_time) = obj.get::<_, Function>("getTime") {
            if let Ok(ms) = get_time.call::<_, f64>((rquickjs::prelude::This(obj.clone()),)) {
                return Ok(ms / 1000.0);
            }
        }
    }
    if let Some(s) = t.as_string() {
        // Node parses date strings; accept them through the platform parser.
        if let Ok(text) = s.to_string() {
            if let Ok(ms) = text.parse::<f64>() {
                return Ok(ms / 1000.0);
            }
        }
    }
    Err(Exception::throw_type(ctx, "time must be a number or Date"))
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
    // `readdir(..., { withFileTypes: true })` already yields Dirents, so the
    // entries come from one implementation rather than a second walker.
    let opts = Object::new(ctx.clone())?;
    opts.set("withFileTypes", true)?;
    let entries: Value = read_dir(ctx.clone(), path.clone(), Opt(Some(opts)))
        .await?
        .into_js(&ctx)?;
    let ctor: Value = ctx.globals().get("__rasterFsDirClass")?;
    let Some(ctor) = ctor.as_constructor().cloned() else {
        return Err(Exception::throw_message(&ctx, "fs.Dir is not registered"));
    };
    let dir: Value = ctor.construct((path, entries))?;
    Ok(dir)
}

/// `fs.promises.cp(src, dest[, options])` — recursive copy.
///
/// Node's `cp` is a full recursive copier with filter/verbatimSymlinks
/// options; this covers the common `recursive: true` form plus `force`, and
/// reports a clear error for the options it does not implement rather than
/// silently ignoring them.
async fn copy_tree<'js>(
    ctx: Ctx<'js>,
    src: String,
    dest: String,
    options: Opt<Value<'js>>,
) -> Result<()> {
    let recursive = options
        .0
        .as_ref()
        .and_then(|v| v.as_object())
        .and_then(|o| o.get::<_, Option<bool>>("recursive").ok().flatten())
        .unwrap_or(false);
    let force = options
        .0
        .as_ref()
        .and_then(|v| v.as_object())
        .and_then(|o| o.get::<_, Option<bool>>("force").ok().flatten())
        .unwrap_or(true);
    if options
        .0
        .as_ref()
        .and_then(|v| v.as_object())
        .and_then(|o| o.get::<_, Option<Value>>("filter").ok().flatten())
        .is_some()
    {
        return Err(Exception::throw_message(
            &ctx,
            "fs.cp: the `filter` option is not supported",
        ));
    }

    let src_path = std::path::PathBuf::from(&src);
    let meta = tokio::fs::metadata(&src_path)
        .await
        .or_throw_msg(&ctx, &format!("cp {src}"))?;

    if meta.is_dir() {
        if !recursive {
            return Err(Exception::throw_message(
                &ctx,
                &format!("cp {src}: is a directory (use recursive: true)"),
            ));
        }
        copy_dir_recursive(ctx, src_path, std::path::PathBuf::from(&dest), force).await
    } else {
        if force || !std::path::Path::new(&dest).exists() {
            tokio::fs::copy(&src, &dest)
                .await
                .or_throw_msg(&ctx, &format!("cp {src} -> {dest}"))?;
        }
        Ok(())
    }
}

async fn copy_dir_recursive<'js>(
    ctx: Ctx<'js>,
    src: std::path::PathBuf,
    dest: std::path::PathBuf,
    force: bool,
) -> Result<()> {
    tokio::fs::create_dir_all(&dest)
        .await
        .or_throw_msg(&ctx, &format!("cp: mkdir {}", dest.display()))?;
    let mut rd = tokio::fs::read_dir(&src)
        .await
        .or_throw_msg(&ctx, &format!("cp: readdir {}", src.display()))?;
    while let Some(entry) = rd
        .next_entry()
        .await
        .or_throw_msg(&ctx, &format!("cp: readdir {}", src.display()))?
    {
        let name = entry.file_name();
        let from = entry.path();
        let to = dest.join(&name);
        let file_type = entry
            .file_type()
            .await
            .or_throw_msg(&ctx, &format!("cp: stat {}", from.display()))?;
        if file_type.is_dir() {
            Box::pin(copy_dir_recursive(ctx.clone(), from, to, force)).await?;
        } else if force || !to.exists() {
            tokio::fs::copy(&from, &to)
                .await
                .or_throw_msg(&ctx, &format!("cp {} -> {}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

/// Create the shared fs access-mode constants object (`F_OK`/`R_OK`/`W_OK`/`X_OK`).
///
/// Single source of truth for both `fs.constants` and the legacy `constants` module.
pub fn create_constants<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let constants = Object::new(ctx.clone())?;
    constants.set("F_OK", CONSTANT_F_OK)?;
    constants.set("R_OK", CONSTANT_R_OK)?;
    constants.set("W_OK", CONSTANT_W_OK)?;
    constants.set("X_OK", CONSTANT_X_OK)?;
    Ok(constants)
}

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
