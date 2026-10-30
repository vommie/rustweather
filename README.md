# RustWeather

The temperature in your tray and a weather radar that leaves nothing to wish for. A Rust + Slint rewrite of a PyQt tray
script, for Linux with KDE Plasma.

- **Tray:** the temperature on a tile in its color, with a blue dot when rain is near, a triangle for warnings, a small trend
  arrow, and a pale tile when the data is old. The tooltip says what matters in plain sentences.
- **Radar:** free zoom and pan, the current picture at once, smooth motion, a timeline with past and a two-hour forecast,
  rain arrival at your place, a rain curve, a click on the map for exact values, saved views, warnings and lightning as
  layers, a small always-on-top window.
- **Sources:** DWD through Bright Sky, Open-Meteo, RainViewer outside Germany, a wetterdienst.de station page as a fallback.
- **Maps:** CARTO (needs your key), OpenStreetMap, or the app's own map, which needs neither key nor network.

## Build

```bash
cargo build --release
```

## Arch package

```bash
./build-package.sh
```

This writes `dist/rustweather-<version>-1-x86_64.pkg.tar.zst`; install it with `sudo pacman -U`.

## Desktop integration

Settings > Desktop shows the state and installs the application menu entry and the icons for your user, in `~/.local/share`. It
notices a moved program and offers a repair. Uninstall removes them and the autostart, and leaves your settings and keys alone. Without
the window:

```bash
rustweather --desktop install
```

## API keys

CARTO maps need a key: Settings > Sources. It is stored in `~/.config/rustweather/secrets.toml`, readable only by you.
Keys are never part of the program, of this repository or of the logs.

## Files

Settings in `~/.config/rustweather`, cache in `~/.cache/rustweather`, logs in `~/.local/state/rustweather/logs`.

## License

MIT. Map borders: Natural Earth (public domain). Icons and tray digits: Tabler Icons (MIT). See `assets/README.md`.
