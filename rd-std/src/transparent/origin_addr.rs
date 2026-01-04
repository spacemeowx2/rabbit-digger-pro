use std::{io, net::SocketAddr};

pub trait OriginAddrExt {
    fn origin_addr(&self) -> io::Result<SocketAddr>;
}

use std::os::unix::prelude::AsRawFd;

impl<T: AsRawFd> OriginAddrExt for T {
    fn origin_addr(&self) -> io::Result<SocketAddr> {
        use socket2::SockAddr;

        let fd = self.as_raw_fd();

        unsafe {
            let (_, origin_addr) = SockAddr::try_init(|origin_addr, origin_addr_len| {
                let ret = if libc::getsockopt(
                    fd,
                    libc::SOL_IP,
                    libc::SO_ORIGINAL_DST,
                    origin_addr as *mut _,
                    origin_addr_len,
                ) == 0
                {
                    0
                } else {
                    libc::getsockopt(
                        fd,
                        libc::SOL_IPV6,
                        libc::IP6T_SO_ORIGINAL_DST,
                        origin_addr as *mut _,
                        origin_addr_len,
                    )
                };
                if ret != 0 {
                    let err = io::Error::last_os_error();
                    return Err(err);
                }
                Ok(())
            })?;
            Ok(origin_addr.as_socket().expect("SocketAddr"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_origin_addr_ext_for_types() {
        use std::os::unix::io::AsRawFd;

        let file = std::fs::File::open("/dev/null").unwrap();
        let _ = file.origin_addr();

        #[cfg(unix)]
        {
            use std::os::unix::net::UnixStream;
            let stream = UnixStream::connect("/tmp/does_not_exist");
            if let Ok(s) = stream {
                let _ = s.origin_addr();
            }
        }
    }
}
