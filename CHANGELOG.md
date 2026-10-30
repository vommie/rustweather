# Changelog

## [Unreleased]

### Added

- Tray icon drawn from the app's own SVG pieces, with white digits on a tile in the temperature's color, and signs for rain,
  warnings, stale data and the trend.
- Tooltip in plain sentences; a panel above the tray with the weather, the rain curve and a small radar.
- Now, Forecast, Radar and Warnings pages, light and dark, following the system.
- Radar from the DWD and RainViewer with free zoom, smooth motion, forecast, saved views, warning and lightning layers, and a
  small always-on-top window.
- CARTO, OpenStreetMap and an offline map of the app's own; API keys in Settings, stored outside the program.
- Desktop notifications for rain, warnings, thunderstorms, frost and heat, with quiet hours.
- Loading indicators, a disk cache with limits, automatic retries, and a Diagnostics page.
- Import of the old `weather_tray.json`.
- Clouds layer on the radar from Meteosat pictures, with the picture's time on its button.
- Wind layer on the radar: particles flow with the current wind, colored from calm to stormy.
- App icon: a radar screen in the Breeze manner, with a desktop file.
- Settings > Desktop shows whether RustWeather is part of the desktop and installs, repairs or uninstalls the menu entry, the
  icons and the autostart in your own folders. `--desktop install`, `uninstall` and `status` do the same from a terminal.
- Arch package with `build-package.sh`.
- The application menu entry offers Radar and Settings as actions; a second start opens what it asks for.

### Fixed

- The radar no longer fetches or animates behind a locked screen; it catches up when you are back.
