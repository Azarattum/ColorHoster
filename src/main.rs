#![feature(let_chains)]

mod chunks;
mod config;
mod consts;
mod device;
mod keyboard;

use anyhow::{Result, anyhow};
use chrono::Local;
use clap::Parser;
use fern::colors::{Color, ColoredLevelConfig};
use futures::future;
use itertools::Itertools;
use log::{debug, error, info, warn};
use palette::{encoding::Srgb, rgb::Rgb};
use std::{fs, path::PathBuf, str::FromStr, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Mutex,
};

use consts::{
    DEVICE_TYPE_KEYBOARD, MODE_FLAG_HAS_MODE_SPECIFIC_COLOR, MODE_FLAG_HAS_PER_LED_COLOR,
    MODE_FLAG_HAS_RANDOM_COLOR, OPENRGB_PROTOCOL_VERSION, Request, ZONE_TYPE_MATRIX,
    openrgb_keycode,
};
use keyboard::Keyboard;

/// Color Hoster is OpenRGB compatible high-performance SDK server for VIA per-key RGB
#[derive(Parser, Debug)]
#[command(
    version,
    after_help = "\x1b[1;4mExample:\x1b[0m ./ColorHoster -b -j ./p1_he_ansi_v1.0.json"
)]
struct CLI {
    /// Set a directory to look for VIA `.json` definitions for keyboards (scans current directory by default)
    #[arg(short, long)]
    directory: Option<PathBuf>,

    /// Add a direct path to a VIA `.json` file (can be multiple)
    #[arg(short, long)]
    json: Vec<std::path::PathBuf>,

    /// Allow direct mode to change brightness values
    #[arg(short, long)]
    brightness: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    setup_logger();
    match start().await {
        Err(e) => error!("{}", e),
        _ => (),
    };

    Ok(())
}

async fn start() -> Result<()> {
    let args = CLI::parse();

    let handles: Vec<_> = args
        .directory
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
        .chain(args.json.into_iter())
        .filter_map(|x| fs::read_to_string(x).ok())
        .unique()
        .map(|x| Keyboard::from_str(x))
        .collect();

    if handles.is_empty() {
        return Err(anyhow!("No keyboard `.json` files found!"));
    }

    let keyboards = future::try_join_all(handles).await?;

    debug!("Connected to {}!", keyboards.join(", "));
    let keyboards = Arc::new(
        keyboards
            .into_iter()
            .map(|x| Mutex::new(x))
            .collect::<Vec<_>>(),
    );

    if !args.brightness {
        let handles = keyboards.iter().map(|keyboard| async {
            let mut keyboard = keyboard.lock().await;
            keyboard.reset_brightness().await
        });

        future::try_join_all(handles).await?;
    }

    let port = 6742;
    let address = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&address).await?;
    debug!("Started TCP server at {}!", address);
    info!("The application is running successfully!");

    loop {
        let (stream, _) = listener.accept().await?;

        let keyboard = keyboards.clone();
        tokio::spawn(async move {
            let mut client = "Unknown".to_string();
            let reason = handle_connection(stream, &keyboard, &mut client, args.brightness)
                .await
                .unwrap_err();

            let is_disconnect = reason
                .downcast_ref::<std::io::Error>()
                .map_or(false, |e| e.kind() == std::io::ErrorKind::UnexpectedEof);

            if is_disconnect {
                debug!("Client \x1B[1m{client}\x1B[0m disconnected.");
            } else {
                warn!("\x1B[1m{client}\x1B[0m\x1B[33m disconnected due to an error: {reason}");
            }
        });
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    keyboards: &Vec<Mutex<Keyboard>>,
    client: &mut String,
    with_brightness: bool,
) -> Result<()> {
    loop {
        let magic = stream.read_u32_le().await?;
        if magic != 1111970383 {
            return Err(anyhow!("Invalid packet header!"));
        }

        let device = stream.read_u32_le().await? as usize;
        let kind = stream.read_u32_le().await?;

        handle_request(
            kind,
            device,
            keyboards,
            &mut stream,
            client,
            with_brightness,
        )
        .await?;
    }
}

