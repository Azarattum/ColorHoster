# Color Hoster 🌈 ⌨️

[OpenRGB](https://openrgb.org/) [compatible](https://gitlab.com/OpenRGBDevelopers/OpenRGB-Wiki/-/blob/stable/Developer-Documentation/OpenRGB-SDK-Documentation.md) high-performance SDK server for [VIA per-key RGB](https://github.com/the-via/app/blob/80dd7453a2f0a53233cd2c5bcc526847feb17e0e/src/utils/keyboard-api.ts#L372-L384).

**✨ Features:**

- 🎮 **OpenRGB Protocol v3 Support** - Full compatibility with OpenRGB clients
- 🔋 **Optimized Updates** - Only sends changed LED data in minimal chunks
- 🎨 **Flexible Brightness Control** - Choose between hue/saturation-only or full RGB+brightness
- ⌨️ **Multi-Keyboard Support** - Manage multiple connected devices simultaneously
- 🦀 **Rust-Powered** - Async I/O and thread-safe concurrency
- 🌐 **Cross-Platform** - Windows/macOS/Linux support (x86 & ARM)
- 💾 **Profile Management** - Save/load lighting configurations

## CLI Options

```bash
Usage: ColorHoster [OPTIONS]

Options:
  -d, --directory <DIRECTORY>  Set a directory to look for VIA `.json` definitions for keyboards [default: ./]
  -j, --json <JSON>            Add a direct path to a VIA `.json` file (can be multiple)
  -b, --brightness             Allow direct mode to change brightness values
      --profiles <PROFILES>    Set a directory for storing and loading profiles [default: ./profiles]
  -p, --port <PORT>            Set the port to listen on [default: 6742]
  -h, --help                   Print help
  -V, --version                Print version

Example: ./ColorHoster -b -j ./p1_he_ansi_v1.0.json
```

## Getting Started

### Prebuilt Binaries
Download [the latest release](https://github.com/Azarattum/ColorHoster/releases) for your OS.

### Configuration
1. Get your keyboard's VIA JSON file with led positions defined in the `keymap` section (example: [p1_he_ansi_v1.0.json](https://github.com/Azarattum/QMK/blob/hall_effect_custom/keyboards/lemokey/p1_he/via_json/p1_he_ansi_v1.0.json))
2. Flash QMK firmware with VIA RGB support ([example implementation](https://github.com/Azarattum/QMK/commit/b80ff1fdd85fe8d2eb7c604f02568b8adf5f949f))

### Running
```bash
./ColorHoster --brightness --json ./path/to/your_keyboard.json
```

## Client Integration

ColorHoster should be compatible with any OpenRGB v3 protocol client, enabling RGB control through various applications. Some example clients include:

- [OpenRGB](https://openrgb.org/) (can be use as a client as well)
- [Project Aurora](https://www.project-aurora.com/) (may require renaming the binary to `OpenRGB.exe` to be detected)
- [Artemis RGB](https://artemis-rgb.com/)

### To Connect
1. Launch ColorHoster with your keyboard configuration
2. In your RGB control software:
   - Add a new OpenRGB SDK device
   - Set address to `127.0.0.1`
   - Set port to `6742` (default)
3. The client should automatically detect:
   - Keyboard LED layout
   - Available lighting modes
   - Real-time control capabilities

## Technical Details

VIA's RGB protocol doesn't seem to be documented anywhere, so it was reverse-engineered from  [the keyboard API in the VIA app](https://github.com/the-via/app/blob/80dd7453a2f0a53233cd2c5bcc526847feb17e0e/src/utils/keyboard-api.ts#L372-L384). The protocol in ColorHoster is also extended to support per-key brightness adjustments (originally it allowed to modify only hue and saturation).