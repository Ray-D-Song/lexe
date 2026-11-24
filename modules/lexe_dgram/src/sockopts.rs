use std::{
    ffi::CString,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};

use rquickjs::{Ctx, Error, Exception, Function, Result, Value};
use socket2::{Domain, Protocol, SockAddr, SockAddrStorage, SockRef, Socket as Socket2, Type};
use tokio::net::UdpSocket;

#[derive(Clone, Debug, Default)]
pub struct SocketBindOptions {
    pub reuse_addr: bool,
    pub reuse_port: bool,
    pub ipv6_only: bool,
    pub recv_buffer_size: Option<usize>,
    pub send_buffer_size: Option<usize>,
}

/// Throw a Node-compatible JS error (`Error`, `TypeError`, `RangeError`) with `.code`.
pub fn throw_coded<'js>(
    ctx: &Ctx<'js>,
    err_name: &str,
    code: &str,
    message: impl AsRef<str>,
) -> Error {
    let msg = message.as_ref();
    let ctor: Function<'js> = match ctx.eval(err_name) {
        Ok(f) => f,
        Err(err) => return err,
    };
    let Ok(val): Result<Value<'js>> = ctor.call((msg,)) else {
        return Exception::throw_message(ctx, msg);
    };
    if let Some(obj) = val.as_object() {
        let _ = obj.set("code", code);
    }
    match Exception::from_value(val) {
        Ok(exc) => exc.throw(),
        Err(err) => err,
    }
}

pub fn make_coded_exception<'js>(
    ctx: &Ctx<'js>,
    err_name: &str,
    code: &str,
    message: &str,
) -> Result<Value<'js>> {
    let ctor: Function<'js> = ctx.eval(err_name)?;
    let val: Value<'js> = ctor.call((message,))?;
    if let Some(obj) = val.as_object() {
        obj.set("code", code)?;
    }
    Ok(val)
}

#[cfg(unix)]
fn os_error_code(err: &io::Error) -> &'static str {
    match err.raw_os_error() {
        Some(libc::EBADF) => "EBADF",
        Some(libc::EINVAL) => "EINVAL",
        Some(libc::EADDRINUSE) => "EADDRINUSE",
        Some(libc::EADDRNOTAVAIL) => "EADDRNOTAVAIL",
        Some(libc::EAFNOSUPPORT) => "EAFNOSUPPORT",
        Some(libc::EOPNOTSUPP) => "ENOTSUP",
        Some(libc::EACCES) => "EACCES",
        Some(libc::EPERM) => "EPERM",
        Some(libc::ENODEV) => "ENODEV",
        Some(libc::ENOPROTOOPT) => "ENOPROTOOPT",
        Some(libc::ENOBUFS) => "ENOBUFS",
        Some(libc::ENOMEM) => "ENOMEM",
        Some(libc::ECONNREFUSED) => "ECONNREFUSED",
        Some(libc::EHOSTUNREACH) => "EHOSTUNREACH",
        Some(libc::ENETUNREACH) => "ENETUNREACH",
        _ => "EINVAL",
    }
}

#[cfg(not(unix))]
fn os_error_code(err: &io::Error) -> &'static str {
    use io::ErrorKind;
    match err.kind() {
        ErrorKind::AddrInUse => "EADDRINUSE",
        ErrorKind::AddrNotAvailable => "EADDRNOTAVAIL",
        ErrorKind::PermissionDenied => "EACCES",
        ErrorKind::ConnectionRefused => "ECONNREFUSED",
        ErrorKind::Unsupported => "ENOTSUP",
        ErrorKind::InvalidInput => "EINVAL",
        _ => "EINVAL",
    }
}

pub fn throw_syscall_error<'js>(ctx: &Ctx<'js>, syscall: &str, err: &io::Error) -> Error {
    let code = os_error_code(err);
    throw_coded(ctx, "Error", code, format!("{syscall} {code}"))
}

