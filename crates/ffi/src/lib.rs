use rabbit_digger_pro::{config::ImportSource, App};
use std::{ffi::CStr, os::raw::c_char, ptr};
use tokio::{runtime::Runtime, sync::mpsc};
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt};
use tracing_subscriber::{layer::SubscriberExt, prelude::*};

struct RdpRuntime {
    runtime: Runtime,
    sender: mpsc::UnboundedSender<String>,
}

#[repr(transparent)]
pub struct RDP(*mut RdpRuntime);
pub type RESULT = i32;

/// No error.
pub const RESULT_OK: RESULT = 0;
/// Unknown error.
pub const RESULT_ERR_UNKNOWN: RESULT = -1;
/// Utf8 error.
pub const RESULT_ERR_UTF8: RESULT = -2;
/// The other side is closed.
pub const RESULT_ERR_CLOSED: RESULT = -3;

#[no_mangle]
pub extern "C" fn rdp_setup_stdout_logger() {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var(
            "RUST_LOG",
            "rabbit_digger=debug,rabbit_digger_pro=debug,rd_std=debug,raw=debug,ss=debug",
        )
    }

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stdout))
        .init();
}

#[no_mangle]
pub extern "C" fn rdp_run(rabbit_digger: *mut RDP, config: *const c_char) -> RESULT {
    let config = unsafe {
        let config = CStr::from_ptr(config);
        match config.to_str() {
            Ok(s) => s.to_owned(),
            Err(_) => return RESULT_ERR_UTF8,
        }
    };
    let runtime = Runtime::new().expect("Failed to run tokio");
    let (tx, rx) = mpsc::unbounded_channel();
    tx.send(config).expect("Failed to send config");
    match runtime.block_on(async move {
        let app = App::new().await?;

        let rx = UnboundedReceiverStream::new(rx).map(ImportSource::Text);
        let config_stream = Box::pin(app.cfg_mgr.config_stream_from_sources(rx).await?);

        tokio::spawn(async move {
            if let Err(e) = app.rd.start_stream(config_stream).await {
                tracing::error!("start_stream exited with error: {:?}", e);
            }
        });

        Result::<_, anyhow::Error>::Ok(())
    }) {
        Ok(_) => {}
        Err(_) => {
            return RESULT_ERR_UNKNOWN;
        }
    };
    let rt = RdpRuntime {
        runtime,
        sender: tx,
    };
    unsafe {
        *rabbit_digger = RDP(Box::into_raw(Box::new(rt)));
    }
    RESULT_OK
}

#[no_mangle]
pub extern "C" fn rdp_update_config(rabbit_digger: RDP, config: *const c_char) -> RESULT {
    let config = unsafe {
        let config = CStr::from_ptr(config);
        match config.to_str() {
            Ok(s) => s.to_owned(),
            Err(_) => return RESULT_ERR_UTF8,
        }
    };
    let rt: &RdpRuntime = unsafe { &*(rabbit_digger.0 as *const RdpRuntime) };

    match rt.sender.send(config) {
        Ok(_) => RESULT_OK,
        Err(_) => RESULT_ERR_CLOSED,
    }
}

#[no_mangle]
pub extern "C" fn rdp_stop(rabbit_digger: *mut RDP) -> RESULT {
    unsafe {
        let rt = Box::from_raw((*rabbit_digger).0);
        *rabbit_digger = RDP(ptr::null_mut());

        rt.runtime.shutdown_background();
    }
    RESULT_OK
}
