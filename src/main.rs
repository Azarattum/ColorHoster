#![feature(let_chains)]

mod chunks;
mod keyboard;

use anyhow::{Result, anyhow};
use enumn::N;
use futures::future;
use keyboard::Keyboard;
use palette::Srgb;
use std::{fs, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Mutex,
};

const OPENRGB_PROTOCOL_VERSION: u32 = 0x3;

#[tokio::main]
async fn main() -> Result<()> {
    let json_str = vec![fs::read_to_string("./p1_he_ansi_v1.0.json")?];
    let handles: Vec<_> = json_str.iter().map(|x| Keyboard::from_str(&x)).collect();
    let keyboards = future::try_join_all(handles).await?;

    println!("Connected to {}!", keyboards.join(", "));
    let keyboards = Arc::new(
        keyboards
            .into_iter()
            .map(|x| Mutex::new(x))
            .collect::<Vec<_>>(),
    );

    let port = 6742;
    let address = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&address).await?;
    println!("Started TCP server at {}!", address);

    loop {
        let (stream, _) = listener.accept().await?;

        let keyboard = keyboards.clone();
        tokio::spawn(async move {
            let reason = handle_connection(stream, &keyboard).await.unwrap_err();
            let is_disconnect = reason
                .downcast_ref::<std::io::Error>()
                .map_or(false, |e| e.kind() == std::io::ErrorKind::UnexpectedEof);

            if is_disconnect {
                println!("Client disconnected!");
            } else {
                println!("Client handling error: {reason}");
            }
        });
    }
}

#[derive(PartialEq, Debug, N)]
#[repr(u32)]
enum Request {
    GetControllerCount = 0,
    GetControllerData = 1,
    GetProtocolVersion = 40,
    SetClientName = 50,
    DeviceListUpdated = 100,
    GetProfileList = 150,
    SaveProfile = 151,
    LoadProfile = 152,
    DeleteProfile = 153,
    ResizeZone = 1000,
    UpdateLeds = 1050,
    UpdateZoneLeds = 1051,
    UpdateSingleLed = 1052,
    SetCustomMode = 1100,
    UpdateMode = 1101,
    SaveMode = 1102,
}

async fn handle_connection(mut stream: TcpStream, keyboards: &Vec<Mutex<Keyboard>>) -> Result<()> {
    loop {
        let magic = stream.read_u32_le().await?;
        if magic != 1111970383 {
            return Err(anyhow!("Invalid packet header!"));
        }

        let device = stream.read_u32_le().await? as usize;
        let kind = stream.read_u32_le().await?;

        handle_request(kind, device, keyboards, &mut stream).await?;
    }
}

async fn handle_request(
    kind: u32,
    device: usize,
    keyboards: &Vec<Mutex<Keyboard>>,
    stream: &mut TcpStream,
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
        Some(Request::GetProtocolVersion) => {
            let _client_version = stream.read_u32_le().await?;
            let version = OPENRGB_PROTOCOL_VERSION.to_le_bytes();
            stream.write_response(kind, &version).await?;
        }
        Some(Request::SetClientName) => {
            let mut name: Vec<u8> = vec![0; length as usize];
            stream.read_exact(&mut name).await?;
            println!("Client {} joined us!", String::from_utf8_lossy(&name));
        }
        Some(Request::UpdateSingleLed) => {
            let led_index = stream.read_u32_le().await? as usize;
            let rgb = stream.read_rgb().await?;

            keyboard.update_leds(vec![rgb], led_index, true).await?;
        }
        Some(Request::UpdateLeds) => {
            let _data_length = stream.read_u32_le().await?;
            let led_count = stream.read_u16_le().await?;
            let mut colors: Vec<Srgb> = Vec::new();
            for _ in 0..led_count {
                colors.push(stream.read_rgb().await?);
            }

            keyboard.update_leds(colors, 0, true).await?;
        }
        Some(_) => todo!("Unhandled request: {}", kind),
        None => todo!("Unhandled request: {}", kind),
    }

    anyhow::Ok(())
}

trait StreamExtensions {
    async fn read_rgb(&mut self) -> Result<Srgb<f32>>;
    async fn write_response(&mut self, kind: u32, data: &[u8]) -> Result<()>;
}

impl StreamExtensions for TcpStream {
    async fn read_rgb(&mut self) -> Result<Srgb<f32>> {
        let mut buf: [u8; 4] = [0; 4];
        self.read_exact(&mut buf).await?;
        Ok(Srgb::new(buf[0], buf[1], buf[2]).into_format())
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
