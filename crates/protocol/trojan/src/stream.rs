use rd_interface::{AsyncRead, AsyncWrite};

pub trait IOStream: AsyncRead + AsyncWrite + Unpin + Send + Sync {}

impl<T> IOStream for T where T: AsyncRead + AsyncWrite + Unpin + Send + Sync {}
