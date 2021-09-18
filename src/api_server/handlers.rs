use std::{convert::Infallible, error::Error, future::ready, path::PathBuf, time::Duration};

use super::reject::{custom_reject, ApiError};
use futures::{pin_mut, SinkExt, Stream, StreamExt, TryStreamExt};
use json_patch::Patch;
use rabbit_digger::{RabbitDigger, Uuid};
use rd_interface::Value;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{
    fs::{create_dir_all, read_to_string, remove_file, File},
    pin,
    time::interval,
};
use tokio_stream::wrappers::IntervalStream;
use tokio_util::codec::{BytesCodec, FramedWrite};
use warp::{
    path::Tail,
    ws::{Message, WebSocket},
    Buf,
};

use crate::config::{ConfigManager, ImportSource};

pub async fn get_config(rd: RabbitDigger) -> Result<impl warp::Reply, warp::Rejection> {
    Ok(warp::reply::json(
        &rd.config()
            .await
            .map_err(|_| warp::reject::custom(ApiError::NotFound))?,
    ))
}

pub async fn post_config(
    rd: RabbitDigger,
    cfg_mgr: ConfigManager,
    source: ImportSource,
) -> Result<impl warp::Reply, warp::Rejection> {
    let stream = cfg_mgr.config_stream(source).await.map_err(custom_reject)?;

    let reply = warp::reply::json(&Value::Null);

    rd.stop().await.map_err(custom_reject)?;
    rd.start_stream(stream).await.map_err(custom_reject)?;

    Ok(reply)
}

pub async fn get_registry(rd: RabbitDigger) -> Result<impl warp::Reply, Infallible> {
    Ok(rd.registry(|r| warp::reply::json(&r)).await)
}

pub async fn get_connection(rd: RabbitDigger) -> Result<impl warp::Reply, Infallible> {
    Ok(rd.connection(|c| warp::reply::json(&c)).await)
}

pub async fn get_state(rd: RabbitDigger) -> Result<impl warp::Reply, warp::Rejection> {
    Ok(warp::reply::json(
        &rd.state_str()
            .await
            .map_err(|_| warp::reject::custom(ApiError::NotFound))?,
    ))
}

#[derive(Debug, Deserialize)]
pub struct UpdateNet {
    net_name: String,
    patch: Patch,
}
pub async fn update_net(
    rd: RabbitDigger,
    cfg_mgr: ConfigManager,
    UpdateNet { net_name, patch }: UpdateNet,
) -> Result<impl warp::Reply, warp::Rejection> {
    rd.update_net(&net_name, move |o| {
        if let Err(e) = json_patch::patch(o, &patch) {
            tracing::warn!("Patch failed: {:?}", e);
        }
    })
    .await
    .map_err(custom_reject)?;

    let patch = rd
        .get_config(|i| {
            i.map(|(left, right)| {
                let patch =
                    json_patch::diff(&serde_json::to_value(left)?, &serde_json::to_value(right)?);
                anyhow::Result::<(String, Patch)>::Ok((left.id.clone(), patch))
            })
        })
        .await;
    if let Some(Ok((id, patch))) = patch {
        cfg_mgr
            .select_storage()
            .set(&id, &serde_json::to_string(&patch).map_err(custom_reject)?)
            .await
            .map_err(custom_reject)?;
    }

    let reply = warp::reply::json(&Value::Null);
    Ok(reply)
}

pub async fn delete_conn(
    rd: RabbitDigger,
    uuid: Uuid,
) -> Result<impl warp::Reply, warp::Rejection> {
    let ok = rd.stop_connection(uuid).await.map_err(custom_reject)?;
    Ok(warp::reply::json(&ok))
}

pub async fn get_userdata(
    mut userdata: PathBuf,
    tail: Tail,
) -> Result<impl warp::Reply, warp::Rejection> {
    // TOOD prevent ".." attack
    userdata.push(tail.as_str());
    let body = read_to_string(userdata)
        .await
        .map_err(|_| warp::reject::custom(ApiError::NotFound))?;
    Ok(warp::reply::json(&json!({ "body": body })))
}

pub async fn put_userdata(
    mut userdata: PathBuf,
    tail: Tail,
    body: impl Stream<Item = Result<impl Buf, warp::Error>>,
) -> Result<impl warp::Reply, warp::Rejection> {
    create_dir_all(&userdata).await.map_err(custom_reject)?;
    // TOOD prevent ".." attack
    userdata.push(tail.as_str());
    let file = File::create(&userdata).await.map_err(custom_reject)?;
    let mut stream = FramedWrite::new(file, BytesCodec::new());
    let mut size = 0;
    pin_mut!(body);
    while let Some(mut chunk) = body.try_next().await.map_err(custom_reject)? {
        let len = chunk.remaining();
        size += len;
        stream
            .send(chunk.copy_to_bytes(len))
            .await
            .map_err(custom_reject)?;
    }
    Ok(warp::reply::json(&json!({ "copied": size })))
}

pub async fn delete_userdata(
    mut userdata: PathBuf,
    tail: Tail,
) -> Result<impl warp::Reply, warp::Rejection> {
    create_dir_all(&userdata).await.map_err(custom_reject)?;
    // TOOD prevent ".." attack
    userdata.push(tail.as_str());

    remove_file(&userdata).await.map_err(custom_reject)?;

    Ok(warp::reply::json(&json!({ "ok": true })))
}

async fn forward<E>(
    sub: impl Stream<Item = Result<Message, E>>,
    mut ws: WebSocket,
) -> anyhow::Result<()>
where
    E: Error + Send + Sync + 'static,
{
    pin!(sub);
    while let Some(item) = sub.try_next().await? {
        ws.send(item).await?;
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct ConnectionQuery {
    #[serde(default)]
    pub patch: bool,
    #[serde(default)]
    pub without_connections: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MaybePatch {
    Full(Value),
    Patch(json_patch::Patch),
}

pub async fn ws_conn(
    rd: RabbitDigger,
    query: ConnectionQuery,
    ws: warp::ws::Ws,
) -> Result<impl warp::Reply, Infallible> {
    let ConnectionQuery {
        patch: patch_mode,
        without_connections,
    } = query;
    let stream = IntervalStream::new(interval(Duration::from_secs(1)));
    let stream = stream
        .then(move |_| {
            let rd = rd.clone();
            async move { rd.connection(|c| serde_json::to_value(c)).await }
        })
        .map_ok(move |mut v| {
            if let (Some(o), true) = (v.as_object_mut(), without_connections) {
                o.remove("connections");
            }
            v
        })
        .scan(Option::<Value>::None, move |last, r| {
            ready(Some(match (patch_mode, r) {
                (true, Ok(x)) => {
                    let r = if let Some(lv) = last {
                        MaybePatch::Patch(json_patch::diff(lv, &x))
                    } else {
                        MaybePatch::Full(x.clone())
                    };
                    *last = Some(x);
                    Ok(r)
                }
                (_, Ok(x)) => Ok(MaybePatch::Full(x)),
                (_, Err(e)) => Err(e),
            }))
        })
        .map_ok(|p| Message::text(serde_json::to_string(&p).unwrap()));
    Ok(ws.on_upgrade(move |ws| async move {
        if let Err(e) = forward(stream, ws).await {
            tracing::error!("WebSocket event error: {:?}", e)
        }
    }))
}