async fn handle_request(
    kind: u32,
    device: usize,
    keyboards: &Vec<Mutex<Keyboard>>,
    stream: &mut TcpStream,
    client: &mut String,
    with_brightness: bool,
) -> Result<()> {
    let length = stream.read_u32_le().await?;
    let mut keyboard = keyboards
        .get(device)
        .ok_or(anyhow!("Unknown device!"))?
        .lock()
        .await;

    match Request::n(kind) {
        Some(Request::GetControllerCount) => {
            let count: u32 = keyboards.len() as u32;
            stream.write_response(kind, &count.to_le_bytes()).await?;
        }
        Some(Request::GetControllerData) => {
            if length > 0 {
                stream.read_u32_le().await?;
            }

            let config = keyboard.config();
            let id = format!("{:04x}:{:04x}", config.vendor_id, config.product_id);

            let mut buffer = Vec::new();
            buffer.extend_from_slice(&0u32.to_le_bytes()); // Data size (will update later)

            buffer.extend_from_slice(&DEVICE_TYPE_KEYBOARD.to_le_bytes());
            buffer.extend_from_str(&config.name);
            buffer.extend_from_str("Unknown");
            buffer.extend_from_str(&format!("{} via ColorHoster", &config.name));
            buffer.extend_from_str(env!("CARGO_PKG_VERSION"));
            buffer.extend_from_str(&id);
            buffer.extend_from_str(&format!("HID: {}", id));

            buffer.extend_from_slice(&(config.effects.len() as u16).to_le_bytes());
            buffer.extend_from_slice(&(keyboard.effect() as i32).to_le_bytes());

            for (name, id, flags) in &config.effects {
                buffer.extend_from_str(name);

                buffer.extend_from_slice(&id.to_le_bytes());
                buffer.extend_from_slice(&flags.to_le_bytes());
                buffer.extend_from_slice(&config.speed.0.to_le_bytes());
                buffer.extend_from_slice(&config.speed.1.to_le_bytes());
                buffer.extend_from_slice(&config.brightness.0.to_le_bytes());
                buffer.extend_from_slice(&config.brightness.1.to_le_bytes());

                let mode_colors: u32 = 1;
                buffer.extend_from_slice(&mode_colors.to_le_bytes());
                buffer.extend_from_slice(&mode_colors.to_le_bytes());
                buffer.extend_from_slice(&(keyboard.speed() as u32).to_le_bytes());
                buffer.extend_from_slice(&(keyboard.brightness() as u32).to_le_bytes());
                buffer.extend_from_slice(&(0u32).to_le_bytes()); // Direction is constant

                let color_mode = if flags & MODE_FLAG_HAS_PER_LED_COLOR != 0 {
                    1u32
                } else if flags & MODE_FLAG_HAS_MODE_SPECIFIC_COLOR != 0 {
                    2u32
                } else if flags & MODE_FLAG_HAS_RANDOM_COLOR != 0 {
                    3u32
                } else {
                    0u32
                };
                buffer.extend_from_slice(&color_mode.to_le_bytes());

                // TODO: I guess in per-key mode we should send colors for all keys instead (or not, see color section)
                buffer.extend_from_slice(&(mode_colors as u16).to_le_bytes());
                buffer.extend_from_color(&keyboard.color());
            }

            // TODO: should we support more than a single zone?
            buffer.extend_from_slice(&(1u16).to_le_bytes());

            let leds_count = config.count_leds();
            buffer.extend_from_str("Keyboard");
            buffer.extend_from_slice(&ZONE_TYPE_MATRIX.to_le_bytes());
            buffer.extend_from_slice(&leds_count.to_le_bytes());
            buffer.extend_from_slice(&leds_count.to_le_bytes());
            buffer.extend_from_slice(&leds_count.to_le_bytes());

            let matrix_data_size = (config.matrix.0 * config.matrix.1 * 4) + 8;
            buffer.extend_from_slice(&(matrix_data_size as u16).to_le_bytes());
            buffer.extend_from_slice(&config.matrix.1.to_le_bytes());
            buffer.extend_from_slice(&config.matrix.0.to_le_bytes());

            let mut led_matrix = vec![0xFFFFFFFF; (config.matrix.0 * config.matrix.1) as usize];
            for (led, (row, col)) in config.leds.iter().filter_map(|led| *led) {
                led_matrix[row as usize * config.matrix.0 as usize + col as usize] = led as u32;
            }
            buffer.extend_from_u32s(&led_matrix);

            buffer.extend_from_slice(&(leds_count as u16).to_le_bytes());
            let keymap = keyboard.keymap();
            for item in config.leds.iter() {
                if let &Some((led, (row, col))) = item {
                    let scancode = keymap[row as usize * config.matrix.0 as usize + col as usize];
                    buffer.extend_from_str(&format!("Key: {}", openrgb_keycode(scancode)));
                    buffer.extend_from_slice(&(led as u32).to_le_bytes());
                }
            }

            // TODO: are these per-key colors?
            buffer.extend_from_slice(&(leds_count as u16).to_le_bytes());
            for color in keyboard.colors() {
                buffer.extend_from_color(&color);
            }

            let buffer_length = buffer.len() as u32;
            buffer[0..4].copy_from_slice(&buffer_length.to_le_bytes());

            stream.write_response(kind, &buffer).await?;
        }
        Some(Request::GetProtocolVersion) => {
            let _client_version = stream.read_u32_le().await?;
            let version = OPENRGB_PROTOCOL_VERSION.to_le_bytes();
            stream.write_response(kind, &version).await?;
        }
        Some(Request::SetClientName) => {
            let mut name: Vec<u8> = vec![0; length as usize];
            stream.read_exact(&mut name).await?;
            *client = String::from_utf8_lossy(&name).to_string();
            debug!("Client \x1B[1m{}\x1B[0m connected.", client);
        }
        Some(Request::UpdateSingleLed) => {
            let led_index = stream.read_u32_le().await? as usize;
            let rgb = stream.read_rgb().await?;

            keyboard
                .update_colors(vec![rgb], led_index, with_brightness)
                .await?;
        }
        Some(Request::UpdateLeds) => {
            let _data_length = stream.read_u32_le().await?;
            let led_count = stream.read_u16_le().await?;
            let mut colors: Vec<Rgb<Srgb, f32>> = Vec::new();
            for _ in 0..led_count {
                colors.push(stream.read_rgb().await?);
            }

            keyboard.update_colors(colors, 0, with_brightness).await?;
        }
        Some(Request::UpdateMode) => {
            let data_length = stream.read_u32_le().await?;
            let effect = stream.read_i32_le().await? as u8;
            keyboard.update_effect(effect).await?;

            let name_length = stream.read_u16_le().await? as usize;
            let mut buffer = vec![0; data_length as usize - 10];
            stream.read_exact(&mut buffer).await?;

            let speed = buffer.read_u32_le(name_length + 32)?;
            keyboard.update_speed(speed as u8).await?;

            let brightness = buffer.read_u32_le(name_length + 36)?;
            keyboard.update_brightness(brightness as u8).await?;

            if buffer.read_u16_le(name_length + 48)? > 0 {
                let color = buffer.read_rgb(name_length + 50)?;
                keyboard.update_color(color).await?;
            }
        }
        Some(Request::SetCustomMode) => {
            if let Some(effect) = keyboard
                .config()
                .effects
                .iter()
                .find(|x| x.2 & MODE_FLAG_HAS_PER_LED_COLOR != 0)
                .map(|x| x.1 as u8)
            {
                keyboard.update_effect(effect).await?;
            }
        }
        Some(_) => Err(anyhow!("Unknown request id {}!", kind))?,
        None => Err(anyhow!("Unknown request id {}!", kind))?,
    };

    anyhow::Ok(())
}

