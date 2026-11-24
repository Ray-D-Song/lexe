use rquickjs::{Ctx, Function, Object, Result};

fn new_null_proto_object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let create: Function<'js> = ctx.eval("(function () { return Object.create(null); })")?;
    create.call(())
}

fn build_null_proto_object<'js>(
    ctx: &Ctx<'js>,
    visit: impl FnOnce(&mut dyn FnMut(&'static str, i32) -> Result<()>) -> Result<()>,
) -> Result<Object<'js>> {
    let obj = new_null_proto_object(ctx)?;
    visit(&mut |name, val| obj.set(name, val))?;
    Ok(obj)
}

/// Visit every flat OS constant (dlopen, errno, signals, priority) in Node's `constants` module.
pub fn visit_flat_os_constants(mut f: impl FnMut(&'static str, i32) -> Result<()>) -> Result<()> {
    visit_dlopen_constants(&mut f)?;
    visit_errno_constants(&mut f)?;
    visit_signal_constants(&mut f)?;
    visit_priority_constants(&mut f)
}

/// Build `os.constants` matching Node.js (`signals`, `errno`, `dlopen`, `priority`,
/// and `UV_UDP_REUSEADDR`).
pub fn create_os_constants<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let constants = new_null_proto_object(ctx)?;
    constants.set("UV_UDP_REUSEADDR", 4i32)?;
    constants.set("dlopen", create_dlopen_constants(ctx)?)?;
    constants.set("errno", create_errno_constants(ctx)?)?;
    constants.set("signals", create_signal_constants(ctx)?)?;
    constants.set("priority", create_priority_constants(ctx)?)?;
    Ok(constants)
}

pub fn visit_priority_constants(mut f: impl FnMut(&'static str, i32) -> Result<()>) -> Result<()> {
    for (name, val) in [
        ("PRIORITY_LOW", 19i32),
        ("PRIORITY_BELOW_NORMAL", 10),
        ("PRIORITY_NORMAL", 0),
        ("PRIORITY_ABOVE_NORMAL", -7),
        ("PRIORITY_HIGH", -14),
        ("PRIORITY_HIGHEST", -20),
    ] {
        f(name, val)?;
    }
    Ok(())
}

pub fn create_priority_constants<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    build_null_proto_object(ctx, |f| visit_priority_constants(f))
}

