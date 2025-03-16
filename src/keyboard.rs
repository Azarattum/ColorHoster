use anyhow::{Result, anyhow};
use async_hid::{AccessMode, Device, DeviceInfo};
use futures::future::{self};
use futures_lite::StreamExt;
use palette::{Hsv, IntoColor, rgb::Rgb};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::borrow::Borrow;

use crate::chunks::ChunkChanged;

const QMK_USAGE_PAGE: u16 = 0xFF60;
const QMK_USAGE_ID: u16 = 0x61;

const QMK_CUSTOM_SET_COMMAND: u8 = 0x07;
const QMK_CUSTOM_CHANNEL: u8 = 0x0;
const QMK_COMMAND_UPDATE_HS: u8 = 0x1;
const QMK_COMMAND_UPDATE_BRIGHTNESS: u8 = 0x2;

pub struct Keyboard {
    config: Config,
    device: Device,
    colors: (Vec<(u8, u8)>, Vec<u8>),
}

impl Keyboard {
    pub async fn update_leds(
        &mut self,
        colors: Vec<Rgb>,
        offset: usize,
        with_brightness: bool,
    ) -> Result<()> {
        if offset + colors.len() > self.colors.0.len() {
            return Err(anyhow!("Trying to update more leds than possible!"));
        }

        let hsv_colors = colors.into_iter().map(|rgb| {
            let hsv: Hsv = rgb.into_color();
            return hsv.into_format::<u8>();
        });

        let brightness: Vec<_> = hsv_colors.clone().map(|x| x.value).collect();
        let chroma: Vec<_> = hsv_colors.map(|x| (x.hue.into(), x.saturation)).collect();

        let mut report_template: [u8; 32] = [0; 32];
        report_template[0] = QMK_CUSTOM_SET_COMMAND;
        report_template[1] = QMK_CUSTOM_CHANNEL;
        let chunk_size: usize = (32 - 5) / 2;

        let chroma_reports = chroma
            .chunk_changed(chunk_size, &self.colors.0[offset..])
            .map(|(local_offset, chunk)| {
                let mut chroma_report = report_template.clone();
                chroma_report[2] = QMK_COMMAND_UPDATE_HS;
                chroma_report[3] = (local_offset + offset) as u8;
                chroma_report[4] = chunk.len() as u8;
                chroma_report[5..(5 + chunk.len() * 2)].copy_from_slice(chunk.as_bytes());
                return chroma_report;
            });

        let brightness_reports = brightness
            .chunk_changed(chunk_size, &self.colors.1[offset..])
            .map(|(local_offset, chunk)| {
                let mut brightness_report = report_template.clone();
                brightness_report[2] = QMK_COMMAND_UPDATE_BRIGHTNESS;
                brightness_report[3] = (local_offset + offset) as u8;
                brightness_report[4] = chunk.len() as u8;
                brightness_report[5..(5 + chunk.len())].copy_from_slice(chunk);
                return brightness_report;
            });

        let maybe_brightness_reports = with_brightness
            .then_some(brightness_reports)
            .into_iter()
            .flatten();

        let device = &self.device;
        let handles: Vec<_> = chroma_reports
            .chain(maybe_brightness_reports)
            .map(|report| async move { device.write_output_report(&report).await })
            .collect();

        self.colors.0[offset..offset + chroma.len()].copy_from_slice(&chroma);
        if with_brightness {
            self.colors.1[offset..offset + brightness.len()].copy_from_slice(&brightness);
        }

        let results: Result<Vec<_>, _> = future::join_all(handles).await.into_iter().collect();
        results.map(|_| ()).map_err(|e| e.into())
    }

    pub async fn from_config(config: Config) -> Result<Keyboard> {
        let device = DeviceInfo::enumerate()
            .await?
            .find(|info: &DeviceInfo| {
                info.matches(
                    QMK_USAGE_PAGE,
                    QMK_USAGE_ID,
                    config.vendor_id,
                    config.product_id,
                )
            })
            .await
            .ok_or(anyhow!(
                "{} cannot be detected (VID: {}, PID: {})!",
                config.name,
                config.vendor_id,
                config.product_id
            ))?
            .open(AccessMode::ReadWrite)
            .await?;

        let max_led = config.leds.iter().max().unwrap_or(&Some(0)).unwrap_or(0) + 1;

        Ok(Keyboard {
            config,
            device,
            colors: (vec![(0, 0); max_led as usize], vec![255; max_led as usize]),
        })
    }

    pub async fn from_str(json_str: &str) -> Result<Keyboard> {
        let config: Config = serde_json::from_str(json_str)?;
        return Keyboard::from_config(config).await;
    }
}

impl Borrow<str> for Keyboard {
    fn borrow(&self) -> &str {
        &self.config.name
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    name: String,
    #[serde(rename = "vendorId", deserialize_with = "hex_to_u32")]
    vendor_id: u16,
    #[serde(rename = "productId", deserialize_with = "hex_to_u32")]
    product_id: u16,
    #[serde(rename = "layouts", deserialize_with = "layouts_to_leds")]
    leds: Vec<Option<u8>>,
}

fn hex_to_u32<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    u16::from_str_radix(&s[2..], 16).map_err(serde::de::Error::custom)
}

#[derive(Deserialize)]
struct Layouts {
    keymap: Vec<Vec<Value>>,
}

fn layouts_to_leds<'de, D>(deserializer: D) -> Result<Vec<Option<u8>>, D::Error>
where
    D: Deserializer<'de>,
{
    let layouts = Layouts::deserialize(deserializer)?;
    let mut leds = Vec::new();

    for element in layouts.keymap.iter().flatten() {
        if let Value::String(key) = element {
            let mut flags = key.split('\n');
            let led = flags
                .nth(1)
                .and_then(|x| x.strip_prefix("l"))
                .and_then(|x| x.parse::<u8>().ok())
                .and_then(|x| {
                    // Skip LEDs for encoder keys
                    if let Some(encoder) = flags.nth(7)
                        && encoder.starts_with("e")
                    {
                        return None;
                    }
                    Some(x)
                });

            leds.push(led);
        }
    }

    Ok(leds)
}

trait AsBytes {
    fn as_bytes(&self) -> &[u8];
}

impl AsBytes for &[(u8, u8)] {
    fn as_bytes(&self) -> &[u8] {
        let ptr = self.as_ptr() as *const u8;
        let len = self.len() * 2;
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }
}