pub fn throw_buffer_size_error<'js>(ctx: &Ctx<'js>, err: &io::Error) -> Error {
    let code = os_error_code(err);
    throw_coded(
        ctx,
        "Error",
        "ERR_SOCKET_BUFFER_SIZE",
        format!("Could not get or set buffer size: {code} ({err})"),
    )
}

/// Resolve host and port, fast-pathing IP literals and filtering DNS results by IP family.
pub async fn resolve_socket_addr(
    host: &str,
    port: u16,
    is_ipv6: bool,
) -> io::Result<SocketAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, port));
    }

    let mut addrs = tokio::net::lookup_host((host, port)).await?;
    let mut first = None;
    for addr in &mut addrs {
        if addr.is_ipv6() == is_ipv6 {
            return Ok(addr);
        }
        if first.is_none() {
            first = Some(addr);
        }
    }
    first.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "address resolution returned empty"))
}

pub fn configure_and_bind_socket(
    addr: SocketAddr,
    is_ipv6: bool,
    opts: &SocketBindOptions,
) -> io::Result<UdpSocket> {
    let domain = if is_ipv6 { Domain::IPV6 } else { Domain::IPV4 };
    let sock = Socket2::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
    if opts.reuse_addr {
        sock.set_reuse_address(true)?;
    }
    #[cfg(all(
        unix,
        not(any(target_os = "solaris", target_os = "illumos", target_os = "cygwin"))
    ))]
    if opts.reuse_port {
        sock.set_reuse_port(true)?;
    }
    if is_ipv6 && opts.ipv6_only {
        sock.set_only_v6(true)?;
    }
    sock.set_nonblocking(true)?;
    sock.bind(&SockAddr::from(addr))?;
    if let Some(size) = opts.recv_buffer_size {
        sock.set_recv_buffer_size(size)?;
    }
    if let Some(size) = opts.send_buffer_size {
        sock.set_send_buffer_size(size)?;
    }
    let std_sock: std::net::UdpSocket = sock.into();
    UdpSocket::from_std(std_sock)
}

#[cfg(unix)]
fn unspec_sockaddr() -> SockAddr {
    let mut storage = SockAddrStorage::zeroed();
    let len = std::mem::size_of::<libc::sockaddr>() as socket2::socklen_t;
    unsafe {
        let sa = storage.view_as::<libc::sockaddr>();
        sa.sa_family = libc::AF_UNSPEC as libc::sa_family_t;
        SockAddr::new(storage, len)
    }
}

#[cfg(windows)]
fn unspec_sockaddr() -> SockAddr {
    use windows_sys::Win32::Networking::WinSock::{AF_UNSPEC, SOCKADDR};
    let mut storage = SockAddrStorage::zeroed();
    let len = std::mem::size_of::<SOCKADDR>() as socket2::socklen_t;
    unsafe {
        let sa = storage.view_as::<SOCKADDR>();
        sa.sa_family = AF_UNSPEC;
        SockAddr::new(storage, len)
    }
}

pub fn disconnect_udp_socket(socket: &UdpSocket) -> io::Result<()> {
    let res = SockRef::from(socket).connect(&unspec_sockaddr());
    match res {
        Ok(()) => Ok(()),
        #[cfg(unix)]
        Err(err)
            if matches!(
                err.raw_os_error(),
                Some(libc::EAFNOSUPPORT) | Some(libc::EADDRNOTAVAIL)
            ) =>
        {
            Ok(())
        },
        Err(err) => Err(err),
    }
}

pub fn sock_set_broadcast(socket: Option<&UdpSocket>, ctx: &Ctx<'_>, flag: bool) -> Result<()> {
    let Some(sock) = socket else {
        return Err(throw_coded(ctx, "Error", "EBADF", "setBroadcast EBADF"));
    };
    SockRef::from(sock)
        .set_broadcast(flag)
        .map_err(|e| throw_syscall_error(ctx, "setBroadcast", &e))
}

