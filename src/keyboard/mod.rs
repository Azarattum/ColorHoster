mod chunks;
mod device;
mod keyboard;

use anyhow::Result;
use async_hid::{Device, DeviceId};
use colored::Colorize;
use indexmap::IndexMap;
use log::warn;
use palette::{encoding::Srgb, rgb::Rgb};
use std::{
    cmp::{max, min},
    mem::{self, Discriminant},
    sync::{Arc, Mutex},
};
use tokio::sync::{Mutex as AsyncMutex, Notify};

use crate::{config::Config, keyboard::keyboard::KeyboardController};

#[derive(Clone)]
pub struct Keyboard {
    actions: Arc<Mutex<IndexMap<Discriminant<KeyboardAction>, KeyboardAction>>>,
    keyboard: Arc<AsyncMutex<KeyboardController>>,
    notifier: Notifier,
}

impl Keyboard {
    pub async fn from_config(config: Config, device: Device) -> Result<Keyboard> {
        let keyboard = KeyboardController::from_config(config, device).await?;
        let keyboard = Arc::new(AsyncMutex::new(keyboard));
        let weak_keyboard = Arc::downgrade(&keyboard);

        let notifier = Notifier {
            notify: Arc::new(Notify::new()),
        };

        let actions = Arc::new(Mutex::new(IndexMap::<
            Discriminant<KeyboardAction>,
            KeyboardAction,
        >::new()));

        let handler_actions = actions.clone();
        let handler_notify = notifier.clone();
        tokio::spawn(async move {
            'handle: loop {
                handler_notify.notify.notified().await;

                'drain: loop {
                    let keyboard = match weak_keyboard.upgrade() {
                        Some(keyboard) => keyboard,
                        None => break 'handle,
                    };

                    let action = {
                        let mut actions = handler_actions.lock().unwrap();
                        match actions.shift_remove_index(0) {
                            Some((_, action)) => action,
                            None => break 'drain,
                        }
                    };

                    let mut keyboard = keyboard.lock().await;
                    let action_name = action.as_name();

                    if let Err(error) = handle_action(action, &mut keyboard).await {
                        warn!(
                            "{}\x1B[33m failed to execute action {}\x1B[33m: {error}",
                            keyboard.config().name.bold(),
                            action_name.bold(),
                        )
                    }
                }
            }
        });

        Ok(Keyboard {
            keyboard,
            actions,
            notifier,
        })
    }

    fn perform_action(&self, action: KeyboardAction) {
        let mut actions = self.actions.lock().unwrap();
        let id = mem::discriminant(&action);

        let action = match (actions.shift_remove(&id), action) {
            (
                Some(KeyboardAction::UpdateColors(colors_old, offset_old, _)),
                KeyboardAction::UpdateColors(colors_new, offset_new, with_brightness),
            ) => {
                let (colors, offset) =
                    merge_colors(colors_old, offset_old as i32, colors_new, offset_new as i32);
                KeyboardAction::UpdateColors(colors, offset, with_brightness)
            }
            (_, action) => action,
        };

        actions.insert(id, action);
        self.notifier.notify.notify_one();
    }

    pub async fn keymap(&self) -> Vec<u16> {
        self.keyboard.lock().await.keymap().clone()
    }

    pub fn reset_brightness(&self) {
        self.perform_action(KeyboardAction::ResetBrightness);
    }

    pub fn update_colors(&self, colors: Vec<Option<Rgb>>, offset: usize, with_brightness: bool) {
        self.perform_action(KeyboardAction::UpdateColors(
            colors,
            offset,
            with_brightness,
        ));
    }

    pub async fn colors(&self) -> Vec<Rgb<Srgb, u8>> {
        self.keyboard.lock().await.colors()
    }

    pub fn update_color(&self, color: Rgb<Srgb, u8>) {
        self.perform_action(KeyboardAction::UpdateColor(color));
    }

    pub async fn color(&self) -> Rgb<Srgb, u8> {
        self.keyboard.lock().await.color()
    }

    pub fn update_effect(&self, effect: u8) {
        self.perform_action(KeyboardAction::UpdateEffect(effect));
    }

    pub async fn effect(&self) -> u8 {
        self.keyboard.lock().await.effect()
    }

    pub fn update_speed(&self, speed: u8) {
        self.perform_action(KeyboardAction::UpdateSpeed(speed));
    }

    pub async fn speed(&self) -> u8 {
        self.keyboard.lock().await.speed()
    }

    pub fn update_brightness(&self, brightness: u8) {
        self.perform_action(KeyboardAction::UpdateBrightness(brightness));
    }

    pub async fn brightness(&self) -> u8 {
        self.keyboard.lock().await.brightness()
    }

    pub async fn config(&self) -> Config {
        self.keyboard.lock().await.config().clone()
    }

    pub async fn save_state(&self) -> Result<String> {
        self.keyboard.lock().await.save_state()
    }

    pub fn load_state(&self, state: String, with_brightness: bool) {
        self.perform_action(KeyboardAction::LoadState(state, with_brightness));
    }

    pub fn persist_state(&self) {
        self.perform_action(KeyboardAction::PersistState);
    }

    pub async fn device_id(&self) -> DeviceId {
        self.keyboard.lock().await.device_id().clone()
    }

    pub async fn into_config(self) -> Config {
        Arc::try_unwrap(self.keyboard)
            .unwrap()
            .into_inner()
            .into_config()
    }
}

