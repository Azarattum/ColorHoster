use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;

use crate::consts::{MODE_FLAG_HAS_BRIGHTNESS, MODE_FLAG_HAS_PER_LED_COLOR, MODE_FLAG_HAS_SPEED};

#[derive(Debug)]
pub struct Config {
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub leds: Vec<Option<(u8, (u8, u8))>>,
    pub effects: Vec<(String, i32, u32)>,
    pub speed: (u32, u32),
    pub brightness: (u32, u32),
    pub matrix: (u32, u32),
}

impl Config {
    pub fn from_str(json_str: &str) -> Result<Config> {
        let keyboard_json: KeyboardJson = serde_json::from_str(json_str)?;

        let vendor_id = parse_hex(&keyboard_json.vendor_id);
        let product_id = parse_hex(&keyboard_json.product_id);
        let matrix = (keyboard_json.matrix.cols, keyboard_json.matrix.rows);

        let leds: Vec<_> = keyboard_json
            .layouts
            .keymap
            .iter()
            .flatten()
            .filter_map(|entry| match entry {
                KeymapEntry::Key(key) => Some(key),
                _ => None,
            })
            .map(|x| extract_led(x))
            .collect();

        let menus: Vec<_> = keyboard_json
            .menus
            .into_iter()
            .flat_map(|x| x.content)
            .flat_map(|x| x.content)
            .collect();

        let speed = menus
            .iter()
            .find_map(|x| match x {
                MenuOption::Range {
                    content, options, ..
                } if content
                    .get(0)
                    .is_some_and(|x| x == "id_qmk_rgb_matrix_effect_speed") =>
                {
                    Some(*options)
                }
                _ => None,
            })
            .unwrap_or((0, 0));

        let brightness = menus
            .iter()
            .find_map(|x| match x {
                MenuOption::Range {
                    content, options, ..
                } if content
                    .get(0)
                    .is_some_and(|x| x == "id_qmk_rgb_matrix_brightness") =>
                {
                    Some(*options)
                }
                _ => None,
            })
            .unwrap_or((0, 0));

        // TODO: parse from `showIf` in JSON
        let flags = MODE_FLAG_HAS_BRIGHTNESS | MODE_FLAG_HAS_SPEED | MODE_FLAG_HAS_PER_LED_COLOR;

        let effects: Vec<_> = menus
            .into_iter()
            .flat_map(|x| match x {
                MenuOption::Dropdown {
                    content, options, ..
                } if content
                    .get(0)
                    .is_some_and(|x| x == "id_qmk_rgb_matrix_effect") =>
                {
                    Some(
                        options
                            .iter()
                            .map(|x| (x.0.clone(), x.1, flags))
                            .collect::<Vec<_>>(),
                    )
                }
                _ => None,
            })
            .flatten()
            .collect();

        Ok(Config {
            name: keyboard_json.name,
            vendor_id,
            product_id,
            leds,
            effects,
            speed,
            brightness,
            matrix,
        })
    }

    pub fn count_leds(&self) -> u32 {
        let index = self.leds.iter().max().unwrap_or(&None);
        if let Some(index) = index {
            return index.0 as u32 + 1;
        } else {
            return 0;
        }
    }
}

fn parse_hex(hex_str: &str) -> u16 {
    u16::from_str_radix(hex_str.trim_start_matches("0x"), 16).unwrap_or(0)
}

fn extract_led(key: &str) -> Option<(u8, (u8, u8))> {
    let mut flags = key.split('\n');

    let position: Vec<_> = flags.nth(0)?.split(',').collect();
    let row = position[0].trim().parse::<u8>().ok()?;
    let col = position[1].trim().parse::<u8>().ok()?;

    let led = flags
        .nth(0)
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
        })?;

    Some((led, (row, col)))
}

#[derive(Debug, Deserialize)]
pub struct KeyboardJson {
    name: String,
    #[serde(rename = "vendorId")]
    vendor_id: String,
    #[serde(rename = "productId")]
    product_id: String,
    matrix: MatrixDimensions,
    menus: Vec<Menu>,
    layouts: Layouts,
}

#[derive(Debug, Deserialize)]
pub struct MatrixDimensions {
    rows: u32,
    cols: u32,
}

#[derive(Debug, Deserialize)]
pub struct Menu {
    content: Vec<MenuContent>,
}

#[derive(Debug, Deserialize)]
pub struct MenuContent {
    content: Vec<MenuOption>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum MenuOption {
    #[serde(alias = "range")]
    Range {
        options: (u32, u32),
        content: Vec<Value>,
    },
    #[serde(alias = "dropdown")]
    Dropdown {
        content: Vec<Value>,
        options: Vec<(String, i32)>,
    },
    #[allow(dead_code)]
    #[serde(untagged)]
    Other(Value),
}

#[derive(Debug, Deserialize)]
pub struct Layouts {
    keymap: Vec<Vec<KeymapEntry>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum KeymapEntry {
    Key(String),
    #[allow(dead_code)]
    Group(Value),
}