pub fn sock_set_ttl(socket: Option<&UdpSocket>, ctx: &Ctx<'_>, is_ipv6: bool, ttl: i32) -> Result<i32> {
    if !(1..=255).contains(&ttl) {
        return Err(throw_coded(ctx, "Error", "EINVAL", "setTTL EINVAL"));
    }
    let Some(sock) = socket else {
        return Err(throw_coded(ctx, "Error", "EBADF", "setTTL EBADF"));
    };
    let res = if is_ipv6 {
        SockRef::from(sock).set_unicast_hops_v6(ttl as u32)
    } else {
        SockRef::from(sock).set_ttl_v4(ttl as u32)
    };
    res.map_err(|e| throw_syscall_error(ctx, "setTTL", &e))?;
    Ok(ttl)
}

pub fn sock_set_multicast_ttl(
    socket: Option<&UdpSocket>,
    ctx: &Ctx<'_>,
    is_ipv6: bool,
    ttl: i32,
) -> Result<i32> {
    if !(0..=255).contains(&ttl) {
        return Err(throw_coded(ctx, "Error", "EINVAL", "setMulticastTTL EINVAL"));
    }
    let Some(sock) = socket else {
        return Err(throw_coded(ctx, "Error", "EBADF", "setMulticastTTL EBADF"));
    };
    let res = if is_ipv6 {
        SockRef::from(sock).set_multicast_hops_v6(ttl as u32)
    } else {
        SockRef::from(sock).set_multicast_ttl_v4(ttl as u32)
    };
    res.map_err(|e| throw_syscall_error(ctx, "setMulticastTTL", &e))?;
    Ok(ttl)
}

pub fn sock_set_multicast_loopback(
    socket: Option<&UdpSocket>,
    ctx: &Ctx<'_>,
    is_ipv6: bool,
    flag: bool,
) -> Result<bool> {
    let Some(sock) = socket else {
        return Err(throw_coded(
            ctx,
            "Error",
            "EBADF",
            "setMulticastLoopback EBADF",
        ));
    };
    let res = if is_ipv6 {
        SockRef::from(sock).set_multicast_loop_v6(flag)
    } else {
        SockRef::from(sock).set_multicast_loop_v4(flag)
    };
    res.map_err(|e| throw_syscall_error(ctx, "setMulticastLoopback", &e))?;
    Ok(flag)
}

#[cfg(unix)]
fn resolve_iface_name(name: &str) -> io::Result<u32> {
    let c_name =
        CString::new(name).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    let idx = unsafe { libc::if_nametoindex(c_name.as_ptr()) };
    if idx == 0 {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    Ok(idx)
}

#[cfg(windows)]
fn resolve_iface_name(name: &str) -> io::Result<u32> {
    use windows_sys::Win32::NetworkManagement::IpHelper::if_nametoindex;
    let c_name =
        CString::new(name).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "EINVAL"))?;
    let idx = unsafe { if_nametoindex(c_name.as_ptr().cast()) };
    if idx == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "EINVAL"));
    }
    Ok(idx)
}

fn parse_ipv6_interface(iface: &str) -> io::Result<u32> {
    let Some((ip_part, zone)) = iface.split_once('%') else {
        let ip: Ipv6Addr = iface
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "EINVAL"))?;
        if ip.is_unspecified() {
            return Ok(0);
        }
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "EINVAL"));
    };

    ip_part
        .parse::<Ipv6Addr>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "EINVAL"))?;
    resolve_iface_name(zone)
}

pub fn sock_set_multicast_interface(
    socket: Option<&UdpSocket>,
    ctx: &Ctx<'_>,
    is_ipv6: bool,
    iface: &str,
) -> Result<()> {
    let Some(sock) = socket else {
        return Err(throw_coded(
            ctx,
            "Error",
            "EBADF",
            "setMulticastInterface EBADF",
        ));
    };
    let sock_ref = SockRef::from(sock);
    if is_ipv6 {
        let idx = parse_ipv6_interface(iface)
            .map_err(|e| throw_syscall_error(ctx, "setMulticastInterface", &e))?;
        sock_ref
            .set_multicast_if_v6(idx)
            .map_err(|e| throw_syscall_error(ctx, "setMulticastInterface", &e))
    } else {
        let ip: Ipv4Addr = iface
            .parse()
            .map_err(|_| throw_coded(ctx, "Error", "EINVAL", "setMulticastInterface EINVAL"))?;
        sock_ref
            .set_multicast_if_v4(&ip)
            .map_err(|e| throw_syscall_error(ctx, "setMulticastInterface", &e))
    }
}

