use anyhow::Result;
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
        let geometry = Self::parse_leds(&layouts.keymap);

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

    fn parse_leds(keymap: &[Vec<KeymapEntry>]) -> LedGeometry {
        struct RawKey {
            index: u8,
            matrix_pos: Position,
            col_milli: i64,
            row: i64,
        }
        let mut raw: Vec<RawKey> = Vec::new();

        for (row_idx, entries) in keymap.iter().enumerate() {
            let (mut x, mut y, mut w) = (0.0_f64, 0.0_f64, 1.0_f64);

            for entry in entries {
                match entry {
                    KeymapEntry::Other(value) => {
                        x += value.get("x").and_then(Value::as_f64).unwrap_or(0.0);
                        y += value.get("y").and_then(Value::as_f64).unwrap_or(0.0);
                        w = value.get("w").and_then(Value::as_f64).unwrap_or(w);
                    }
                    KeymapEntry::Key(key) => {
                        if let Some((index, matrix_pos)) = extract_led(key) {
                            raw.push(RawKey {
                                index,
                                matrix_pos,
                                col_milli: (x * 1000.0).round() as i64,
                                row: (row_idx as f64 + y).round() as i64,
                            });
                        }
                        x += w;
                        w = 1.0;
                    }
                }
            }
        }
        let mut rows: Vec<i64> = raw.iter().map(|k| k.row).collect();
        rows.sort_unstable();
        rows.dedup();

        let mut order: Vec<usize> = (0..raw.len()).collect();
        order.sort_by_key(|&i| (rows.binary_search(&raw[i].row).unwrap(), raw[i].col_milli));

        let mut visual = Vec::with_capacity(raw.len());
        let mut matrix_pos = Vec::with_capacity(raw.len());
        let mut last_col_in_row: std::collections::HashMap<u8, i64> = std::collections::HashMap::new();
        let mut max_col = 0i64;

        for i in order {
            let k = &raw[i];
            let row = rows.binary_search(&k.row).unwrap() as u8;
            let last = last_col_in_row.entry(row).or_insert(-1);
            let col = ((k.col_milli + 500).div_euclid(1000)).max(*last + 1);
            *last = col;

            max_col = max_col.max(col);
            visual.push((k.index, (row, col as u8)));
            matrix_pos.push((k.index, k.matrix_pos));
        }
        visual.sort();
        matrix_pos.sort();

        LedGeometry {
            leds: visual,
            matrix_pos,
            matrix_vis: (max_col as u32 + 1, rows.len() as u32),
        }
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
    keymap: Vec<Vec<KeymapEntry>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum KeymapEntry {
    Key(String),
    #[allow(dead_code)]
    Other(Value),
}
