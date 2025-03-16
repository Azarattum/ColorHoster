use anyhow::{Result, anyhow};
use async_hid::{AccessMode, Device, DeviceInfo};
use futures::future::{self};
use futures_lite::StreamExt;
use palette::{Hsv, IntoColor, encoding::Srgb, rgb::Rgb};
use std::borrow::Borrow;

use crate::{
    chunks::ChunkChanged,
    config::Config,
    consts::{
        QMK_COMMAND_UPDATE_BRIGHTNESS, QMK_COMMAND_UPDATE_HS, QMK_CUSTOM_CHANNEL,
        QMK_CUSTOM_SET_COMMAND, QMK_USAGE_ID, QMK_USAGE_PAGE,
    },
};

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

        let leds = config.count_leds() as usize;
        Ok(Keyboard {
            config,
            device,
            colors: (vec![(0, 0); leds], vec![255; leds]),
        })
    }

    pub async fn from_str(json_str: &str) -> Result<Keyboard> {
        let config: Config = Config::from_str(json_str)?;
        return Keyboard::from_config(config).await;
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub async fn mode(&self) -> i32 {
        0 // TODO: request from the device
    }

    pub async fn speed(&self) -> u32 {
        63 // TODO: request from the device
    }

    pub async fn brightness(&self) -> u32 {
        255 // TODO: request from the device
    }

    pub async fn color(&self) -> Rgb<Srgb, u8> {
        Rgb::new(255, 255, 255)
    }

    pub fn colors(&self) -> Vec<Rgb<Srgb, u8>> {
        let colors = self.colors.0.iter().zip(&self.colors.1).map(|((h, s), v)| {
            let rgb: Rgb = Hsv::new(*h, *s, *v).into_format().into_color();
            return rgb.into_format();
        });

        return colors.collect();
    }
}

impl Borrow<str> for Keyboard {
    fn borrow(&self) -> &str {
        &self.config.name
    }
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