pub fn sock_membership(
    socket: Option<&UdpSocket>,
    ctx: &Ctx<'_>,
    is_ipv6: bool,
    join: bool,
    multicast_addr: &str,
    multicast_iface: Option<&str>,
) -> Result<()> {
    let syscall = if join {
        "addMembership"
    } else {
        "dropMembership"
    };
    let Some(sock) = socket else {
        return Err(throw_coded(ctx, "Error", "EBADF", format!("{syscall} EBADF")));
    };
    let sock_ref = SockRef::from(sock);

    if is_ipv6 {
        let group: Ipv6Addr = multicast_addr
            .parse()
            .map_err(|_| throw_coded(ctx, "Error", "EINVAL", format!("{syscall} EINVAL")))?;
        let idx = match multicast_iface {
            None | Some("") => 0,
            Some(s) => parse_ipv6_interface(s).map_err(|e| throw_syscall_error(ctx, syscall, &e))?,
        };
        let res = if join {
            sock_ref.join_multicast_v6(&group, idx)
        } else {
            sock_ref.leave_multicast_v6(&group, idx)
        };
        res.map_err(|e| throw_syscall_error(ctx, syscall, &e))
    } else {
        let group: Ipv4Addr = multicast_addr
            .parse()
            .map_err(|_| throw_coded(ctx, "Error", "EINVAL", format!("{syscall} EINVAL")))?;
        let iface_ip = match multicast_iface {
            None | Some("") => Ipv4Addr::UNSPECIFIED,
            Some(s) => s
                .parse::<Ipv4Addr>()
                .map_err(|_| throw_coded(ctx, "Error", "EINVAL", format!("{syscall} EINVAL")))?,
        };
        let res = if join {
            sock_ref.join_multicast_v4(&group, &iface_ip)
        } else {
            sock_ref.leave_multicast_v4(&group, &iface_ip)
        };
        res.map_err(|e| throw_syscall_error(ctx, syscall, &e))
    }
}