#[cfg(unix)]
pub fn visit_dlopen_constants(mut f: impl FnMut(&'static str, i32) -> Result<()>) -> Result<()> {
    for (name, val) in [
        ("RTLD_LAZY", libc::RTLD_LAZY),
        ("RTLD_NOW", libc::RTLD_NOW),
        ("RTLD_GLOBAL", libc::RTLD_GLOBAL),
        ("RTLD_LOCAL", libc::RTLD_LOCAL),
    ] {
        f(name, val)?;
    }
    #[cfg(target_os = "linux")]
    f("RTLD_DEEPBIND", libc::RTLD_DEEPBIND)?;
    Ok(())
}

#[cfg(not(unix))]
pub fn visit_dlopen_constants(_f: impl FnMut(&'static str, i32) -> Result<()>) -> Result<()> {
    Ok(())
}

pub fn create_dlopen_constants<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    build_null_proto_object(ctx, |f| visit_dlopen_constants(f))
}

#[cfg(unix)]
pub fn visit_signal_constants(mut f: impl FnMut(&'static str, i32) -> Result<()>) -> Result<()> {
    for (name, val) in [
        ("SIGHUP", libc::SIGHUP),
        ("SIGINT", libc::SIGINT),
        ("SIGQUIT", libc::SIGQUIT),
        ("SIGILL", libc::SIGILL),
        ("SIGTRAP", libc::SIGTRAP),
        ("SIGABRT", libc::SIGABRT),
        ("SIGIOT", libc::SIGIOT),
        ("SIGBUS", libc::SIGBUS),
        ("SIGFPE", libc::SIGFPE),
        ("SIGKILL", libc::SIGKILL),
        ("SIGUSR1", libc::SIGUSR1),
        ("SIGSEGV", libc::SIGSEGV),
        ("SIGUSR2", libc::SIGUSR2),
        ("SIGPIPE", libc::SIGPIPE),
        ("SIGALRM", libc::SIGALRM),
        ("SIGTERM", libc::SIGTERM),
        ("SIGCHLD", libc::SIGCHLD),
        ("SIGCONT", libc::SIGCONT),
        ("SIGSTOP", libc::SIGSTOP),
        ("SIGTSTP", libc::SIGTSTP),
        ("SIGTTIN", libc::SIGTTIN),
        ("SIGTTOU", libc::SIGTTOU),
        ("SIGURG", libc::SIGURG),
        ("SIGXCPU", libc::SIGXCPU),
        ("SIGXFSZ", libc::SIGXFSZ),
        ("SIGVTALRM", libc::SIGVTALRM),
        ("SIGPROF", libc::SIGPROF),
        ("SIGWINCH", libc::SIGWINCH),
        ("SIGIO", libc::SIGIO),
        ("SIGSYS", libc::SIGSYS),
    ] {
        f(name, val)?;
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))]
    f("SIGINFO", libc::SIGINFO)?;

    #[cfg(target_os = "linux")]
    for (name, val) in [
        ("SIGSTKFLT", libc::SIGSTKFLT),
        ("SIGPWR", libc::SIGPWR),
        ("SIGPOLL", libc::SIGPOLL),
    ] {
        f(name, val)?;
    }

    Ok(())
}

#[cfg(not(unix))]
pub fn visit_signal_constants(mut f: impl FnMut(&'static str, i32) -> Result<()>) -> Result<()> {
    for (name, val) in [
        ("SIGINT", 2i32),
        ("SIGILL", 4),
        ("SIGABRT_COMPAT", 6),
        ("SIGFPE", 8),
        ("SIGKILL", 9),
        ("SIGSEGV", 11),
        ("SIGTERM", 15),
        ("SIGBREAK", 21),
        ("SIGABRT", 22),
    ] {
        f(name, val)?;
    }
    Ok(())
}

pub fn create_signal_constants<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    build_null_proto_object(ctx, |f| visit_signal_constants(f))
}

