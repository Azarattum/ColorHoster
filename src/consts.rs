use enumn::N;

#[derive(PartialEq, Debug, N)]
#[repr(u32)]
pub enum Request {
    GetControllerCount = 0,
    GetControllerData = 1,
    GetProtocolVersion = 40,
    SetClientName = 50,
    DeviceListUpdated = 100,
    GetProfileList = 150,
    SaveProfile = 151,
    LoadProfile = 152,
    DeleteProfile = 153,
    ResizeZone = 1000,
    UpdateLeds = 1050,
    UpdateZoneLeds = 1051,
    UpdateSingleLed = 1052,
    SetCustomMode = 1100,
    UpdateMode = 1101,
    SaveMode = 1102,
}

pub const QMK_USAGE_PAGE: u16 = 0xFF60;
pub const QMK_USAGE_ID: u16 = 0x61;

pub const QMK_CUSTOM_SET_COMMAND: u8 = 0x07;
pub const QMK_CUSTOM_GET_COMMAND: u8 = 0x08;
pub const QMK_KEYMAP_GET_COMMAND: u8 = 0x12;

pub const QMK_CUSTOM_CHANNEL: u8 = 0x0;
pub const QMK_COMMAND_MATRIX_CHROMA: u8 = 0x1;
pub const QMK_COMMAND_MATRIX_BRIGHTNESS: u8 = 0x2;

pub const QMK_RGB_MATRIX_CHANNEL: u8 = 0x3;
pub const QMK_COMMAND_BRIGHTNESS: u8 = 0x1;
pub const QMK_COMMAND_EFFECT: u8 = 0x2;
pub const QMK_COMMAND_SPEED: u8 = 0x3;
pub const QMK_COMMAND_COLOR: u8 = 0x4;

pub const DEVICE_TYPE_KEYBOARD: i32 = 5;

pub const MODE_FLAG_HAS_SPEED: u32 = 1 << 0;
pub const MODE_FLAG_HAS_BRIGHTNESS: u32 = 1 << 4;
pub const MODE_FLAG_HAS_PER_LED_COLOR: u32 = 1 << 5;
pub const MODE_FLAG_HAS_MODE_SPECIFIC_COLOR: u32 = 1 << 6;
pub const MODE_FLAG_HAS_RANDOM_COLOR: u32 = 1 << 7;

pub const ZONE_TYPE_MATRIX: i32 = 2;

pub fn openrgb_keycode(keycode: u16) -> &'static str {
    match keycode {
        1 => "Right Fn",
        4 => "A",
        5 => "B",
        6 => "C",
        7 => "D",
        8 => "E",
        9 => "F",
        10 => "G",
        11 => "H",
        12 => "I",
        13 => "J",
        14 => "K",
        15 => "L",
        16 => "M",
        17 => "N",
        18 => "O",
        19 => "P",
        20 => "Q",
        21 => "R",
        22 => "S",
        23 => "T",
        24 => "U",
        25 => "V",
        26 => "W",
        27 => "X",
        28 => "Y",
        29 => "Z",
        30 => "1",
        31 => "2",
        32 => "3",
        33 => "4",
        34 => "5",
        35 => "6",
        36 => "7",
        37 => "8",
        38 => "9",
        39 => "0",
        40 => "Enter",
        41 => "Escape",
        42 => "Backspace",
        43 => "Tab",
        44 => "Space",
        45 => "-",
        46 => "=",
        47 => "[",
        48 => "]",
        49 => "\\",
        50 => "#",
        51 => ";",
        52 => "'",
        53 => "`",
        54 => ",",
        55 => ".",
        56 => "/",
        57 => "Caps Lock",
        58 => "F1",
        59 => "F2",
        60 => "F3",
        61 => "F4",
        62 => "F5",
        63 => "F6",
        64 => "F7",
        65 => "F8",
        66 => "F9",
        67 => "F10",
        68 => "F11",
        69 => "F12",
        70 => "Print Screen",
        71 => "Scroll Lock",
        72 => "Pause/Break",
        73 => "Insert",
        74 => "Home",
        75 => "Page Up",
        76 => "Delete",
        77 => "End",
        78 => "Page Down",
        79 => "Right Arrow",
        80 => "Left Arrow",
        81 => "Down Arrow",
        82 => "Up Arrow",
        83 => "Num Lock",
        84 => "Number Pad /",
        85 => "Number Pad *",
        86 => "Number Pad -",
        87 => "Number Pad +",
        88 => "Number Pad Enter",
        89 => "Number Pad 1",
        90 => "Number Pad 2",
        91 => "Number Pad 3",
        92 => "Number Pad 4",
        93 => "Number Pad 5",
        94 => "Number Pad 6",
        95 => "Number Pad 7",
        96 => "Number Pad 8",
        97 => "Number Pad 9",
        98 => "Number Pad 0",
        99 => "Number Pad .",
        100 => "\\ (ISO)",
        101 => "Menu",
        104 => "F13",
        105 => "F14",
        106 => "F15",
        107 => "F16",
        168 => "Media Mute",
        169 => "Media Volume +",
        170 => "Media Volume -",
        171 => "Media Next",
        172 => "Media Previous",
        173 => "Media Stop",
        174 => "Media Play/Pause",
        175 => "Media Select",
        176 => "Media Eject",
        189 => "Brightness Up",
        190 => "Brightness Down",
        196 => "Task Manager",
        202 => "RGB Brightness Up",
        203 => "RGB Brightness Down",
        216 => "Left Shift",
        217 => "Right Shift",
        224 => "Left Control",
        225 => "Left Shift",
        226 => "Left Alt",
        227 => "Left Windows",
        228 => "Right Control",
        229 => "Right Shift",
        230 => "Right Alt",
        231 => "Right Windows",
        x => {
            if x & 0x1f != 0 {
                "Right Fn"
            } else {
                "Unknown"
            }
        }
    }
}
