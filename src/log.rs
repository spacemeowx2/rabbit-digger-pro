use once_cell::sync::OnceCell;
use std::io::Write;
use tokio::sync::broadcast;

static BROADCAST: OnceCell<broadcast::Sender<Box<[u8]>>> = OnceCell::new();

pub fn get_sender() -> &'static broadcast::Sender<Box<[u8]>> {
    BROADCAST.get_or_init(|| {
        let (tx, _) = broadcast::channel::<Box<[u8]>>(32);
        tx
    })
}

pub struct LogWriter {
    sender: broadcast::Sender<Box<[u8]>>,
}

impl Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.sender.send(buf.into()).ok();
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl LogWriter {
    pub fn new() -> Self {
        LogWriter {
            sender: get_sender().clone(),
        }
    }
}

impl Default for LogWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_log_writer_broadcasts() {
        let mut rx = get_sender().subscribe();
        let mut w = LogWriter::new();
        let n = w.write(b"hello").unwrap();
        assert_eq!(n, 5);

        let msg = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&*msg, b"hello");
    }
}
