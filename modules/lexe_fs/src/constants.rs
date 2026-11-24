use rquickjs::{Ctx, Function, Object, Result};

use crate::{CONSTANT_F_OK, CONSTANT_R_OK, CONSTANT_W_OK, CONSTANT_X_OK};

const UV_FS_SYMLINK_DIR: i32 = 1;
const UV_FS_SYMLINK_JUNCTION: i32 = 2;

const UV_FS_COPYFILE_EXCL: i32 = 1;
const UV_FS_COPYFILE_FICLONE: i32 = 2;
const UV_FS_COPYFILE_FICLONE_FORCE: i32 = 4;

const UV_DIRENT_UNKNOWN: i32 = 0;
const UV_DIRENT_FILE: i32 = 1;
const UV_DIRENT_DIR: i32 = 2;
const UV_DIRENT_LINK: i32 = 3;
const UV_DIRENT_FIFO: i32 = 4;
const UV_DIRENT_SOCKET: i32 = 5;
const UV_DIRENT_CHAR: i32 = 6;
const UV_DIRENT_BLOCK: i32 = 7;

#[cfg(windows)]
const UV_FS_O_FILEMAP: i32 = 0x2000_0000;
#[cfg(not(windows))]
const UV_FS_O_FILEMAP: i32 = 0;

/// Visit every constant in `fs.constants` (`F_OK`/`R_OK`/`W_OK`/`X_OK`, `O_*`, `S_*`,
/// `COPYFILE_*`, `UV_FS_*`, and `UV_DIRENT_*`).
pub fn visit_fs_constants(mut f: impl FnMut(&'static str, i32) -> Result<()>) -> Result<()> {
    for (name, val) in [
        ("UV_FS_SYMLINK_DIR", UV_FS_SYMLINK_DIR),
        ("UV_FS_SYMLINK_JUNCTION", UV_FS_SYMLINK_JUNCTION),
        ("F_OK", CONSTANT_F_OK as i32),
        ("R_OK", CONSTANT_R_OK as i32),
        ("W_OK", CONSTANT_W_OK as i32),
        ("X_OK", CONSTANT_X_OK as i32),
        ("UV_FS_COPYFILE_EXCL", UV_FS_COPYFILE_EXCL),
        ("COPYFILE_EXCL", UV_FS_COPYFILE_EXCL),
        ("UV_FS_COPYFILE_FICLONE", UV_FS_COPYFILE_FICLONE),
        ("COPYFILE_FICLONE", UV_FS_COPYFILE_FICLONE),
        ("UV_FS_COPYFILE_FICLONE_FORCE", UV_FS_COPYFILE_FICLONE_FORCE),
        ("COPYFILE_FICLONE_FORCE", UV_FS_COPYFILE_FICLONE_FORCE),
        ("UV_DIRENT_UNKNOWN", UV_DIRENT_UNKNOWN),
        ("UV_DIRENT_FILE", UV_DIRENT_FILE),
        ("UV_DIRENT_DIR", UV_DIRENT_DIR),
        ("UV_DIRENT_LINK", UV_DIRENT_LINK),
        ("UV_DIRENT_FIFO", UV_DIRENT_FIFO),
        ("UV_DIRENT_SOCKET", UV_DIRENT_SOCKET),
        ("UV_DIRENT_CHAR", UV_DIRENT_CHAR),
        ("UV_DIRENT_BLOCK", UV_DIRENT_BLOCK),
        ("UV_FS_O_FILEMAP", UV_FS_O_FILEMAP),
    ] {
        f(name, val)?;
    }

    visit_platform_fs_constants(&mut f)
}

#[cfg(unix)]
#[allow(clippy::unnecessary_cast)]
fn visit_platform_fs_constants(f: &mut impl FnMut(&'static str, i32) -> Result<()>) -> Result<()> {
    for (name, val) in [
        ("O_RDONLY", libc::O_RDONLY as i32),
        ("O_WRONLY", libc::O_WRONLY as i32),
        ("O_RDWR", libc::O_RDWR as i32),
        ("O_CREAT", libc::O_CREAT as i32),
        ("O_EXCL", libc::O_EXCL as i32),
        ("O_NOCTTY", libc::O_NOCTTY as i32),
        ("O_TRUNC", libc::O_TRUNC as i32),
        ("O_APPEND", libc::O_APPEND as i32),
        ("O_NONBLOCK", libc::O_NONBLOCK as i32),
        ("O_SYNC", libc::O_SYNC as i32),
        ("O_DIRECTORY", libc::O_DIRECTORY as i32),
        ("O_NOFOLLOW", libc::O_NOFOLLOW as i32),
        ("S_IFMT", libc::S_IFMT as i32),
        ("S_IFREG", libc::S_IFREG as i32),
        ("S_IFDIR", libc::S_IFDIR as i32),
        ("S_IFCHR", libc::S_IFCHR as i32),
        ("S_IFBLK", libc::S_IFBLK as i32),
        ("S_IFIFO", libc::S_IFIFO as i32),
        ("S_IFLNK", libc::S_IFLNK as i32),
        ("S_IFSOCK", libc::S_IFSOCK as i32),
        ("S_IRWXU", libc::S_IRWXU as i32),
        ("S_IRUSR", libc::S_IRUSR as i32),
        ("S_IWUSR", libc::S_IWUSR as i32),
        ("S_IXUSR", libc::S_IXUSR as i32),
        ("S_IRWXG", libc::S_IRWXG as i32),
        ("S_IRGRP", libc::S_IRGRP as i32),
        ("S_IWGRP", libc::S_IWGRP as i32),
        ("S_IXGRP", libc::S_IXGRP as i32),
        ("S_IRWXO", libc::S_IRWXO as i32),
        ("S_IROTH", libc::S_IROTH as i32),
        ("S_IWOTH", libc::S_IWOTH as i32),
        ("S_IXOTH", libc::S_IXOTH as i32),
    ] {
        f(name, val)?;
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "haiku"
    ))]
    f("O_DSYNC", libc::O_DSYNC as i32)?;

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    f("O_SYMLINK", libc::O_SYMLINK as i32)?;

    #[cfg(any(target_os = "linux", target_os = "android"))]
    f("O_DIRECT", libc::O_DIRECT as i32)?;

    #[cfg(target_os = "linux")]
    f("O_NOATIME", libc::O_NOATIME as i32)?;

    Ok(())
}

