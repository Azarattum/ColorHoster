use anyhow::Result;
use evalexpr::{
    ContextWithMutableVariables, HashMapContext, Node, Value as EvalValue, build_operator_tree,
};
use itertools::Itertools;
use serde::Deserialize;
use serde_json::Value;

use crate::consts::{
    MODE_FLAG_HAS_BRIGHTNESS, MODE_FLAG_HAS_MODE_SPECIFIC_COLOR, MODE_FLAG_HAS_PER_LED_COLOR,
    MODE_FLAG_HAS_RANDOM_COLOR, MODE_FLAG_HAS_SPEED, MODE_FLAG_MANUAL_SAVE,
};

type Position = (u8, u8);
type Range = (u32, u32);
type Effect = (String, i32, u32);

#[derive(Debug)]
pub struct Config {
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub leds: Vec<(u8, Position)>,
    pub effects: Vec<Effect>,
    pub speed: Range,
    pub brightness: Range,
    pub matrix: (u32, u32),
}

impl Config {
    pub fn from_str(json: &str) -> Result<Self> {
        let KeyboardJson {
            name,
            vendor_id,
            product_id,
            matrix,
            menus,
            layouts,
        } = serde_json::from_str(json)?;

        let menus = Self::flatten_menus(menus);

        Ok(Self {
            name,
            vendor_id: parse_hex(&vendor_id),
            product_id: parse_hex(&product_id),
            matrix: (matrix.cols, matrix.rows),
            leds: Self::parse_leds(&layouts.keymap),
            speed: Self::find_range(&menus, "id_qmk_rgb_matrix_effect_speed"),
            brightness: Self::find_range(&menus, "id_qmk_rgb_matrix_brightness"),
            effects: Self::parse_effects(menus),
        })
    }

    fn parse_leds(keymap: &[Vec<KeymapEntry>]) -> Vec<(u8, Position)> {
        keymap
            .iter()
            .flatten()
            .filter_map(|entry| {
                if let KeymapEntry::Key(key) = entry {
                    Some(key)
                } else {
                    None
                }
            })
            .filter_map(extract_led)
            .sorted()
            .collect()
    }

    fn flatten_menus(menus: Vec<Menu>) -> Vec<MenuOption> {
        menus
            .into_iter()
            .flat_map(|x| x.content)
            .flat_map(|x| x.content)
            .collect()
    }

    fn find_range(menus: &[MenuOption], target: &str) -> Range {
        menus
            .iter()
            .find_map(|m| match m {
                MenuOption::Range {
                    content, options, ..
                } if content.first().and_then(Value::as_str) == Some(target) => Some(*options),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn parse_effects(menus: Vec<MenuOption>) -> Vec<Effect> {
        let controls = Self::collect_controls(&menus);

        menus
            .into_iter()
            .find_map(|m| match m {
                MenuOption::Dropdown { content, options }
                    if content.first().and_then(Value::as_str)
                        == Some("id_qmk_rgb_matrix_effect") =>
                {
                    Some(options)
                }
                _ => None,
            })
            .into_iter()
            .flatten()
            .map(|(name, id)| {
                let mut flags = controls
                    .iter()
                    .filter(|x| x.is_active(id))
                    .fold(0, |flags, x| flags | x.flag);

                let has_no_color =
                    flags & (MODE_FLAG_HAS_PER_LED_COLOR | MODE_FLAG_HAS_MODE_SPECIFIC_COLOR) == 0;

                if has_no_color && id != 0 {
                    flags = flags | MODE_FLAG_HAS_RANDOM_COLOR;
                }

                if flags & (MODE_FLAG_HAS_SPEED | MODE_FLAG_HAS_MODE_SPECIFIC_COLOR) != 0 {
                    flags = flags | MODE_FLAG_MANUAL_SAVE;
                }

                return (name, id, flags);
            })
            .collect()
    }

    fn collect_controls(menus: &[MenuOption]) -> Vec<Control> {
        menus
            .iter()
            .filter_map(|m| match m {
                MenuOption::Range {
                    content, show_if, ..
                } => content
                    .first()
                    .and_then(Value::as_str)
                    .and_then(|id| match id {
                        "id_qmk_rgb_matrix_brightness" => {
                            Some(Control::new(show_if, MODE_FLAG_HAS_BRIGHTNESS))
                        }
                        "id_qmk_rgb_matrix_effect_speed" => {
                            Some(Control::new(show_if, MODE_FLAG_HAS_SPEED))
                        }
                        _ => None,
                    }),
                MenuOption::Color { content, show_if } if is_color_control(content) => {
                    Some(Control::new(show_if, MODE_FLAG_HAS_MODE_SPECIFIC_COLOR))
                }
                MenuOption::ColorPalette { content, show_if } if is_color_control(content) => {
                    Some(Control::new(show_if, MODE_FLAG_HAS_PER_LED_COLOR))
                }
                _ => None,
            })
            .collect()
    }

    pub fn count_leds(&self) -> u32 {
        let index = self.leds.iter().max();
        if let Some(index) = index {
            return index.0 as u32 + 1;
        } else {
            return 0;
        }
    }
}

#[derive(Debug)]
struct Control {
    condition: Option<Node>,
    flag: u32,
}

impl Control {
    fn new(expression: &Option<String>, flag: u32) -> Self {
        Self {
            flag,
            condition: expression
                .as_ref()
                .and_then(|x| build_operator_tree(x).ok()),
        }
    }

    fn is_active(&self, effect_id: i32) -> bool {
        self.condition.as_ref().map_or(true, |node| {
            let mut context = HashMapContext::new();
            let identifier = "{id_qmk_rgb_matrix_effect}";
            context
                .set_value(identifier.into(), EvalValue::Int(effect_id.into()))
                .and_then(|_| node.eval_boolean_with_context(&context))
                .unwrap_or(false)
        })
    }
}

fn is_color_control(content: &[Value]) -> bool {
    content.first().and_then(Value::as_str) == Some("id_qmk_rgb_matrix_color")
}

fn parse_hex(s: &str) -> u16 {
    u16::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or(0)
}

fn extract_led(key: &String) -> Option<(u8, Position)> {
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
struct KeyboardJson {
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
struct MatrixDimensions {
    rows: u32,
    cols: u32,
}

#[derive(Debug, Deserialize)]
struct Menu {
    content: Vec<MenuContent>,
}

#[derive(Debug, Deserialize)]
struct MenuContent {
    content: Vec<MenuOption>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum MenuOption {
    #[serde(rename = "range")]
    Range {
        content: Vec<Value>,
        options: Range,
        #[serde(rename = "showIf")]
        show_if: Option<String>,
    },
    #[serde(rename = "dropdown")]
    Dropdown {
        content: Vec<Value>,
        options: Vec<(String, i32)>,
    },
    #[serde(rename = "color")]
    Color {
        content: Vec<Value>,
        #[serde(rename = "showIf")]
        show_if: Option<String>,
    },
    #[serde(rename = "color-palette")]
    ColorPalette {
        content: Vec<Value>,
        #[serde(rename = "showIf")]
        show_if: Option<String>,
    },
    #[allow(dead_code)]
    #[serde(untagged)]
    Other(Value),
}

#[derive(Debug, Deserialize)]
struct Layouts {
    keymap: Vec<Vec<KeymapEntry>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum KeymapEntry {
    Key(String),
    #[allow(dead_code)]
    Other(Value),
}
