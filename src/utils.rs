use std::io::IsTerminal;

use anyhow::{Error, Result};
use chrono::Local;
use colored::Colorize;
use fern::{
    Output,
    colors::{Color, ColoredLevelConfig},
    log_file,
};
use palette::{encoding::Srgb, rgb::Rgb};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

#[cfg(windows)]
const LOG_FILE: &'static str = "C:\\Windows\\Temp\\colorhoster.log";
#[cfg(any(unix, target_os = "macos"))]
const LOG_FILE: &'static str = "/tmp/colorhoster.log";

pub fn setup_logger() -> () {
    let colors = ColoredLevelConfig::new()
        .info(Color::Green)
        .warn(Color::Yellow)
        .error(Color::Red);

    let output = if std::io::stdout().is_terminal() {
        Output::from(std::io::stdout())
    } else {
        Output::from(log_file(LOG_FILE).expect("Failed to open log file!"))
    };

    fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "{} \x1B[{}m{} \x1B[0m[{}]: \x1B[{}m{} \x1B[0m",
                Local::now().format("%H:%M:%S").to_string().bright_black(),
                colors.get_color(&record.level()).to_fg_str(),
                match record.level() {
                    log::Level::Error => "!",
                    log::Level::Warn => "?",
                    log::Level::Info => "+",
                    log::Level::Debug => "|",
                    log::Level::Trace => "->",
                },
                record.target().split(":").next().unwrap().bold(),
                colors.get_color(&record.level()).to_fg_str(),
                message
            ))
        })
        .level(log::LevelFilter::Debug)
        .chain(output)
        .apply()
        .unwrap();
}

pub trait BufferExt {
    fn extend_from_str(&mut self, str: &str);
    fn extend_from_color(&mut self, color: &Rgb<Srgb, u8>);
    fn extend_from_u32s(&mut self, values: &[u32]);

    fn read_u32_le(&self, offset: usize) -> Result<u32>;
    fn read_u16_le(&self, offset: usize) -> Result<u16>;
    fn read_rgb(&self, offset: usize) -> Result<Rgb<Srgb, u8>>;
}

impl BufferExt for Vec<u8> {
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

pub trait StreamExt {
    async fn read_rgb(&mut self) -> Result<Rgb<Srgb, f32>>;
    async fn write_response(&mut self, kind: u32, data: &[u8]) -> Result<()>;
    async fn read_str(&mut self, len: usize) -> Result<String>;
}

impl StreamExt for TcpStream {
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

    async fn read_str(&mut self, len: usize) -> Result<String> {
        let mut buf: Vec<u8> = vec![0; len];
        self.read_exact(&mut buf).await?;
        Ok(String::from_utf8_lossy(&buf[..len - 1]).to_string())
    }
}

pub trait ErrorExt {
    fn is_disconnect(&self) -> bool;
}

impl ErrorExt for Error {
    fn is_disconnect(&self) -> bool {
        self.downcast_ref::<std::io::Error>().map_or(false, |e| {
            e.kind() == std::io::ErrorKind::UnexpectedEof
                || e.kind() == std::io::ErrorKind::ConnectionReset
        })
    }
}
