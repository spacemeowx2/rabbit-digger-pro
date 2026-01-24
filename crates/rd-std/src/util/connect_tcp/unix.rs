use std::{
    io,
    os::unix::io::RawFd,
    pin::Pin,
    task::{Context, Poll},
};

use futures::ready;
use rd_interface::{AsyncRead, AsyncWrite, Fd, ReadBuf, TcpStream};

const PIPE_BUFFER_SIZE: usize = 8192;

fn splice(from_fd: RawFd, to_fd: RawFd, size: usize) -> io::Result<usize> {
    use libc::{splice, SPLICE_F_MOVE, SPLICE_F_NONBLOCK};
    let ret = unsafe {
        splice(
            from_fd,
            std::ptr::null_mut(),
            to_fd,
            std::ptr::null_mut(),
            size,
            SPLICE_F_MOVE | SPLICE_F_NONBLOCK,
        )
    };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(ret as usize)
}

#[derive(Debug)]
struct Pipe {
    rpipe: i32,
    wpipe: i32,
    n: usize,
}

impl Pipe {
    fn new() -> io::Result<Self> {
        use libc::{c_int, O_NONBLOCK};
        let mut pipes = std::mem::MaybeUninit::<[c_int; 2]>::uninit();
        unsafe {
            if libc::pipe2(pipes.as_mut_ptr() as *mut c_int, O_NONBLOCK) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Pipe {
                rpipe: pipes.assume_init()[0],
                wpipe: pipes.assume_init()[1],
                n: 0,
            })
        }
    }
}

#[derive(Debug)]
pub struct SpliceState {
    pipe: Pipe,
    read: usize,
}

impl SpliceState {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            pipe: Pipe::new()?,
            read: 0,
        })
    }
    fn take_read(&mut self) -> usize {
        let read = self.read;
        self.read = 0;
        read
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.rpipe);
            libc::close(self.wpipe);
        }
    }
}

fn poll_splice_inner(
    cx: &mut Context<'_>,
    state: &mut Option<SpliceState>,
    from_fd: RawFd,
    to_fd: RawFd,
    from: &mut TcpStream,
    to: &mut TcpStream,
) -> Poll<io::Result<usize>> {
    let from = Pin::new(from);
    let to = Pin::new(to);

    if state.is_none() {
        *state = Some(SpliceState::new()?);
    }
    let state = state.as_mut().unwrap();
    let Pipe { rpipe, wpipe, n } = &mut state.pipe;

    // wait for ready
    let mut buf = ReadBuf::new(&mut [0; 0]);
    ready!(from.poll_read(cx, &mut buf))?;

    while *n < PIPE_BUFFER_SIZE {
        match splice(from_fd, *wpipe, PIPE_BUFFER_SIZE - *n) {
            Ok(x) => {
                *n += x;
                state.read += x;
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
            Err(err) => return Poll::Ready(Err(err)),
        }
    }

    ready!(to.poll_write(cx, &[0; 0]))?;
    while *n > 0 {
        match splice(*rpipe, to_fd, *n) {
            Ok(x) => *n -= x,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
            Err(err) => return Poll::Ready(Err(err)),
        }
    }

    Poll::Ready(Ok(state.take_read()))
}

pub fn poll_splice(
    cx: &mut Context<'_>,
    state: &mut Option<SpliceState>,
    from: &mut TcpStream,
    to: &mut TcpStream,
) -> Option<Poll<io::Result<usize>>> {
    let from_fd = from.read_passthrough();
    let to_fd = to.write_passthrough();

    if let (Some(Fd::Unix(from_fd)), Some(Fd::Unix(to_fd))) = (from_fd, to_fd) {
        Some(poll_splice_inner(cx, state, from_fd, to_fd, from, to))
    } else {
        None
    }
}
