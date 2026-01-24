// UDP: https://github.com/shadowsocks/shadowsocks-rust/blob/0433b3ec09bcaa26f7460a50287b56c67b687a34/crates/shadowsocks-service/src/local/redir/udprelay/sys/unix/linux.rs#L56

use std::{
    io, mem,
    net::{SocketAddr, UdpSocket},
    os::unix::prelude::{AsRawFd, RawFd},
    ptr,
    task::{Context, Poll},
};

use cfg_if::cfg_if;
use futures::ready;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use tokio::{
    io::unix::AsyncFd,
    net::{TcpListener, TcpSocket},
};

// https://github.com/shadowsocks/shadowsocks-rust/blob/0433b3ec09bcaa26f7460a50287b56c67b687a34/crates/shadowsocks-service/src/local/redir/tcprelay/sys/unix/linux.rs#L92-L134
pub async fn create_tcp_listener(addr: SocketAddr) -> io::Result<TcpListener> {
    let socket = match addr {
        SocketAddr::V4(..) => TcpSocket::new_v4()?,
        SocketAddr::V6(..) => TcpSocket::new_v6()?,
    };

    // For Linux 2.4+ TPROXY
    // Sockets have to set IP_TRANSPARENT, IPV6_TRANSPARENT for retrieving original destination by getsockname()
    unsafe {
        let fd = socket.as_raw_fd();

        let enable: libc::c_int = 1;
        let ret = match addr {
            SocketAddr::V4(..) => libc::setsockopt(
                fd,
                libc::IPPROTO_IP,
                libc::IP_TRANSPARENT,
                &enable as *const _ as *const _,
                mem::size_of_val(&enable) as libc::socklen_t,
            ),
            SocketAddr::V6(..) => libc::setsockopt(
                fd,
                libc::IPPROTO_IPV6,
                libc::IPV6_TRANSPARENT,
                &enable as *const _ as *const _,
                mem::size_of_val(&enable) as libc::socklen_t,
            ),
        };

        if ret != 0 {
            return Err(io::Error::last_os_error());
        }
    }

    // tokio requires allow reuse addr
    socket.set_reuseaddr(true)?;

    // bind, listen as original
    socket.bind(addr)?;
    socket.listen(128)
}

pub struct TransparentUdp(AsyncFd<UdpSocket>);

// https://github.com/shadowsocks/shadowsocks-rust/blob/0433b3ec09bcaa26f7460a50287b56c67b687a34/crates/shadowsocks-service/src/local/redir/udprelay/sys/unix/linux.rs#L56
fn create_udp_socket(
    addr: SocketAddr,
    reuse_port: bool,
    mark: Option<u32>,
) -> io::Result<TransparentUdp> {
    let socket = Socket::new(Domain::for_address(addr), Type::DGRAM, Some(Protocol::UDP))?;
    set_socket_before_bind(&addr, &socket)?;

    socket.set_nonblocking(true)?;
    socket.set_reuse_address(true)?;
    if reuse_port {
        socket.set_reuse_port(true)?;
    }
    if let Some(mark) = mark {
        socket.set_mark(mark)?;
    }

    socket.bind(&SockAddr::from(addr))?;

    let io = AsyncFd::new(socket.into())?;
    Ok(TransparentUdp(io))
}

fn set_socket_before_bind(addr: &SocketAddr, socket: &Socket) -> io::Result<()> {
    let fd = socket.as_raw_fd();

    let enable: libc::c_int = 1;
    unsafe {
        // 1. Set IP_TRANSPARENT, IPV6_TRANSPARENT to allow binding to non-local addresses
        let ret = match *addr {
            SocketAddr::V4(..) => libc::setsockopt(
                fd,
                libc::SOL_IP,
                libc::IP_TRANSPARENT,
                &enable as *const _ as *const _,
                mem::size_of_val(&enable) as libc::socklen_t,
            ),
            SocketAddr::V6(..) => libc::setsockopt(
                fd,
                libc::SOL_IPV6,
                libc::IPV6_TRANSPARENT,
                &enable as *const _ as *const _,
                mem::size_of_val(&enable) as libc::socklen_t,
            ),
        };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }

        // 2. Set IP_RECVORIGDSTADDR, IPV6_RECVORIGDSTADDR
        let ret = match *addr {
            SocketAddr::V4(..) => libc::setsockopt(
                fd,
                libc::SOL_IP,
                libc::IP_RECVORIGDSTADDR,
                &enable as *const _ as *const _,
                mem::size_of_val(&enable) as libc::socklen_t,
            ),
            SocketAddr::V6(..) => libc::setsockopt(
                fd,
                libc::SOL_IPV6,
                libc::IPV6_RECVORIGDSTADDR,
                &enable as *const _ as *const _,
                mem::size_of_val(&enable) as libc::socklen_t,
            ),
        };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }
    }

    Ok(())
}

