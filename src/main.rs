#![feature(let_chains)]

mod chunks;
mod config;
mod consts;
mod device;
mod handlers;
mod keyboard;
mod utils;

use anyhow::{Result, anyhow};
use clap::Parser;
use colored::Colorize;
use futures::future;
use handlers::{HandlerContext, handle};
use itertools::Itertools;
use log::{debug, error, info, warn};
use std::{fs, path::PathBuf, str::FromStr, sync::Arc};
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
    sync::Mutex,
};

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
}

#[tokio::main]
async fn main() {
    utils::setup_logger();
    if let Err(error) = listen().await {
        error!("{}", error);
    }
}

async fn listen() -> Result<()> {
    let args = CLI::parse();

    let keyboards = load_keyboards(args.directory, args.json).await?;
    reset_brightness(&keyboards, args.brightness).await?;

    let profiles_dir = args.profiles.unwrap_or_else(|| PathBuf::from("./profiles"));
    tokio::fs::create_dir_all(&profiles_dir).await?;

    let address = format!("127.0.0.1:{}", args.port);
    let listener = TcpListener::bind(&address).await?;
    debug!("Started TCP server at {}!", address);
    info!("The application is running successfully!");

    loop {
        let (stream, _) = listener.accept().await?;

        let mut ctx = HandlerContext {
            keyboards: keyboards.clone(),
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
        let magic = stream.read_u32_le().await?;
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
