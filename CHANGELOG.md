## 0.7.1

- Added support for the vendor flag on OpenRGB clients.

  The vendor name is read from the manufacturer string from what the OS got off the keyboard's USB firmware when it was plugged in, using the manufacturer field added to async-hid's DeviceInfo in version 0.5.1. The manufacturer name has been compiled into the QMK firmware and is then basically reported to OpenRGB clients.

## 0.7.0

- (potentially breaking) Direct mode is now always reported to clients at index 0 (this does not affect your JSON configs)
- Backwards compatibility for clients using lower OpenRGB protocol versions
- TCP response buffering to avoid data races on some client readers

## 0.6.3

- Improved simultaneous multi-keyboard support
- Updated dependencies

## 0.6.1

- Added support for implicit option indexing in VIA json configs. Thanks, @gzowski

## 0.6.0

- Improved latency for high-frequency updates

## 0.5.2

- Fixed led ordering on some keyboards (see #4). Thanks, @DEREFERENC3D

## 0.5.1

- Fixed HID issues on Windows

## 0.5.0

- `colorhoster.toml` config file can now override the program defaults
- `--service create` now saves any extra options to `colorhoster.toml`

## 0.4.0

- Support for hot-plugging keyboards without restarting the app
- Improved communication reliability

## 0.3.2

- Fixed loading keymap layout on some keyboards (tested with Keychron Q6 HE)

## 0.3.1

- Fixed buffer reports on Linux (potentially)

## 0.3.0

- (breaking) Made default directory for `.json` files and profiles relative to the executable directory
- Improved service mode stability on windows
- More stable interruptions and better logs

## 0.2.0

- Added experimental service mode
- Fixed race condition on program init

## 0.1.7

- Fixed ANSI backslash keycode

## 0.1.6

- Improved windows support
- Refactors and optimizations

## 0.1.5

- Added support for all OpenRGB SDK API requests
- Now mode settings can be saved to the keyboard's memory

## 0.1.4

- Most common OpenRGB SDK APIs are implemented
- VIA JSON config parser
- Configurable CLI parameters
- Cross-build CI
