# Changelog

All notable changes to RustWeather are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- A click on a warning area opens the full warning in a floating panel on the right, in the DWD's German wording.
- Temperature layer on the radar: labels across the map, more of them the closer you zoom in.
- The radar plays when it opens (Settings > Radar), and the weather card always plays its small radar. Both use the same data.

### Fixed

- Outside Germany the rain vanished after the first picture when playing: frames still on their way are now skipped, and tiles for a place
  you already left no longer use up RainViewer's limit.

## [0.1.2] - 2026-10-30

### Changed

- Clouds, warnings and lightning are cached like the map: what was seen once appears at once, also after a restart, until the data is new.
- The rain curve in the weather card only appears when there is rain, and it is labeled.

### Added

- The weather card shows the time of the last update.

## [0.1.1] - 2026-10-30

### Changed

- The Now page is redesigned: a colored summary, symbols, small gauges for wind, UV, air and the sun, and a clearer 24-hour curve.
- The Forecast page has a temperature curve, a legend, named columns, weather symbols and tooltips on every hour and day.
- The app icon now matches RustView, RustCut and RustKnown.

### Added

- Every window comes back with its size and place, maximized or not.

### Fixed

- The weather card no longer opens in the top left corner of the screen now and then.
- Logs say what really failed: a slow server is no longer reported as "no connection", a stalled name lookup is retried, and a failing
  map layer is not asked again in a loop.
- The first press on the radar map was lost, so a first drag did nothing.
- Tooltips never appeared; now every button, card and chart explains itself.

- Clouds, warnings and lightning now stay on their place on the map and follow every pan and zoom at once; warnings no longer wait
  for the slow lightning data.
- The notice about a missing CARTO key disappears as soon as a key is stored.
- The Settings button of that notice, and anything else that opens a window, now brings an already open window to the front.

## [0.1.0] - 2026-10-30

First release, replacing the `weather_tray.py` script.

### Added

- Tray icon drawn from the app's own SVG pieces, with white digits on a tile in the temperature's color, and signs for rain,
  warnings, stale data and the trend.
- Tooltip in plain sentences; a panel above the tray with the weather, the rain curve and a small radar.
- Now, Forecast, Radar and Warnings pages, light and dark, following the system.
- Radar from the DWD and RainViewer with free zoom, smooth motion, forecast, saved views, warning and lightning layers, and a
  small always-on-top window.
- CARTO, OpenStreetMap and an offline map of the app's own; API keys in Settings, stored outside the program.
- A weather card while the pointer rests on the tray icon, as an option on X11.
- Settings > Alerts can send a test notification.
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

[Unreleased]: https://github.com/vommie/rustweather/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/vommie/rustweather/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/vommie/rustweather/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/vommie/rustweather/releases/tag/v0.1.0