#[cfg(not(unix))]
fn visit_platform_fs_constants(f: &mut impl FnMut(&'static str, i32) -> Result<()>) -> Result<()> {
    for (name, val) in [
        ("O_RDONLY", 0i32),
        ("O_WRONLY", 1),
        ("O_RDWR", 2),
        ("O_APPEND", 8),
        ("O_CREAT", 256),
        ("O_TRUNC", 512),
        ("O_EXCL", 1024),
        ("S_IFMT", 0xf000),
        ("S_IFREG", 0x8000),
        ("S_IFDIR", 0x4000),
        ("S_IFCHR", 0x2000),
        ("S_IFIFO", 0x1000),
        ("S_IFLNK", 0xa000),
        ("S_IRWXU", 0o700),
        ("S_IRUSR", 0o400),
        ("S_IWUSR", 0o200),
        ("S_IXUSR", 0o100),
    ] {
        f(name, val)?;
    }
    Ok(())
}

/// Create the shared `fs.constants` object (`Object.create(null)` matching Node.js).
///
/// Single source of truth for `fs.constants`, `fs.promises.constants`, and `node:constants`.
pub fn create_constants<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let create: Function<'js> = ctx.eval("(function () { return Object.create(null); })")?;
    let constants: Object<'js> = create.call(())?;
    visit_fs_constants(|name, val| constants.set(name, val))?;
    Ok(constants)
}