#[cfg(unix)]
pub fn visit_errno_constants(mut f: impl FnMut(&'static str, i32) -> Result<()>) -> Result<()> {
    for (name, val) in [
        ("E2BIG", libc::E2BIG),
        ("EACCES", libc::EACCES),
        ("EADDRINUSE", libc::EADDRINUSE),
        ("EADDRNOTAVAIL", libc::EADDRNOTAVAIL),
        ("EAFNOSUPPORT", libc::EAFNOSUPPORT),
        ("EAGAIN", libc::EAGAIN),
        ("EALREADY", libc::EALREADY),
        ("EBADF", libc::EBADF),
        ("EBADMSG", libc::EBADMSG),
        ("EBUSY", libc::EBUSY),
        ("ECANCELED", libc::ECANCELED),
        ("ECHILD", libc::ECHILD),
        ("ECONNABORTED", libc::ECONNABORTED),
        ("ECONNREFUSED", libc::ECONNREFUSED),
        ("ECONNRESET", libc::ECONNRESET),
        ("EDEADLK", libc::EDEADLK),
        ("EDESTADDRREQ", libc::EDESTADDRREQ),
        ("EDOM", libc::EDOM),
        ("EDQUOT", libc::EDQUOT),
        ("EEXIST", libc::EEXIST),
        ("EFAULT", libc::EFAULT),
        ("EFBIG", libc::EFBIG),
        ("EHOSTUNREACH", libc::EHOSTUNREACH),
        ("EIDRM", libc::EIDRM),
        ("EILSEQ", libc::EILSEQ),
        ("EINPROGRESS", libc::EINPROGRESS),
        ("EINTR", libc::EINTR),
        ("EINVAL", libc::EINVAL),
        ("EIO", libc::EIO),
        ("EISCONN", libc::EISCONN),
        ("EISDIR", libc::EISDIR),
        ("ELOOP", libc::ELOOP),
        ("EMFILE", libc::EMFILE),
        ("EMLINK", libc::EMLINK),
        ("EMSGSIZE", libc::EMSGSIZE),
        ("EMULTIHOP", libc::EMULTIHOP),
        ("ENAMETOOLONG", libc::ENAMETOOLONG),
        ("ENETDOWN", libc::ENETDOWN),
        ("ENETRESET", libc::ENETRESET),
        ("ENETUNREACH", libc::ENETUNREACH),
        ("ENFILE", libc::ENFILE),
        ("ENOBUFS", libc::ENOBUFS),
        ("ENODATA", libc::ENODATA),
        ("ENODEV", libc::ENODEV),
        ("ENOENT", libc::ENOENT),
        ("ENOEXEC", libc::ENOEXEC),
        ("ENOLCK", libc::ENOLCK),
        ("ENOLINK", libc::ENOLINK),
        ("ENOMEM", libc::ENOMEM),
        ("ENOMSG", libc::ENOMSG),
        ("ENOPROTOOPT", libc::ENOPROTOOPT),
        ("ENOSPC", libc::ENOSPC),
        ("ENOSR", libc::ENOSR),
        ("ENOSTR", libc::ENOSTR),
        ("ENOSYS", libc::ENOSYS),
        ("ENOTCONN", libc::ENOTCONN),
        ("ENOTDIR", libc::ENOTDIR),
        ("ENOTEMPTY", libc::ENOTEMPTY),
        ("ENOTSOCK", libc::ENOTSOCK),
        ("ENOTSUP", libc::ENOTSUP),
        ("ENOTTY", libc::ENOTTY),
        ("ENXIO", libc::ENXIO),
        ("EOPNOTSUPP", libc::EOPNOTSUPP),
        ("EOVERFLOW", libc::EOVERFLOW),
        ("EPERM", libc::EPERM),
        ("EPIPE", libc::EPIPE),
        ("EPROTO", libc::EPROTO),
        ("EPROTONOSUPPORT", libc::EPROTONOSUPPORT),
        ("EPROTOTYPE", libc::EPROTOTYPE),
        ("ERANGE", libc::ERANGE),
        ("EROFS", libc::EROFS),
        ("ESPIPE", libc::ESPIPE),
        ("ESRCH", libc::ESRCH),
        ("ESTALE", libc::ESTALE),
        ("ETIME", libc::ETIME),
        ("ETIMEDOUT", libc::ETIMEDOUT),
        ("ETXTBSY", libc::ETXTBSY),
        ("EWOULDBLOCK", libc::EWOULDBLOCK),
        ("EXDEV", libc::EXDEV),
    ] {
        f(name, val)?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn visit_errno_constants(mut f: impl FnMut(&'static str, i32) -> Result<()>) -> Result<()> {
    for (name, val) in [
        ("E2BIG", 7i32),
        ("EACCES", 13),
        ("EADDRINUSE", 100),
        ("EADDRNOTAVAIL", 101),
        ("EAFNOSUPPORT", 102),
        ("EAGAIN", 11),
        ("EALREADY", 103),
        ("EBADF", 9),
        ("EBADMSG", 104),
        ("EBUSY", 16),
        ("ECANCELED", 105),
        ("ECHILD", 10),
        ("ECONNABORTED", 106),
        ("ECONNREFUSED", 107),
        ("ECONNRESET", 108),
        ("EDEADLK", 36),
        ("EDESTADDRREQ", 109),
        ("EDOM", 33),
        ("EEXIST", 17),
        ("EFAULT", 14),
        ("EFBIG", 27),
        ("EHOSTUNREACH", 110),
        ("EIDRM", 111),
        ("EILSEQ", 42),
        ("EINPROGRESS", 112),
        ("EINTR", 4),
        ("EINVAL", 22),
        ("EIO", 5),
        ("EISCONN", 113),
        ("EISDIR", 21),
        ("ELOOP", 114),
        ("EMFILE", 24),
        ("EMLINK", 31),
        ("EMSGSIZE", 115),
        ("ENAMETOOLONG", 38),
        ("ENETDOWN", 116),
        ("ENETRESET", 117),
        ("ENETUNREACH", 118),
        ("ENFILE", 23),
        ("ENOBUFS", 119),
        ("ENODATA", 120),
        ("ENODEV", 19),
        ("ENOENT", 2),
        ("ENOEXEC", 8),
        ("ENOLCK", 39),
        ("ENOLINK", 121),
        ("ENOMEM", 12),
        ("ENOMSG", 122),
        ("ENOPROTOOPT", 123),
        ("ENOSPC", 28),
        ("ENOSR", 124),
        ("ENOSTR", 125),
        ("ENOSYS", 40),
        ("ENOTCONN", 126),
        ("ENOTDIR", 20),
        ("ENOTEMPTY", 41),
        ("ENOTSOCK", 128),
        ("ENOTSUP", 129),
        ("ENOTTY", 25),
        ("ENXIO", 6),
        ("EOPNOTSUPP", 130),
        ("EOVERFLOW", 132),
        ("EPERM", 1),
        ("EPIPE", 32),
        ("EPROTO", 134),
        ("EPROTONOSUPPORT", 135),
        ("EPROTOTYPE", 136),
        ("ERANGE", 34),
        ("EROFS", 30),
        ("ESPIPE", 29),
        ("ESRCH", 3),
        ("ETIME", 137),
        ("ETIMEDOUT", 138),
        ("ETXTBSY", 139),
        ("EWOULDBLOCK", 140),
        ("EXDEV", 18),
        ("WSAEINTR", 10004),
        ("WSAEBADF", 10009),
        ("WSAEACCES", 10013),
        ("WSAEFAULT", 10014),
        ("WSAEINVAL", 10022),
        ("WSAEMFILE", 10024),
        ("WSAEWOULDBLOCK", 10035),
        ("WSAEINPROGRESS", 10036),
        ("WSAEALREADY", 10037),
        ("WSAENOTSOCK", 10038),
        ("WSAEDESTADDRREQ", 10039),
        ("WSAEMSGSIZE", 10040),
        ("WSAEPROTOTYPE", 10041),
        ("WSAENOPROTOOPT", 10042),
        ("WSAEPROTONOSUPPORT", 10043),
        ("WSAEOPNOTSUPP", 10045),
        ("WSAEAFNOSUPPORT", 10047),
        ("WSAEADDRINUSE", 10048),
        ("WSAEADDRNOTAVAIL", 10049),
        ("WSAENETDOWN", 10050),
        ("WSAENETUNREACH", 10051),
        ("WSAENETRESET", 10052),
        ("WSAECONNABORTED", 10053),
        ("WSAECONNRESET", 10054),
        ("WSAENOBUFS", 10055),
        ("WSAEISCONN", 10056),
        ("WSAENOTCONN", 10057),
        ("WSAETIMEDOUT", 10060),
        ("WSAECONNREFUSED", 10061),
        ("WSAELOOP", 10062),
        ("WSAENAMETOOLONG", 10063),
        ("WSAEHOSTUNREACH", 10065),
        ("WSAENOTEMPTY", 10066),
    ] {
        f(name, val)?;
    }
    Ok(())
}

pub fn create_errno_constants<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    build_null_proto_object(ctx, |f| visit_errno_constants(f))
}
