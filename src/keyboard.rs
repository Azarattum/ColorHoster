use anyhow::{Result, anyhow};
use async_hid::{Device, DeviceId};
use futures::future::{self};
use palette::{Hsv, IntoColor, encoding::Srgb, rgb::Rgb};
use serde::{Deserialize, Serialize};
use std::borrow::Borrow;

use crate::{
    chunks::ChunkChanged,
    config::Config,
    consts::{
        QMK_COMMAND_BRIGHTNESS, QMK_COMMAND_COLOR, QMK_COMMAND_EFFECT,
        QMK_COMMAND_MATRIX_BRIGHTNESS, QMK_COMMAND_MATRIX_CHROMA, QMK_COMMAND_SPEED,
        QMK_CUSTOM_CHANNEL, QMK_CUSTOM_GET_COMMAND, QMK_CUSTOM_SAVE_COMMAND,
        QMK_CUSTOM_SET_COMMAND, QMK_KEYMAP_GET_COMMAND, QMK_RGB_MATRIX_CHANNEL,
    },
    device::KeyboardDevice,
};

pub struct Keyboard {
    config: Config,
    keymap: Vec<u16>,
    device: KeyboardDevice<33>, // TODO: make this configurable
    state: KeyboardState,
}

#[derive(Serialize, Deserialize)]
struct KeyboardState {
    colors: (Vec<(u8, u8)>, Vec<u8>),
    color: (u8, u8),
    brightness: u8,
    effect: u8,
    speed: u8,
}

impl Keyboard {
    pub async fn from_config(config: Config, device: Device) -> Result<Keyboard> {
        let device = KeyboardDevice::from_device(device).await?;
        let leds = config.count_leds() as usize;

        let (keymap, colors, color, effect, speed, brightness) = tokio::try_join!(
            Keyboard::load_keymap(&device, (config.matrix.0 * config.matrix.1) as usize),
            Keyboard::load_colors(&device, leds),
            Keyboard::load_color(&device),
            Keyboard::load_effect(&device),
            Keyboard::load_speed(&device),
            Keyboard::load_brightness(&device),
        )?;

        Ok(Keyboard {
            config,
            keymap,
            device,
            state: KeyboardState {
                colors,
                color,
                brightness,
                effect,
                speed,
            },
        })
    }

    pub fn keymap(&self) -> &Vec<u16> {
        &self.keymap
    }

    pub async fn reset_brightness(&mut self) -> Result<()> {
        let device = &self.device;

        let mut report_template = device.create_report();
        report_template[0] = QMK_CUSTOM_SET_COMMAND;
        report_template[1] = QMK_CUSTOM_CHANNEL;
        report_template[2] = QMK_COMMAND_MATRIX_BRIGHTNESS;

        let handles: Vec<_> = vec![255u8; self.state.colors.1.len()]
            .chunk_changed(report_template.len() - 5, &self.state.colors.1)
            .map(|(local_offset, chunk)| {
                let mut report = report_template.clone();
                report[3] = local_offset as u8;
                report[4] = chunk.len() as u8;
                report[5..(5 + chunk.len())].copy_from_slice(chunk);
                return report;
            })
            .map(|report| async move { device.send_report(report).await })
            .collect();

        future::join_all(handles).await;
        Ok(())
    }