fn setup_logger() -> () {
    let colors = ColoredLevelConfig::new()
        .info(Color::Green)
        .warn(Color::Yellow)
        .error(Color::Red);

    fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "\x1B[{}m{} \x1B[{}m{} \x1B[0m\x1B[1m[{}]\x1B[0m: \x1B[{}m{} \x1B[0m",
                Color::BrightBlack.to_fg_str(),
                Local::now().format("%H:%M:%S").to_string(),
                colors.get_color(&record.level()).to_fg_str(),
                match record.level() {
                    log::Level::Error => "!",
                    log::Level::Warn => "?",
                    log::Level::Info => "+",
                    log::Level::Debug => "|",
                    log::Level::Trace => "->",
                },
                record.target(),
                colors.get_color(&record.level()).to_fg_str(),
                message
            ))
        })
        .level(log::LevelFilter::Debug)
        .chain(std::io::stdout())
        .apply()
        .unwrap();
}

trait BufferExtensions {
    fn extend_from_str(&mut self, str: &str);
    fn extend_from_color(&mut self, color: &Rgb<Srgb, u8>);
    fn extend_from_u32s(&mut self, values: &[u32]);

    fn read_u32_le(&self, offset: usize) -> Result<u32>;
    fn read_u16_le(&self, offset: usize) -> Result<u16>;
    fn read_rgb(&self, offset: usize) -> Result<Rgb<Srgb, u8>>;
}

impl BufferExtensions for Vec<u8> {
    fn extend_from_str(&mut self, str: &str) {
        self.extend_from_slice(&((str.len() + 1) as u16).to_le_bytes());
        self.extend_from_slice(str.as_bytes());
        self.push(0);
    }

    fn extend_from_color(&mut self, color: &Rgb<Srgb, u8>) {
        self.extend_from_slice(&[color.red, color.green, color.blue, 0]);
    }

    fn extend_from_u32s(&mut self, values: &[u32]) {
        self.extend_from_slice(
            &values
                .iter()
                .flat_map(|x| x.to_le_bytes())
                .collect::<Vec<_>>(),
        );
    }

    fn read_u32_le(&self, offset: usize) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self[offset..offset + 4].try_into().unwrap(),
        ))
    }

    fn read_u16_le(&self, offset: usize) -> Result<u16> {
        Ok(u16::from_le_bytes(
            self[offset..offset + 2].try_into().unwrap(),
        ))
    }

    fn read_rgb(&self, offset: usize) -> Result<Rgb<Srgb, u8>> {
        Ok(Rgb::new(self[offset], self[offset + 1], self[offset + 2]))
    }
}

trait StreamExtensions {
    async fn read_rgb(&mut self) -> Result<Rgb<Srgb, f32>>;
    async fn write_response(&mut self, kind: u32, data: &[u8]) -> Result<()>;
}

impl StreamExtensions for TcpStream {
    async fn read_rgb(&mut self) -> Result<Rgb<Srgb, f32>> {
        let mut buf: [u8; 4] = [0; 4];
        self.read_exact(&mut buf).await?;
        Ok(Rgb::new(buf[0], buf[1], buf[2]).into_format())
    }

    async fn write_response(&mut self, kind: u32, data: &[u8]) -> Result<()> {
        self.write_all(b"ORGB").await?;
        self.write_u32_le(0).await?;
        self.write_u32_le(kind).await?;
        self.write_u32_le(data.len() as u32).await?;
        self.write_all(&data).await?;
        Ok(())
    }
}