#[cfg(unix)]
fn ipv6_ssm_setsockopt(
    sock: &UdpSocket,
    join: bool,
    source: Ipv6Addr,
    group: Ipv6Addr,
    ifindex: u32,
) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const MCAST_JOIN_SOURCE_GROUP_OPT: libc::c_int = 82;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const MCAST_LEAVE_SOURCE_GROUP_OPT: libc::c_int = 83;

    #[cfg(any(target_os = "linux", target_os = "android"))]
    const MCAST_JOIN_SOURCE_GROUP_OPT: libc::c_int = libc::MCAST_JOIN_SOURCE_GROUP;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const MCAST_LEAVE_SOURCE_GROUP_OPT: libc::c_int = libc::MCAST_LEAVE_SOURCE_GROUP;

    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "linux",
        target_os = "android"
    )))]
    {
        let _ = (sock, join, source, group, ifindex);
        return Err(io::Error::from_raw_os_error(libc::EOPNOTSUPP));
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "linux",
        target_os = "android"
    ))]
    {
        #[cfg_attr(any(target_os = "macos", target_os = "ios"), repr(C, packed(4)))]
        #[cfg_attr(not(any(target_os = "macos", target_os = "ios")), repr(C))]
        struct GroupSourceReq {
            gsr_interface: u32,
            gsr_group: libc::sockaddr_storage,
            gsr_source: libc::sockaddr_storage,
        }

        fn to_storage(ip: Ipv6Addr) -> libc::sockaddr_storage {
            let sa = SockAddr::from(SocketAddr::new(IpAddr::V6(ip), 0));
            let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
            let len = (sa.len() as usize).min(std::mem::size_of::<libc::sockaddr_storage>());
            unsafe {
                std::ptr::copy_nonoverlapping(
                    sa.as_ptr().cast::<u8>(),
                    (&raw mut storage).cast::<u8>(),
                    len,
                );
            }
            storage
        }

        let req = GroupSourceReq {
            gsr_interface: ifindex,
            gsr_group: to_storage(group),
            gsr_source: to_storage(source),
        };
        let opt = if join {
            MCAST_JOIN_SOURCE_GROUP_OPT
        } else {
            MCAST_LEAVE_SOURCE_GROUP_OPT
        };
        let rc = unsafe {
            libc::setsockopt(
                sock.as_raw_fd(),
                libc::IPPROTO_IPV6,
                opt,
                (&raw const req).cast(),
                std::mem::size_of::<GroupSourceReq>() as libc::socklen_t,
            )
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(windows)]
fn ipv6_ssm_setsockopt(
    sock: &UdpSocket,
    join: bool,
    source: Ipv6Addr,
    group: Ipv6Addr,
    ifindex: u32,
) -> io::Result<()> {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{
        setsockopt, GROUP_SOURCE_REQ, IPPROTO_IPV6, MCAST_JOIN_SOURCE_GROUP,
        MCAST_LEAVE_SOURCE_GROUP, SOCKADDR_STORAGE, SOCKET_ERROR,
    };

    fn to_storage(ip: Ipv6Addr) -> SOCKADDR_STORAGE {
        let sa = SockAddr::from(SocketAddr::new(IpAddr::V6(ip), 0));
        let mut storage: SOCKADDR_STORAGE = unsafe { std::mem::zeroed() };
        let len = (sa.len() as usize).min(std::mem::size_of::<SOCKADDR_STORAGE>());
        unsafe {
            std::ptr::copy_nonoverlapping(
                sa.as_ptr().cast::<u8>(),
                (&raw mut storage).cast::<u8>(),
                len,
            );
        }
        storage
    }

    let req = GROUP_SOURCE_REQ {
        gsr_interface: ifindex,
        gsr_group: to_storage(group),
        gsr_source: to_storage(source),
    };
    let opt = if join {
        MCAST_JOIN_SOURCE_GROUP
    } else {
        MCAST_LEAVE_SOURCE_GROUP
    };
    let rc = unsafe {
        setsockopt(
            sock.as_raw_socket() as _,
            IPPROTO_IPV6,
            opt as i32,
            (&raw const req).cast(),
            std::mem::size_of::<GROUP_SOURCE_REQ>() as i32,
        )
    };
    if rc == SOCKET_ERROR {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn sock_source_membership(
    socket: Option<&UdpSocket>,
    ctx: &Ctx<'_>,
    is_ipv6: bool,
    join: bool,
    source_addr: &str,
    group_addr: &str,
    multicast_iface: Option<&str>,
) -> Result<()> {
    let syscall = if join {
        "addSourceSpecificMembership"
    } else {
        "dropSourceSpecificMembership"
    };
    let Some(sock) = socket else {
        return Err(throw_coded(ctx, "Error", "EBADF", format!("{syscall} EBADF")));
    };

    if is_ipv6 {
        let source: Ipv6Addr = source_addr
            .parse()
            .map_err(|_| throw_coded(ctx, "Error", "EINVAL", format!("{syscall} EINVAL")))?;
        let group: Ipv6Addr = group_addr
            .parse()
            .map_err(|_| throw_coded(ctx, "Error", "EINVAL", format!("{syscall} EINVAL")))?;
        let idx = match multicast_iface {
            None | Some("") => 0,
            Some(s) => parse_ipv6_interface(s).map_err(|e| throw_syscall_error(ctx, syscall, &e))?,
        };
        ipv6_ssm_setsockopt(sock, join, source, group, idx)
            .map_err(|e| throw_syscall_error(ctx, syscall, &e))
    } else {
        #[cfg(not(any(
            target_os = "dragonfly",
            target_os = "haiku",
            target_os = "hurd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "redox",
            target_os = "fuchsia",
            target_os = "nto",
            target_os = "espidf",
            target_os = "vita",
            target_os = "wasi",
        )))]
        {
            let source: Ipv4Addr = source_addr
                .parse()
                .map_err(|_| throw_coded(ctx, "Error", "EINVAL", format!("{syscall} EINVAL")))?;
            let group: Ipv4Addr = group_addr
                .parse()
                .map_err(|_| throw_coded(ctx, "Error", "EINVAL", format!("{syscall} EINVAL")))?;
            let iface_ip = match multicast_iface {
                None | Some("") => Ipv4Addr::UNSPECIFIED,
                Some(s) => s
                    .parse::<Ipv4Addr>()
                    .map_err(|_| throw_coded(ctx, "Error", "EINVAL", format!("{syscall} EINVAL")))?,
            };
            let sock_ref = SockRef::from(sock);
            let res = if join {
                sock_ref.join_ssm_v4(&source, &group, &iface_ip)
            } else {
                sock_ref.leave_ssm_v4(&source, &group, &iface_ip)
            };
            res.map_err(|e| throw_syscall_error(ctx, syscall, &e))
        }
        #[cfg(any(
            target_os = "dragonfly",
            target_os = "haiku",
            target_os = "hurd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "redox",
            target_os = "fuchsia",
            target_os = "nto",
            target_os = "espidf",
            target_os = "vita",
            target_os = "wasi",
        ))]
        {
            let _ = (sock, source_addr, group_addr, multicast_iface);
            Err(throw_coded(ctx, "Error", "ENOTSUP", format!("{syscall} ENOTSUP")))
        }
    }
}

