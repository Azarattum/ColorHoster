use std::collections::HashSet;

use anyhow::{Result, ensure};
use evalexpr::{
    ContextWithMutableVariables, HashMapContext, Node, Value as EvalValue, build_operator_tree,
};
use serde::Deserialize;
use serde_json::Value;

use crate::consts::{
    MODE_FLAG_HAS_BRIGHTNESS, MODE_FLAG_HAS_MODE_SPECIFIC_COLOR, MODE_FLAG_HAS_PER_LED_COLOR,
    MODE_FLAG_HAS_RANDOM_COLOR, MODE_FLAG_HAS_SPEED, MODE_FLAG_MANUAL_SAVE,
};

type Position = (u8, u8);
type Range = (u32, u32);
type Effect = (String, i32, u32);
type LedGrid = Vec<(u8, Position)>;
type ScanGrid = Vec<(u8, Position)>;

#[derive(Debug, Clone)]
pub struct Config {
    pub name: String,
    pub vendor: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub leds: LedGrid,
    pub matrix_pos: ScanGrid,
    pub effects: Vec<Effect>,
    pub speed: Range,
    pub brightness: Range,
    pub matrix: (u32, u32),
    pub matrix_vis: (u32, u32),
}

struct LedGeometry {
    leds: LedGrid,
    matrix_pos: ScanGrid,
    matrix_vis: (u32, u32),
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
        let geometry = Self::parse_leds(&layouts.keymap)?;

        Ok(Self {
            name,
            vendor: "Unknown".to_string(),
            vendor_id: parse_hex(&vendor_id),
            product_id: parse_hex(&product_id),
            matrix: (matrix.cols, matrix.rows),
            leds: geometry.leds,
            matrix_pos: geometry.matrix_pos,
            matrix_vis: geometry.matrix_vis,
            speed: Self::find_range(&menus, "id_qmk_rgb_matrix_effect_speed"),
            brightness: Self::find_range(&menus, "id_qmk_rgb_matrix_brightness"),
            effects: Self::parse_effects(menus),
        })
    }

    fn parse_leds(keymap: &Value) -> Result<LedGeometry> {
        struct RawKey {
            index: u8,
            matrix_pos: Position,
            col: i64,
            row: i64,
        }
        let keyboard: kle_serial::Keyboard = serde_json::from_value(keymap.clone())?;
        let annotations: Vec<_> = keymap
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(Value::as_str)
            .map(extract_led_annotation)
            .collect();

        ensure!(
            annotations.len() == keyboard.keys.len(),
            "KLE parser returned an unexpected number of keys"
        );

        let raw: Vec<RawKey> = keyboard
            .keys
            .iter()
            .zip(annotations)
            .filter_map(|(key, annotation)| {
                let (index, matrix_pos) = annotation?;
                let (col, row) = project_key(key);
                Some(RawKey {
                    index,
                    matrix_pos,
                    col,
                    row,
                })
            })
            .collect();

        if raw.is_empty() {
            return Ok(LedGeometry {
                leds: Vec::new(),
                matrix_pos: Vec::new(),
                matrix_vis: (0, 0),
            });
        }

        let min_row = raw.iter().map(|key| key.row).min().unwrap();
        let min_col = raw.iter().map(|key| key.col).min().unwrap();

        let mut order: Vec<usize> = (0..raw.len()).collect();
        order.sort_by_key(|&i| (raw[i].row, raw[i].col, raw[i].index));

        let mut visual = Vec::with_capacity(raw.len());
        let mut matrix_pos = Vec::with_capacity(raw.len());
        let mut occupied = HashSet::with_capacity(raw.len());
        let (mut max_row, mut max_col) = (0i64, 0i64);

        for i in order {
            let k = &raw[i];
            let row = k.row - min_row;
            let mut col = k.col - min_col;

            // Never lose a LED when keys land in the same integer cell
            while !occupied.insert((row, col)) {
                col += 1;
            }

            max_row = max_row.max(row);
            max_col = max_col.max(col);

            ensure!(
                row <= u8::MAX as i64 && col <= u8::MAX as i64,
                "KLE layout is too large for the OpenRGB matrix"
            );

            visual.push((k.index, (row as u8, col as u8)));
            matrix_pos.push((k.index, k.matrix_pos));
        }
        visual.sort();
        matrix_pos.sort();

        Ok(LedGeometry {
            leds: visual,
            matrix_pos,
            matrix_vis: (max_col as u32 + 1, max_row as u32 + 1),
        })
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

        let mut effects: Vec<Effect> = menus
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
            .enumerate()
            .map(|(index, option)| {
                let (name, id) = match option {
                    IndexedOption::Explicit((name, id)) => (name, id),
                    IndexedOption::Implicit(name) => (name, index as i32),
                };

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
            .collect();

        // Lift the direct mode at index 0 to ensure compatibility with some clients
        if let Some(index) = effects
            .iter()
            .position(|(_, _, flags)| flags & MODE_FLAG_HAS_PER_LED_COLOR != 0)
        {
            let effect = effects.remove(index);
            effects.insert(0, effect);
        }

        effects
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

    pub fn get_mode_index(&self, effect_id: i32) -> Option<usize> {
        self.effects.iter().position(|(_, id, _)| *id == effect_id)
    }

    pub fn get_effect_id(&self, mode_index: usize) -> Option<i32> {
        self.effects.get(mode_index).map(|(_, id, _)| *id)
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

fn project_key(key: &kle_serial::Key) -> (i64, i64) {
    let rotation = key.rotation.rem_euclid(360.0);
    let is_rotated = rotation > 1e-9 && (360.0 - rotation) > 1e-9;

    if is_rotated {
        let angle = key.rotation.to_radians();
        let (sin, cos) = angle.sin_cos();
        let relative_x = key.x + key.width / 2.0 - key.rx;
        let relative_y = key.y + key.height / 2.0 - key.ry;
        let x = key.rx + relative_x * cos - relative_y * sin - 0.5;
        let y = key.ry + relative_x * sin + relative_y * cos - 0.5;
        (x.round() as i64, y.round() as i64)
    } else {
        (key.x.round() as i64, (key.y + 1e-9).floor() as i64)
    }
}

fn extract_led_annotation(key: &str) -> Option<(u8, Position)> {
    let mut flags = key.split('\n');

    let (row_str, col_str) = flags.nth(0)?.split_once(',')?;
    let row = row_str.trim().parse::<u8>().ok()?;
    let col = col_str.trim().parse::<u8>().ok()?;
    let led = parse_marker(flags.next()?, 'l')?;

    // The encoder marker occupies KLE slot 9.
    if flags
        .nth(7)
        .and_then(|legend| parse_marker(legend, 'e'))
        .is_some()
    {
        return None;
    }
    Some((led, (row, col)))
}

fn parse_marker(value: &str, prefix: char) -> Option<u8> {
    value.strip_prefix(prefix)?.parse().ok()
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
        options: Vec<IndexedOption>,
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
#[serde(untagged)]
enum IndexedOption {
    Explicit((String, i32)),
    Implicit(String),
}

#[derive(Debug, Deserialize)]
struct Layouts {
    keymap: Value,
}
