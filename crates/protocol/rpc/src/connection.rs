use std::{fmt::Debug, io};

use crate::types::{Request, Response};
use rd_interface::TcpStream;
use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    io::{split, AsyncReadExt, AsyncWriteExt},
    sync::{
        mpsc::{channel, Receiver, Sender},
        Mutex,
    },
    task::JoinHandle,
};

// 1MB
const MAX_ITEM_SIZE: u32 = 1 * 1024 * 1024;
const MAX_DATA_SIZE: u32 = 1 * 1024 * 1024;

#[derive(Copy, Clone)]
pub enum Codec {
    Json,
    Cbor,
}

pub struct Connection<Item, SinkItem> {
    read_rx: Mutex<Receiver<(Item, Vec<u8>)>>,
    write_tx: Sender<(SinkItem, Option<Vec<u8>>)>,

    read_task: JoinHandle<io::Result<()>>,
    write_task: JoinHandle<io::Result<()>>,
}
pub type ClientConnection = Connection<Response, Request>;
pub type ServerConnection = Connection<Request, Response>;

fn map_err<E>(e: E) -> io::Error
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    io::Error::new(io::ErrorKind::Other, e)
}

impl<Item, SinkItem> Connection<Item, SinkItem>
where
    Item: Serialize + DeserializeOwned + Unpin + Debug + Send + 'static,
    SinkItem: Serialize + DeserializeOwned + Unpin + Debug + Send + Sync + 'static,
{
    pub fn new(tcp: TcpStream, codec: Codec) -> Connection<Item, SinkItem> {
        let (mut reader, mut writer) = split(tcp);
        let (read_tx, read_rx) = channel::<(Item, Vec<u8>)>(1);
        let (write_tx, mut write_rx) = channel::<(SinkItem, Option<Vec<u8>>)>(1);

        let read_task = tokio::spawn(async move {
            while !read_tx.is_closed() {
                let item_size = reader.read_u32().await?;
                let data_size = reader.read_u32().await?;

                if item_size > MAX_ITEM_SIZE || data_size > MAX_DATA_SIZE {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "item size {} or data size {} is too big",
                            item_size, data_size
                        ),
                    ));
                }

                let mut item_buf = vec![0; item_size as usize];
                reader.read_exact(&mut item_buf).await?;
                let mut data_buf = vec![0; data_size as usize];
                if data_size > 0 {
                    reader.read_exact(&mut data_buf).await?;
                }

                let item = match codec {
                    Codec::Cbor => cbor4ii::serde::from_slice(&item_buf).map_err(map_err)?,
                    Codec::Json => serde_json::from_slice(&item_buf).map_err(map_err)?,
                };

                // No receiver, exit normally
                if let Err(_) = read_tx.send((item, data_buf)).await {
                    break;
                }
            }
            Ok(())
        });

        let write_task = tokio::spawn(async move {
            while let Some((item, data)) = write_rx.recv().await {
                let item_buf = match codec {
                    Codec::Cbor => cbor4ii::serde::to_vec(Vec::new(), &item).map_err(map_err)?,
                    Codec::Json => serde_json::to_vec(&item).map_err(map_err)?,
                };
                let data_size = data.as_ref().map(|d| d.len() as u32).unwrap_or(0);
                let item_size = item_buf.len() as u32;

                writer.write_u32(item_size).await?;
                writer.write_u32(data_size).await?;
                writer.write_all(&item_buf).await?;
                if let Some(data) = data {
                    writer.write_all(&data).await?;
                }
                writer.flush().await?;
            }
            Ok(())
        });

        Connection {
            read_rx: Mutex::new(read_rx),
            write_tx,
            read_task,
            write_task,
        }
    }
    pub async fn send(&self, req: SinkItem, data: Option<Vec<u8>>) -> io::Result<()> {
        self.write_tx
            .send((req, data))
            .await
            .map_err(|_| io::ErrorKind::BrokenPipe.into())
    }
    pub async fn next(&self) -> io::Result<(Item, Vec<u8>)> {
        self.read_rx
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| io::ErrorKind::BrokenPipe.into())
    }
    pub async fn close(&self) -> io::Result<()> {
        self.read_task.abort();
        self.write_task.abort();

        Ok(())
    }
}
