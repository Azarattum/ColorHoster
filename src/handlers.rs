use anyhow::{Result, anyhow};
use colored::Colorize;
use log::debug;
use palette::{encoding::Srgb, rgb::Rgb};
use std::path::PathBuf;
use tokio::{io::AsyncReadExt, net::TcpStream};
use tokio_util::sync::CancellationToken;

use crate::{
    consts::{
        DEVICE_TYPE_KEYBOARD, MODE_FLAG_HAS_MODE_SPECIFIC_COLOR, MODE_FLAG_HAS_PER_LED_COLOR,
        MODE_FLAG_HAS_RANDOM_COLOR, OPENRGB_PROTOCOL_VERSION, Request, ZONE_TYPE_MATRIX,
        openrgb_keycode,
    },
    keyboards::Keyboards,
    utils::{BufferExt, StreamExt},
};

pub struct HandlerContext {
    pub keyboards: Keyboards,
    pub client: Option<String>,
    pub with_brightness: bool,
    pub profiles_dir: PathBuf,
    pub interrupt: CancellationToken,
}

pub async fn handle(
    request: u32,
    device: u32,
    stream: &mut TcpStream,
    ctx: &mut HandlerContext,
) -> Result<()> {
    let length = stream.read_u32_le().await?;
    let keyboards = ctx.keyboards.items().await;

    match Request::try_from(request).ok() {
        Some(Request::GetProtocolVersion) => {
            let _client_version = stream.read_u32_le().await?;
            let version = OPENRGB_PROTOCOL_VERSION.to_le_bytes();
            stream.write_response(request, &version).await?;
            return Ok(());
        }
        Some(Request::GetControllerCount) => {
            let count: u32 = keyboards.len() as u32;
            stream.write_response(request, &count.to_le_bytes()).await?;
            return Ok(());
        }
        Some(Request::SetClientName) => {
            let mut name: Vec<u8> = vec![0; length as usize];
            stream.read_exact(&mut name).await?;

            let first_time = ctx.client.is_none();
            ctx.client = Some(String::from_utf8_lossy(&name).to_string());
            if first_time {
                debug!("Client {} connected.", ctx.client.clone().unwrap().bold());
            }
            return Ok(());
        }
        _ => {}
    }

    let mut keyboard = keyboards
        .values()
        .nth(device as usize)
        .ok_or(anyhow!("Unknown device!"))?
        .lock()
        .await;

    match Request::try_from(request).ok() {
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

                buffer.extend_from_slice(&(mode_colors as u16).to_le_bytes());
                buffer.extend_from_color(&keyboard.color());
            }

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

            buffer.extend_from_slice(&(leds_count as u16).to_le_bytes());
            for color in keyboard.colors() {
                buffer.extend_from_color(&color);
            }

            let buffer_length = buffer.len() as u32;
            buffer[0..4].copy_from_slice(&buffer_length.to_le_bytes());

            stream.write_response(request, &buffer).await?;
        }
        Some(Request::UpdateSingleLed) => {
            let led_index = stream.read_u32_le().await? as usize;
            let rgb = stream.read_rgb().await?;

            keyboard
                .update_colors(vec![rgb], led_index, ctx.with_brightness)
                .await?;
        }
        Some(Request::UpdateLeds) | Some(Request::UpdateZoneLeds) => {
            let _data_length = stream.read_u32_le().await?;

            if request == Request::UpdateZoneLeds as u32 {
                let _zone = stream.read_u32_le().await?;
            }

            let led_count = stream.read_u16_le().await?;
            let mut colors: Vec<Rgb<Srgb, f32>> = Vec::new();
            for _ in 0..led_count {
                colors.push(stream.read_rgb().await?);
            }

            keyboard
                .update_colors(colors, 0, ctx.with_brightness)
                .await?;
        }
        Some(Request::UpdateMode) | Some(Request::SaveMode) => {
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

            if request == Request::SaveMode as u32 {
                keyboard.persist_state().await?;
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
        Some(Request::SaveProfile) => {
            let profile = stream.read_str(length as usize).await?;
            let path = ctx.profiles_dir.join(format!("{profile}.json"));

            let data = keyboard.save_state()?;
            tokio::fs::write(&path, data).await?;
        }
        Some(Request::LoadProfile) => {
            let profile = stream.read_str(length as usize).await?;
            let path = ctx.profiles_dir.join(format!("{profile}.json"));

            let data = tokio::fs::read_to_string(&path).await?;
            keyboard.load_state(&data, ctx.with_brightness).await?;
        }
        Some(Request::DeleteProfile) => {
            let profile = stream.read_str(length as usize).await?;
            let path = ctx.profiles_dir.join(format!("{profile}.json"));
            tokio::fs::remove_file(&path).await?;
        }
        Some(Request::GetProfileList) => {
            let profiles: Vec<_> = ctx
                .profiles_dir
                .read_dir()?
                .filter_map(|x| x.ok())
                .filter_map(|x| {
                    let name = x.file_name().to_string_lossy().into_owned();
                    Some(name.strip_suffix(".json")?.to_string())
                })
                .collect();

            let mut buffer: Vec<u8> = Vec::new();
            buffer.extend_from_slice(&0u32.to_le_bytes()); // Data size (will update later)
            buffer.extend_from_slice(&(profiles.len() as u16).to_le_bytes());
            for profile in profiles {
                buffer.extend_from_str(&profile);
            }
            let buffer_length = buffer.len() as u32;
            buffer[0..4].copy_from_slice(&buffer_length.to_le_bytes());

            stream.write_response(request, &buffer).await?;
        }
        Some(Request::ResizeZone) => {
            // Keyboards do not support resizing zones, so we just consume the request
            let _zone = stream.read_i32_le().await?;
            let _size = stream.read_i32_le().await?;
        }
        Some(_) => Err(anyhow!("Unknown request id {}!", request))?,
        None => Err(anyhow!("Unknown request id {}!", request))?,
    };

    Ok(())
}
