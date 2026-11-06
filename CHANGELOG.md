# Changelog

All notable changes to RustWeather are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.10] - 2026-11-06

### Changed

- The place a new installation starts with is Berlin (city centre) with the Berlin-Tempelhof station. Places you chose
  yourself stay as they are.
- The repository is public now: contributor guide (AGENTS.md), security policy (SECURITY.md).

## [0.1.9] - 2026-10-31

### Fixed

- Clouds are complete now, at every zoom, by day and by night. Low clouds were thrown away because in the satellite picture they look like the
  ground. The layer now uses EUMETSAT's cloud mask for where clouds are and the infrared picture for their structure, every 15 minutes, and the
  world ring of all weather satellites beyond the view of Meteosat.

## [0.1.8] - 2026-10-31

### Added

- Horizontal lines on the Forecast page at round values, with a stronger one at 0 °C. A filter hides them.

### Changed

- Day tiles use their space: no empty room at the bottom, and the course of the day runs over the whole width.

### Fixed

- A day tile no longer shows a rain symbol next to "dry".

## [0.1.7] - 2026-10-31

### Added

- The Forecast page is new: one band of time from a week ago to 16 days ahead, measured on the left and forecast on the right. Drag it,
  zoom it with the wheel, page through it, jump to a day or back to now. Lanes for temperature, rain, wind and more, each with its numbers,
  and every value can be shown or hidden. The pointer shows all values of an hour.
- Days as tiles with the course of the day, as plain tiles, as columns or as bars, whichever you like, sliding sideways when the window is
  narrow. A click on a day takes the band there.
- A sentence and a few facts on top: next rain, warmest and coldest day, next frost. Warnings, dry spells, nights, the usual range of the
  season, how sure the forecast is and what had been announced for past hours all lie on the band.
- A setting for how often the forecast is fetched, two hours by default, and the page shows the time of the data.

### Changed

- Nothing the app keeps stays longer than a week: map and radar pictures, measured data and logs.

## [0.1.6] - 2026-10-30

### Changed

- A left click on the tray icon always opens the radar in the main window. The weather card comes on hover and from the tray menu.
- Wind comes from the DWD's model too, one request an hour for the whole world, so moving the map needs no new data. Open-Meteo only steps in
  when the DWD fails and no recent grid is left. The particles stay the same.

## [0.1.5] - 2026-10-30

### Added

- The status line names a service that does not answer as it should, like a reached limit, and leads to the Network page.

### Changed

- A new app icon, sun and cloud, in the frame of RustView and RustCut.
- Settings are now a page of the main window, at the bottom of the rail, instead of a window of their own.
- The Network page counts every place of a request toward today, as Open-Meteo does.

### Fixed

- City temperatures and the heatmap come from one DWD model grid, fetched once an hour, for the whole world. Asking Open-Meteo for each
  point used up its daily limit, which froze the map and also took the forecast away for the day. Labels and colors now follow every pan
  and zoom at once.
- A failed temperature load shows a banner instead of staying silent.

## [0.1.4] - 2026-10-30

### Added

- Temperature labels now belong to cities, with their name: big cities first, smaller ones appear as you zoom in.
- The district whose warning is open is outlined on the map, and districts with a warning light up under the pointer.

### Changed

- Every setting now explains itself in a short tooltip, shown over the whole row, instead of text under its name.
- The map that ships with the app is now called Standard map.
- The note about a clicked place stays on that place while the map moves, and closes with a right-click.
- The layer buttons on the radar show only their icon, with the explanation in the tooltip.

### Fixed

- The radar and the wind go on within a second when a minimized window comes back.
- The map previews no longer wait behind slow weather layers: each appears as soon as it is complete, and "Loading…" stands in until then.
- Maps no longer queue behind the DWD's slow map server when clouds, warnings or lightning are on.

## [0.1.3] - 2026-10-30

### Added

- The radar has a map picker right on the map, with a small preview of every map: satellite, satellite without clouds, topographic, streets and terrain, shaded relief,
  TopPlus, light and dark gray, OpenStreetMap, the own map and CARTO. It replaces the setting. Every map but CARTO works without a key; CARTO stays grayed out until there is one, and its tooltip says why.
- A Network page in the settings shows every request live, with statistics per server: running now, last minute, today against the
  service's limit, errors, data and average time. API keys never show.
- In the background the app only fetches the temperature for the tray. Forecast, warnings and the rain outlook wait until a window, the
  weather card or the radar is open, and are fetched then. Behind a locked screen, a screen saver or a fullscreen program nothing is
  fetched at all. Notifications about rain and warnings therefore only come while the app is in use. A new setting on the Alerts page,
  off by default, also checks the official warnings in the background, with one more request per update.
- A temperature heatmap colors the whole map, with its scale in the legend. It shares its values with the temperature labels.
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

[Unreleased]: https://github.com/vommie/rustweather/compare/v0.1.10...HEAD
[0.1.10]: https://github.com/vommie/rustweather/compare/v0.1.9...v0.1.10
[0.1.9]: https://github.com/vommie/rustweather/compare/v0.1.8...v0.1.9
[0.1.8]: https://github.com/vommie/rustweather/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/vommie/rustweather/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/vommie/rustweather/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/vommie/rustweather/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/vommie/rustweather/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/vommie/rustweather/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/vommie/rustweather/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/vommie/rustweather/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/vommie/rustweather/releases/tag/v0.1.0