    pub async fn update_colors(
        &mut self,
        colors: Vec<Rgb>,
        offset: usize,
        with_brightness: bool,
    ) -> Result<()> {
        if offset + colors.len() > self.state.colors.0.len() {
            return Err(anyhow!("Trying to update more leds than possible!"));
        }

        let hsv_colors = colors.into_iter().map(|rgb| {
            let hsv: Hsv = rgb.into_color();
            return hsv.into_format::<u8>();
        });

        let brightness: Vec<_> = hsv_colors.clone().map(|x| x.value).collect();
        let chroma: Vec<_> = hsv_colors.map(|x| (x.hue.into(), x.saturation)).collect();

        let mut report_template = self.device.create_report();
        report_template[0] = QMK_CUSTOM_SET_COMMAND;
        report_template[1] = QMK_CUSTOM_CHANNEL;

        let chroma_reports = chroma
            .chunk_changed(
                (report_template.len() - 5) / 2,
                &self.state.colors.0[offset..],
            )
            .map(|(local_offset, chunk)| {
                let mut chroma_report = report_template.clone();
                chroma_report[2] = QMK_COMMAND_MATRIX_CHROMA;
                chroma_report[3] = (local_offset + offset) as u8;
                chroma_report[4] = chunk.len() as u8;
                chroma_report[5..(5 + chunk.len() * 2)].copy_from_slice(chunk.as_bytes());
                return chroma_report;
            });

        let brightness_reports = brightness
            .chunk_changed(report_template.len() - 5, &self.state.colors.1[offset..])
            .map(|(local_offset, chunk)| {
                let mut brightness_report = report_template.clone();
                brightness_report[2] = QMK_COMMAND_MATRIX_BRIGHTNESS;
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
            .map(|report| async move { device.send_report(report).await })
            .collect();

        self.state.colors.0[offset..offset + chroma.len()].copy_from_slice(&chroma);
        if with_brightness {
            self.state.colors.1[offset..offset + brightness.len()].copy_from_slice(&brightness);
        }

        future::try_join_all(handles).await?;
        Ok(())
    }

    pub fn colors(&self) -> Vec<Rgb<Srgb, u8>> {
        let colors = self
            .state
            .colors
            .0
            .iter()
            .zip(&self.state.colors.1)
            .map(|((h, s), v)| {
                let rgb: Rgb = Hsv::new(*h, *s, *v).into_format().into_color();
                return rgb.into_format();
            });

        return colors.collect();
    }

    pub async fn update_color(&mut self, color: Rgb<Srgb, u8>) -> Result<()> {
        let hsv: Hsv = color.into_format().into_color();
        let hsv = hsv.into_format::<u8>();

        if hsv.hue != self.state.color.0 || hsv.saturation != self.state.color.1 {
            self.state.color = (hsv.hue.into(), hsv.saturation);
            let mut report = self.device.create_report();
            report[0] = QMK_CUSTOM_SET_COMMAND;
            report[1] = QMK_RGB_MATRIX_CHANNEL;
            report[2] = QMK_COMMAND_COLOR;
            report[3] = hsv.hue.into();
            report[4] = hsv.saturation;
            self.device.send_report(report).await?;
        }
        Ok(())
    }

    pub fn color(&self) -> Rgb<Srgb, u8> {
        let rgb: Rgb = Hsv::new(self.state.color.0, self.state.color.1, 255)
            .into_format()
            .into_color();
        return rgb.into_format();
    }

    pub async fn update_effect(&mut self, effect: u8) -> Result<()> {
        if effect != self.state.effect {
            self.state.effect = effect;
            let mut report = self.device.create_report();
            report[0] = QMK_CUSTOM_SET_COMMAND;
            report[1] = QMK_RGB_MATRIX_CHANNEL;
            report[2] = QMK_COMMAND_EFFECT;
            report[3] = effect;
            self.device.send_report(report).await?;
        }
        Ok(())
    }

    pub fn effect(&self) -> u8 {
        self.state.effect
    }

    pub async fn update_speed(&mut self, speed: u8) -> Result<()> {
        if speed != self.state.speed {
            self.state.speed = speed;
            let mut report = self.device.create_report();
            report[0] = QMK_CUSTOM_SET_COMMAND;
            report[1] = QMK_RGB_MATRIX_CHANNEL;
            report[2] = QMK_COMMAND_SPEED;
            report[3] = speed;
            self.device.send_report(report).await?;
        }
        Ok(())
    }

    pub fn speed(&self) -> u8 {
        self.state.speed
    }

    pub async fn update_brightness(&mut self, brightness: u8) -> Result<()> {
        if brightness != self.state.brightness {
            self.state.brightness = brightness;
            let mut report = self.device.create_report();
            report[0] = QMK_CUSTOM_SET_COMMAND;
            report[1] = QMK_RGB_MATRIX_CHANNEL;
            report[2] = QMK_COMMAND_BRIGHTNESS;
            report[3] = brightness;
            self.device.send_report(report).await?;
        }
        Ok(())
    }

    pub fn brightness(&self) -> u8 {
        self.state.brightness
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn save_state(&self) -> Result<String> {
        serde_json::to_string(&self.state).map_err(|x| x.into())
    }

    pub async fn load_state(&mut self, state: &str, with_brightness: bool) -> Result<()> {
        let state: KeyboardState = serde_json::from_str(state)?;
        let colors: Vec<Rgb> = state
            .colors
            .0
            .iter()
            .zip(state.colors.1)
            .map(|(chroma, brightness)| {
                return Hsv::new(chroma.0, chroma.1, brightness)
                    .into_format::<f32>()
                    .into_color();
            })
            .collect();

        let color: Rgb = Hsv::new(state.color.0, state.color.1, 255)
            .into_format()
            .into_color();

        self.update_colors(colors, 0, with_brightness).await?;
        self.update_color(color.into_format()).await?;
        self.update_effect(state.effect).await?;
        self.update_speed(state.speed).await?;
        self.update_brightness(state.brightness).await?;
        Ok(())
    }

    pub async fn persist_state(&mut self) -> Result<()> {
        let mut report = self.device.create_report();
        report[0] = QMK_CUSTOM_SAVE_COMMAND;
        report[1] = QMK_RGB_MATRIX_CHANNEL;
        self.device.send_report(report).await?;
        Ok(())
    }

    pub fn device_id(&self) -> &DeviceId {
        &self.device.id
    }

    pub fn into_config(self) -> Config {
        self.config
    }

    async fn load_colors<const N: usize>(
        device: &KeyboardDevice<N>,
        count: usize,
    ) -> Result<(Vec<(u8, u8)>, Vec<u8>)> {
        let mut colors = (vec![(0, 0); count], vec![255; count]);

        let mut report_template = device.create_report();
        report_template[0] = QMK_CUSTOM_GET_COMMAND;
        report_template[1] = QMK_CUSTOM_CHANNEL;

        let chroma_chunk_size: usize = (report_template.len() - 5) / 2;
        let chroma_chunks = (count as f32 / chroma_chunk_size as f32).ceil() as usize;
        let chroma_reports = (0..chroma_chunks).map(|i| {
            let mut chroma_report = report_template.clone();
            chroma_report[2] = QMK_COMMAND_MATRIX_CHROMA;
            chroma_report[3] = (i * chroma_chunk_size) as u8;
            chroma_report[4] = chroma_chunk_size.min(count - i * chroma_chunk_size) as u8;
            return chroma_report;
        });

        let brightness_chunk_size: usize = report_template.len() - 5;
        let brightness_chunks = (count as f32 / brightness_chunk_size as f32).ceil() as usize;
        let brightness_reports = (0..brightness_chunks).map(|i| {
            let mut brightness_report = report_template.clone();
            brightness_report[2] = QMK_COMMAND_MATRIX_BRIGHTNESS;
            brightness_report[3] = (i * brightness_chunk_size) as u8;
            brightness_report[4] =
                brightness_chunk_size.min(count - i * brightness_chunk_size) as u8;
            return brightness_report;
        });

        let requests = chroma_reports
            .chain(brightness_reports)
            .map(|report| async move { device.request_report(report, 5).await });

        future::try_join_all(requests)
            .await?
            .into_iter()
            .for_each(|response| {
                let is_brightness = response[2] == QMK_COMMAND_MATRIX_BRIGHTNESS;
                let offset = response[3] as usize;
                let count = response[4] as usize;
                if is_brightness {
                    colors.1[offset..offset + count].copy_from_slice(&response[5..5 + count]);
                } else {
                    colors.0[offset..offset + count]
                        .as_bytes_mut()
                        .copy_from_slice(&response[5..5 + count * 2]);
                }
            });

        Ok(colors)
    }

    async fn load_color<const N: usize>(device: &KeyboardDevice<N>) -> Result<(u8, u8)> {
        let mut report = device.create_report();
        report[0] = QMK_CUSTOM_GET_COMMAND;
        report[1] = QMK_RGB_MATRIX_CHANNEL;
        report[2] = QMK_COMMAND_COLOR;
        let response = device.request_report(report, 3).await?;
        Ok((response[3], response[4]))
    }

    async fn load_effect<const N: usize>(device: &KeyboardDevice<N>) -> Result<u8> {
        let mut report = device.create_report();
        report[0] = QMK_CUSTOM_GET_COMMAND;
        report[1] = QMK_RGB_MATRIX_CHANNEL;
        report[2] = QMK_COMMAND_EFFECT;
        let response = device.request_report(report, 3).await?;
        Ok(response[3])
    }

    async fn load_speed<const N: usize>(device: &KeyboardDevice<N>) -> Result<u8> {
        let mut report = device.create_report();
        report[0] = QMK_CUSTOM_GET_COMMAND;
        report[1] = QMK_RGB_MATRIX_CHANNEL;
        report[2] = QMK_COMMAND_SPEED;
        let response = device.request_report(report, 3).await?;
        Ok(response[3])
    }

    async fn load_brightness<const N: usize>(device: &KeyboardDevice<N>) -> Result<u8> {
        let mut report = device.create_report();
        report[0] = QMK_CUSTOM_GET_COMMAND;
        report[1] = QMK_RGB_MATRIX_CHANNEL;
        report[2] = QMK_COMMAND_BRIGHTNESS;
        let response = device.request_report(report, 3).await?;
        Ok(response[3])
    }

    async fn load_keymap<const N: usize>(
        device: &KeyboardDevice<N>,
        count: usize,
    ) -> Result<Vec<u16>> {
        let mut keymap = vec![0u16; count];

        let mut report_template = device.create_report();
        report_template[0] = QMK_KEYMAP_GET_COMMAND;

        let keymap_chunk_size: usize = report_template.len() - 4;
        let keymap_chunks = ((count * 2) as f32 / keymap_chunk_size as f32).ceil() as usize;

        let requests = (0..keymap_chunks)
            .map(|i| {
                let mut report = report_template.clone();
                let offset = (i * keymap_chunk_size) as u16;
                report[1..3].copy_from_slice(&offset.to_be_bytes());
                report[3] = (keymap_chunk_size.min((count * 2) - i * keymap_chunk_size)) as u8;
                return report;
            })
            .map(|report| async move { device.request_report(report, 4).await });

        future::try_join_all(requests)
            .await?
            .into_iter()
            .for_each(|response| {
                let offset = u16::from_be_bytes(response[1..3].try_into().unwrap()) as usize / 2;
                let count = (response[3] / 2) as usize;
                (0..count).for_each(|i| {
                    keymap[offset + i] =
                        u16::from_be_bytes(response[4 + i * 2..4 + i * 2 + 2].try_into().unwrap());
                });
            });

        Ok(keymap)
    }
}

impl Borrow<str> for Keyboard {
    fn borrow(&self) -> &str {
        &self.config.name
    }
}

trait AsBytes {
    fn as_bytes(&self) -> &[u8];
    fn as_bytes_mut(&mut self) -> &mut [u8];
}

impl AsBytes for [(u8, u8)] {
    fn as_bytes(&self) -> &[u8] {
        let ptr = self.as_ptr() as *const u8;
        let len = self.len() * 2;
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }

    fn as_bytes_mut(&mut self) -> &mut [u8] {
        let ptr = self.as_mut_ptr() as *mut u8;
        let len = self.len() * 2;
        unsafe { std::slice::from_raw_parts_mut(ptr, len) }
    }
}