fn get_destination_addr(msg: &libc::msghdr) -> io::Result<SocketAddr> {
    unsafe {
        let (_, addr) = SockAddr::try_init(|dst_addr, dst_addr_len| {
            let mut cmsg: *mut libc::cmsghdr = libc::CMSG_FIRSTHDR(msg);
            while !cmsg.is_null() {
                let rcmsg = &*cmsg;
                match (rcmsg.cmsg_level, rcmsg.cmsg_type) {
                    (libc::SOL_IP, libc::IP_RECVORIGDSTADDR) => {
                        ptr::copy(
                            libc::CMSG_DATA(cmsg),
                            dst_addr as *mut _,
                            mem::size_of::<libc::sockaddr_in>(),
                        );
                        *dst_addr_len = mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;

                        return Ok(());
                    }
                    (libc::SOL_IPV6, libc::IPV6_RECVORIGDSTADDR) => {
                        ptr::copy(
                            libc::CMSG_DATA(cmsg),
                            dst_addr as *mut _,
                            mem::size_of::<libc::sockaddr_in6>(),
                        );
                        *dst_addr_len = mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t;

                        return Ok(());
                    }
                    _ => {}
                }
                cmsg = libc::CMSG_NXTHDR(msg, cmsg);
            }

            let err = io::Error::new(
                io::ErrorKind::InvalidData,
                "missing destination address in msghdr",
            );
            Err(err)
        })?;

        Ok(addr.as_socket().expect("SocketAddr"))
    }
}

fn recv_dest_from(
    socket: &UdpSocket,
    buf: &mut [u8],
) -> io::Result<(usize, SocketAddr, SocketAddr)> {
    unsafe {
        let mut control_buf = [0u8; 64];
        let mut src_addr: libc::sockaddr_storage = mem::zeroed();

        let mut msg: libc::msghdr = mem::zeroed();
        msg.msg_name = &mut src_addr as *mut _ as *mut _;
        msg.msg_namelen = mem::size_of_val(&src_addr) as libc::socklen_t;

        let mut iov = libc::iovec {
            iov_base: buf.as_mut_ptr() as *mut _,
            iov_len: buf.len() as libc::size_t,
        };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;

        msg.msg_control = control_buf.as_mut_ptr() as *mut _;
        cfg_if! {
            if #[cfg(any(target_env = "musl", all(target_env = "uclibc", target_arch = "arm")))] {
                msg.msg_controllen = control_buf.len() as libc::socklen_t;
            } else {
                msg.msg_controllen = control_buf.len() as libc::size_t;
            }
        }

        let fd = socket.as_raw_fd();
        let ret = libc::recvmsg(fd, &mut msg, 0);
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }

        let (_, src_saddr) = SockAddr::try_init(|a, l| {
            ptr::copy_nonoverlapping(msg.msg_name, a as *mut _, msg.msg_namelen as usize);
            *l = msg.msg_namelen;
            Ok(())
        })?;

        Ok((
            ret as usize,
            src_saddr.as_socket().expect("SocketAddr"),
            get_destination_addr(&msg)?,
        ))
    }
}

impl AsRawFd for TransparentUdp {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl TransparentUdp {
    pub fn poll_send_to(
        &self,
        cx: &mut Context<'_>,
        buf: &[u8],
        target: SocketAddr,
    ) -> Poll<io::Result<usize>> {
        loop {
            let mut guard = ready!(self.0.poll_write_ready(cx))?;

            match guard.try_io(|inner| inner.get_ref().send_to(buf, target)) {
                Ok(result) => return Poll::Ready(result),
                Err(_would_block) => continue,
            }
        }
    }
    pub fn poll_recv(
        &self,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<(usize, SocketAddr, SocketAddr)>> {
        loop {
            let mut guard = ready!(self.0.poll_read_ready(cx))?;

            match guard.try_io(|inner| recv_dest_from(inner.get_ref(), buf)) {
                Ok(result) => return Poll::Ready(result),
                Err(_) => continue,
            }
        }
    }
    pub fn listen(addr: SocketAddr) -> io::Result<TransparentUdp> {
        create_udp_socket(addr, false, None)
    }
    pub fn bind_any(addr: SocketAddr, mark: Option<u32>) -> io::Result<TransparentUdp> {
        let socket = create_udp_socket(addr, true, mark)?;

        Ok(socket)
    }
}
