#![feature(let_chains)]

mod chunks;
mod config;
mod consts;
mod device;
mod handlers;
mod keyboard;
mod utils;

use anyhow::{Result, anyhow};
use ceviche::controller::*;
use ceviche::{Service, ServiceEvent};
use clap::{Parser, ValueEnum};
use colored::Colorize;
use futures::future;
use handlers::{HandlerContext, handle};
use itertools::Itertools;
use log::{debug, error, info, warn};
use std::env;
use std::sync::mpsc::{self, Receiver, Sender};
use std::{fs, path::PathBuf, str::FromStr, sync::Arc};
use tokio::runtime::Runtime;
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;

use keyboard::Keyboard;
use utils::ErrorExt;

/// Color Hoster is OpenRGB compatible high-performance SDK server for VIA per-key RGB
#[derive(Parser, Debug)]
#[command(
    version,
    after_help = format!("{} ./ColorHoster -b -j ./p1_he_ansi_v1.0.json", "Example:".bold())
)]
struct CLI {
    /// Set a directory to look for VIA `.json` definitions for keyboards [default: ./]
    #[arg(short, long)]
    directory: Option<PathBuf>,

    /// Add a direct path to a VIA `.json` file (can be multiple)
    #[arg(short, long)]
    json: Vec<std::path::PathBuf>,

    /// Allow direct mode to change brightness values
    #[arg(short, long)]
    brightness: bool,

    /// Set a directory for storing and loading profiles [default: ./profiles]
    #[arg(long)]
    profiles: Option<PathBuf>,

    /// Set the port to listen on
    #[arg(short, long, default_value_t = 6742)]
    port: u32,

    /// Manage Color Hoster service
    #[arg(short, long)]
    service: Option<ServiceAction>,
}

#[derive(Clone, Debug, ValueEnum)]
enum ServiceAction {
    Create,
    Delete,
    Start,
    Stop,
}

fn main() {
    let mut controller = Controller::new(
        "colorhoster",
        "Color Hoster",
        "OpenRGB compatible high-performance SDK server for VIA per-key RGB.",
    );

    let result: Result<()> = match CLI::parse().service {
        Some(ServiceAction::Create) => controller.create().map_err(|x| x.into()),
        Some(ServiceAction::Delete) => controller.delete().map_err(|x| x.into()),
        Some(ServiceAction::Start) => controller.start().map_err(|x| x.into()),
        Some(ServiceAction::Stop) => controller.stop().map_err(|x| x.into()),
        None => {
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
        }
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
    let args = CLI::parse_from(args);
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

    let profiles_dir = args.profiles.unwrap_or_else(|| PathBuf::from("./profiles"));
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
            keyboards: keyboards.clone(),
            interrupt: interrupt.clone(),
            client: "Unknown".to_string(),
            with_brightness: args.brightness,
            profiles_dir: profiles_dir.clone(),
        };

        tokio::spawn(async move {
            let error = handle_connection(stream, &mut ctx).await.unwrap_err();

            if error.is_disconnect() {
                debug!("Client {} disconnected.", ctx.client.bold());
            } else {
                warn!(
                    "{} disconnected due to an error: {error}",
                    ctx.client.bold()
                );
            }
        });
    }
}

async fn handle_connection(mut stream: TcpStream, ctx: &mut HandlerContext) -> Result<()> {
    loop {
        let magic = tokio::select! {
            data = stream.read_u32_le() => data?,
            _ = ctx.interrupt.cancelled() => return Ok(()),
        };
        if magic != 1111970383 {
            return Err(anyhow!("Invalid packet header!"));
        }

        let device = stream.read_u32_le().await?;
        let kind = stream.read_u32_le().await?;

        handle(kind, device, &mut stream, ctx).await?;
    }
}

async fn load_keyboards(
    directory: Option<PathBuf>,
    json: Vec<PathBuf>,
) -> Result<Arc<Vec<Mutex<Keyboard>>>> {
    let handles: Vec<_> = directory
        .unwrap_or(PathBuf::from_str(".").unwrap())
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
        .map(|x| Keyboard::from_str(x))
        .collect();

    if handles.is_empty() {
        return Err(anyhow!("No keyboard `.json` files found!"));
    }

    let keyboards = future::try_join_all(handles).await?;

    debug!("Connected keyboards: {}", keyboards.join(", "));
    Ok(Arc::new(keyboards.into_iter().map(Mutex::new).collect()))
}

async fn reset_brightness(
    keyboards: &Arc<Vec<Mutex<Keyboard>>>,
    with_brightness: bool,
) -> Result<()> {
    if !with_brightness {
        let handles = keyboards.iter().map(|keyboard| async {
            let mut keyboard = keyboard.lock().await;
            keyboard.reset_brightness().await
        });

        future::try_join_all(handles).await?;
    }

    Ok(())
}
