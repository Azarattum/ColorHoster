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
        QMK_COMMAND_UPDATE_BRIGHTNESS, QMK_COMMAND_UPDATE_COLOR, QMK_COMMAND_UPDATE_EFFECT,
        QMK_COMMAND_UPDATE_MATRIX_BRIGHTNESS, QMK_COMMAND_UPDATE_MATRIX_CHROMA,
        QMK_COMMAND_UPDATE_SPEED, QMK_CUSTOM_CHANNEL, QMK_CUSTOM_SET_COMMAND,
        QMK_RGB_MATRIX_CHANNEL, QMK_USAGE_ID, QMK_USAGE_PAGE,
    },
};

pub struct Keyboard {
    config: Config,
    device: Device,

    colors: (Vec<(u8, u8)>, Vec<u8>),
    color: (u8, u8),
    brightness: u8,
    effect: u8,
    speed: u8,
}

impl Keyboard {
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

            // TODO: request from device
            colors: (vec![(0, 0); leds], vec![255; leds]),
            color: (0, 0),
            brightness: 255,
            effect: 0,
            speed: 127,
        })
    }

    pub async fn from_str(json_str: &str) -> Result<Keyboard> {
        let config: Config = Config::from_str(json_str)?;
        return Keyboard::from_config(config).await;
    }

    pub async fn update_colors(
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
                chroma_report[2] = QMK_COMMAND_UPDATE_MATRIX_CHROMA;
                chroma_report[3] = (local_offset + offset) as u8;
                chroma_report[4] = chunk.len() as u8;
                chroma_report[5..(5 + chunk.len() * 2)].copy_from_slice(chunk.as_bytes());
                return chroma_report;
            });

        let brightness_reports = brightness
            .chunk_changed(chunk_size, &self.colors.1[offset..])
            .map(|(local_offset, chunk)| {
                let mut brightness_report = report_template.clone();
                brightness_report[2] = QMK_COMMAND_UPDATE_MATRIX_BRIGHTNESS;
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

    pub fn colors(&self) -> Vec<Rgb<Srgb, u8>> {
        let colors = self.colors.0.iter().zip(&self.colors.1).map(|((h, s), v)| {
            let rgb: Rgb = Hsv::new(*h, *s, *v).into_format().into_color();
            return rgb.into_format();
        });

        return colors.collect();
    }

    pub async fn update_effect(&mut self, effect: u8) -> Result<()> {
        if effect != self.effect {
            self.effect = effect;
            let mut report: [u8; 32] = [0; 32];
            report[0] = QMK_CUSTOM_SET_COMMAND;
            report[1] = QMK_RGB_MATRIX_CHANNEL;
            report[2] = QMK_COMMAND_UPDATE_EFFECT;
            report[3] = effect;
            self.device.write_output_report(&report).await?;
        }
        Ok(())
    }

    pub fn effect(&self) -> u8 {
        self.effect
    }

    pub async fn update_speed(&mut self, speed: u8) -> Result<()> {
        if speed != self.speed {
            self.speed = speed;
            let mut report: [u8; 32] = [0; 32];
            report[0] = QMK_CUSTOM_SET_COMMAND;
            report[1] = QMK_RGB_MATRIX_CHANNEL;
            report[2] = QMK_COMMAND_UPDATE_SPEED;
            report[3] = speed;
            self.device.write_output_report(&report).await?;
        }
        Ok(())
    }

    pub fn speed(&self) -> u8 {
        self.speed
    }

    pub async fn update_brightness(&mut self, brightness: u8) -> Result<()> {
        if brightness != self.brightness {
            self.brightness = brightness;
            let mut report: [u8; 32] = [0; 32];
            report[0] = QMK_CUSTOM_SET_COMMAND;
            report[1] = QMK_RGB_MATRIX_CHANNEL;
            report[2] = QMK_COMMAND_UPDATE_BRIGHTNESS;
            report[3] = brightness;
            self.device.write_output_report(&report).await?;
        }
        Ok(())
    }

    pub fn brightness(&self) -> u8 {
        self.brightness
    }

    pub async fn update_color(&mut self, color: Rgb<Srgb, u8>) -> Result<()> {
        let hsv: Hsv = color.into_format().into_color();
        let hsv = hsv.into_format::<u8>();

        if hsv.hue != self.color.0 || hsv.saturation != self.color.1 {
            self.color = (hsv.hue.into(), hsv.saturation);
            let mut report: [u8; 32] = [0; 32];
            report[0] = QMK_CUSTOM_SET_COMMAND;
            report[1] = QMK_RGB_MATRIX_CHANNEL;
            report[2] = QMK_COMMAND_UPDATE_COLOR;
            report[3] = hsv.hue.into();
            report[4] = hsv.saturation;
            self.device.write_output_report(&report).await?;
        }
        Ok(())
    }

    pub fn color(&self) -> Rgb<Srgb, u8> {
        let rgb: Rgb = Hsv::new(self.color.0, self.color.1, 255)
            .into_format()
            .into_color();
        return rgb.into_format();
    }

    pub fn config(&self) -> &Config {
        &self.config
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
