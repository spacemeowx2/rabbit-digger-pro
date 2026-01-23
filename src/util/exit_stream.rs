use async_stream::try_stream;
use futures::{future::ready, Stream, StreamExt};
use std::io;
use tokio::signal::ctrl_c;

pub fn exit_stream() -> impl Stream<Item = io::Result<usize>> {
    let signals = try_stream! {
        loop {
            ctrl_c().await?;
            yield ();
        }
    };
    exit_stream_from_signals(signals)
}

fn exit_stream_from_signals<S>(signals: S) -> impl Stream<Item = io::Result<usize>>
where
    S: Stream<Item = io::Result<()>>,
{
    signals.scan(0usize, |times, item| {
        ready(Some(match item {
            Ok(()) => {
                *times += 1;
                Ok(*times)
            }
            Err(e) => Err(e),
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_stream::wrappers::UnboundedReceiverStream;

    #[tokio::test]
    async fn test_exit_stream_counts() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        let signals = UnboundedReceiverStream::new(rx).map(|_| Ok(()));
        let s = exit_stream_from_signals(signals);
        futures::pin_mut!(s);

        tx.send(()).unwrap();
        tx.send(()).unwrap();

        assert_eq!(s.next().await.unwrap().unwrap(), 1);
        assert_eq!(s.next().await.unwrap().unwrap(), 2);
    }

    #[tokio::test]
    async fn test_exit_stream_propagates_error() {
        // Simulate a single failing signal.
        let signals = futures::stream::once(async {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        });
        let s = exit_stream_from_signals(signals);
        futures::pin_mut!(s);

        let err = s.next().await.unwrap().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }
}
