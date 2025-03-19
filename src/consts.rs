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
