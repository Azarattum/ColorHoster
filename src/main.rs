#![feature(let_chains)]

mod chunks;
mod cli;
mod config;
mod consts;
mod device;
mod handlers;
mod keyboard;
mod keyboards;
mod report;
mod utils;

use anyhow::{Result, anyhow};
use ceviche::controller::*;
use ceviche::{Service, ServiceEvent};
use colored::Colorize;
use futures::future;
use itertools::Itertools;
use log::{debug, error, info, warn};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use tokio::runtime::Runtime;
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::CancellationToken;

use cli::{CLI, ServiceAction};
use config::Config;
use consts::Request;
use handlers::{HandlerContext, handle};
use keyboards::Keyboards;
use utils::{ErrorExt, StreamExt};

fn main() {
    let mut controller = Controller::new(
        "colorhoster",
        "Color Hoster",
        "OpenRGB compatible high-performance SDK server for VIA per-key RGB.",
    );

    let args = CLI::parse_args(env::args());

    if let Some(ServiceAction::Create) = args.service {
        utils::setup_logger();
        match args.save_to_config() {
            Err(error) => error!("Failed to write service config: {error}"),
            Ok(true) => debug!("Service config created: {:?}", CLI::config_path()),
            _ => {}
        }
    }

    let result: Result<()> = match args.service {
        Some(ServiceAction::Create) => controller.create().map_err(|x| x.into()),
        Some(ServiceAction::Delete) => controller.delete().map_err(|x| x.into()),
        Some(ServiceAction::Start) => controller.start().map_err(|x| x.into()),
        Some(ServiceAction::Stop) => controller.stop().map_err(|x| x.into()),
        None => controller.register(service_main_wrapper).or_else(|_| {
            let (tx, rx) = mpsc::channel();
            let _tx = tx.clone();

            match ctrlc::set_handler(move || {
                _ = tx.send(ServiceEvent::Stop);
            }) {
                Err(error) => Err(error.into()),
                Ok(()) => {
                    service_main(rx, _tx, env::args().collect(), true);
                    Ok(())
                }
            }
        }),
    };

    if let Err(error) = result {
        utils::setup_logger();
        error!("Error: {error}");
    }
}

Service!("colorhoster", service_main);
fn service_main(
    rx: Receiver<ServiceEvent<()>>,
    _tx: Sender<ServiceEvent<()>>,
    args: Vec<String>,
    _standalone_mode: bool,
) -> u32 {
    utils::setup_logger();
    let args = CLI::parse_args(args);
    let interrupt = CancellationToken::new();
    let runtime = Runtime::new().expect("Failed to create async runtime!");

    let result = runtime.block_on(async {
        let service_task = tokio::spawn(run(args, interrupt.clone()));
        let stop_monitor = tokio::task::spawn_blocking(move || {
            while let Ok(ServiceEvent::Stop) = rx.recv() {
                interrupt.cancel();
                break;
            }
        });

        tokio::pin!(service_task);
        tokio::select! {
            result = &mut service_task => result,
            _ = stop_monitor => service_task.await,
        }
    });

    runtime.shutdown_background();

    match result {
        Ok(Ok(())) => return 0,
        Ok(Err(error)) => {
            error!("Error: {}", error);
            return 1;
        }
        Err(error) => {
            error!("Task execution failed: {}", error);
            return 1;
        }
    }
}

async fn run(args: CLI, interrupt: CancellationToken) -> Result<()> {
    let keyboards = load_keyboards(args.directory, args.json).await?;
    reset_brightness(&keyboards, args.brightness).await?;

    let profiles_dir = args
        .profiles
        .unwrap_or_else(|| CLI::current_dir().join(PathBuf::from("./profiles")));

    tokio::fs::create_dir_all(&profiles_dir).await?;

    let address = format!("127.0.0.1:{}", args.port);
    let listener = TcpListener::bind(&address).await?;
    debug!("Started TCP server at {}!", address);
    info!("The application is running successfully!");

    loop {
        let (stream, _) = tokio::select! {
            client = listener.accept() => client?,
            _ = interrupt.cancelled() => return Ok(()),
        };

        let mut ctx = HandlerContext {
            client: None,
            keyboards: keyboards.clone(),
            interrupt: interrupt.clone(),
            with_brightness: args.brightness,
            profiles_dir: profiles_dir.clone(),
        };

        tokio::spawn(async move {
            match handle_connection(stream, &mut ctx).await {
                Err(error) if error.is_disconnect() => {
                    debug!(
                        "Client {} disconnected.",
                        ctx.client.unwrap_or("Unknown".to_string()).bold()
                    )
                }
                Err(error) => warn!(
                    "{}\x1B[33m disconnected due to an error: {error}",
                    ctx.client.unwrap_or("Unknown".to_string()).bold()
                ),
                Ok(()) => (),
            }
        });
    }
}

async fn handle_connection(mut stream: TcpStream, ctx: &mut HandlerContext) -> Result<()> {
    let mut device_notification = ctx.keyboards.subscribe();

    loop {
        let magic = tokio::select! {
            data = stream.read_u32_le() => data?,
            _ = ctx.interrupt.cancelled() => return Ok(()),
            _ = device_notification.recv() => {
                stream.write_response(Request::DeviceListUpdated.into(), &[]).await?;
                continue;
            }
        };
        if magic != 1111970383 {
            return Err(anyhow!("Invalid packet header!"));
        }

        let device = stream.read_u32_le().await?;
        let kind = stream.read_u32_le().await?;

        handle(kind, device, &mut stream, ctx).await?;
    }
}

async fn load_keyboards(directory: Option<PathBuf>, json: Vec<PathBuf>) -> Result<Keyboards> {
    let configs = directory
        .unwrap_or(CLI::current_dir())
        .read_dir()?
        .filter_map(|path| {
            let path = path.as_ref().ok()?.path();
            if path.extension()?.to_str() == Some("json") {
                Some(path)
            } else {
                None
            }
        })
        .chain(json.into_iter())
        .filter_map(|x| fs::read_to_string(x).ok())
        .unique()
        .map(|x| Config::from_str(&x).map(|config| ((config.vendor_id, config.product_id), config)))
        .collect::<Result<HashMap<_, _>>>()?;

    if configs.is_empty() {
        return Err(anyhow!("No keyboard `.json` files found!"));
    }

    let keyboards = Keyboards::from_configs(configs).await?;
    keyboards.watch()?;
    Ok(keyboards)
}

async fn reset_brightness(keyboards: &Keyboards, with_brightness: bool) -> Result<()> {
    if !with_brightness {
        let keyboards = keyboards.items().await;
        let handles = keyboards.values().map(|keyboard| async {
            let mut keyboard = keyboard.lock().await;
            keyboard.reset_brightness().await
        });

        future::try_join_all(handles).await?;
    }

    Ok(())
}