pub async fn handle_action(
    action: KeyboardAction,
    keyboard: &mut KeyboardController,
) -> Result<()> {
    match action {
        KeyboardAction::UpdateColors(colors, offset, with_brightness) => {
            keyboard
                .update_colors(colors, offset, with_brightness)
                .await
        }
        KeyboardAction::LoadState(data, with_brightness) => {
            keyboard.load_state(&data, with_brightness).await
        }
        KeyboardAction::UpdateBrightness(brightness) => {
            keyboard.update_brightness(brightness).await
        }
        KeyboardAction::UpdateEffect(effect) => keyboard.update_effect(effect).await,
        KeyboardAction::UpdateColor(color) => keyboard.update_color(color).await,
        KeyboardAction::UpdateSpeed(speed) => keyboard.update_speed(speed).await,
        KeyboardAction::PersistState => keyboard.persist_state().await,
        KeyboardAction::ResetBrightness => keyboard.reset_brightness().await,
    }
}

#[derive(Debug, Clone)]
pub enum KeyboardAction {
    UpdateColors(Vec<Option<Rgb>>, usize, bool),
    UpdateEffect(u8),
    UpdateSpeed(u8),
    UpdateBrightness(u8),
    UpdateColor(Rgb<Srgb, u8>),
    LoadState(String, bool),
    PersistState,
    ResetBrightness,
}

impl KeyboardAction {
    fn as_name(&self) -> &'static str {
        match self {
            KeyboardAction::UpdateColors(_, _, _) => "UpdateColors",
            KeyboardAction::UpdateEffect(_) => "UpdateEffect",
            KeyboardAction::UpdateSpeed(_) => "UpdateSpeed",
            KeyboardAction::UpdateBrightness(_) => "UpdateBrightness",
            KeyboardAction::UpdateColor(_) => "UpdateColor",
            KeyboardAction::LoadState(_, _) => "LoadState",
            KeyboardAction::PersistState => "PersistState",
            KeyboardAction::ResetBrightness => "ResetBrightness",
        }
    }
}

#[derive(Clone)]
pub struct Notifier {
    pub notify: Arc<Notify>,
}

impl Drop for Notifier {
    fn drop(&mut self) {
        self.notify.notify_one();
    }
}

fn merge_colors(
    colors_old: Vec<Option<Rgb>>,
    offset_old: i32,
    colors_new: Vec<Option<Rgb>>,
    offset_new: i32,
) -> (Vec<Option<Rgb>>, usize) {
    let (mut colors_left, offset_left, colors_right, offset_right, new_left) =
        if offset_old < offset_new {
            (colors_old, offset_old, colors_new, offset_new, false)
        } else {
            (colors_new, offset_new, colors_old, offset_old, true)
        };

    let (left_len, right_len) = (colors_left.len() as i32, colors_right.len() as i32);

    let gap = offset_right - (offset_left + left_len);
    let new_len = (left_len + gap + right_len) as usize;

    let offset = (if new_left {
        max(left_len + gap, left_len)
    } else {
        left_len + gap
    }) as usize;

    let count = (if new_left {
        min(right_len + gap, right_len)
    } else {
        right_len
    }) as usize;

    colors_left.resize(new_len, None);
    colors_left[offset..offset + count]
        .copy_from_slice(&colors_right[right_len as usize - count..right_len as usize]);

    return (colors_left, offset_left as usize);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_non_overlapping() {
        let red = Some(Rgb::new(1., 0., 0.));
        let blue = Some(Rgb::new(0., 0., 1.));

        let (colors, offset) = merge_colors(vec![red, red, red], 2, vec![blue, blue, blue], 5);
        assert_eq!(colors, vec![red, red, red, blue, blue, blue]);
        assert_eq!(offset, 2);

        let (colors, offset) = merge_colors(vec![red, red, red], 5, vec![blue, blue, blue], 2);
        assert_eq!(colors, vec![blue, blue, blue, red, red, red]);
        assert_eq!(offset, 2);
    }

    #[test]
    fn merges_with_gap() {
        let red = Some(Rgb::new(1., 0., 0.));
        let blue = Some(Rgb::new(0., 0., 1.));

        let (colors, offset) = merge_colors(vec![red, red, red], 2, vec![blue, blue, blue], 7);
        assert_eq!(colors, vec![red, red, red, None, None, blue, blue, blue]);
        assert_eq!(offset, 2);

        let (colors, offset) = merge_colors(vec![red, red, red], 7, vec![blue, blue, blue], 2);
        assert_eq!(colors, vec![blue, blue, blue, None, None, red, red, red]);
        assert_eq!(offset, 2);
    }

    #[test]
    fn merges_with_overlap() {
        let red = Some(Rgb::new(1., 0., 0.));
        let blue = Some(Rgb::new(0., 0., 1.));

        let (colors, offset) = merge_colors(vec![red, red, red], 2, vec![blue, blue, blue], 4);
        assert_eq!(colors, vec![red, red, blue, blue, blue]);
        assert_eq!(offset, 2);

        let (colors, offset) = merge_colors(vec![red, red, red], 4, vec![blue, blue, blue], 2);
        assert_eq!(colors, vec![blue, blue, blue, red, red]);
        assert_eq!(offset, 2);
    }

    #[test]
    fn full_overwrite() {
        let red = Some(Rgb::new(1., 0., 0.));
        let blue = Some(Rgb::new(0., 0., 1.));

        let (colors, offset) = merge_colors(vec![red, red, red], 2, vec![blue, blue, blue], 2);
        assert_eq!(colors, vec![blue, blue, blue]);
        assert_eq!(offset, 2);
    }
}