fn sock_get_buffer_size(socket: Option<&UdpSocket>, ctx: &Ctx<'_>, recv: bool) -> Result<usize> {
    let Some(sock) = socket else {
        return Err(throw_buffer_size_error(
            ctx,
            &io::Error::new(io::ErrorKind::InvalidInput, "bad file descriptor"),
        ));
    };
    let sock_ref = SockRef::from(sock);
    let res = if recv {
        sock_ref.recv_buffer_size()
    } else {
        sock_ref.send_buffer_size()
    };
    res.map_err(|e| throw_buffer_size_error(ctx, &e))
}

fn sock_set_buffer_size(
    socket: Option<&UdpSocket>,
    ctx: &Ctx<'_>,
    size: i64,
    recv: bool,
) -> Result<()> {
    if size < 0 {
        return Err(throw_coded(
            ctx,
            "Error",
            "ERR_SOCKET_BAD_BUFFER_SIZE",
            "Buffer size must be a positive integer",
        ));
    }
    let Some(sock) = socket else {
        return Err(throw_buffer_size_error(
            ctx,
            &io::Error::new(io::ErrorKind::InvalidInput, "bad file descriptor"),
        ));
    };
    let sock_ref = SockRef::from(sock);
    let res = if recv {
        sock_ref.set_recv_buffer_size(size as usize)
    } else {
        sock_ref.set_send_buffer_size(size as usize)
    };
    res.map_err(|e| throw_buffer_size_error(ctx, &e))
}

pub fn sock_get_recv_buffer_size(socket: Option<&UdpSocket>, ctx: &Ctx<'_>) -> Result<usize> {
    sock_get_buffer_size(socket, ctx, true)
}

pub fn sock_set_recv_buffer_size(
    socket: Option<&UdpSocket>,
    ctx: &Ctx<'_>,
    size: i64,
) -> Result<()> {
    sock_set_buffer_size(socket, ctx, size, true)
}

pub fn sock_get_send_buffer_size(socket: Option<&UdpSocket>, ctx: &Ctx<'_>) -> Result<usize> {
    sock_get_buffer_size(socket, ctx, false)
}

pub fn sock_set_send_buffer_size(
    socket: Option<&UdpSocket>,
    ctx: &Ctx<'_>,
    size: i64,
) -> Result<()> {
    sock_set_buffer_size(socket, ctx, size, false)
}
