# AGENTS.md

Rules for anyone, human or coding agent, who changes RustWeather. Keep changes small and in line with these rules.

## Stack

- Rust and Slint only (a tray icon plus a radar and forecast window). No web views, no Python or shell helpers at
  runtime, no extra runtimes. Target: Arch Linux / Manjaro with KDE Plasma.
- `crates/rustweather-core` has no UI; `crates/rustweather` is the app.

## Keys and privacy

- Never put an API key into the repository, a test, a script or a log. Keys live only in the user's
  `~/.config/rustweather/secrets.toml` (mode 0600); URLs that may carry a key are never logged.
- Fetch only what is on screen. The tray asks for the temperature only; while the session is locked, the screen saver
  runs or a fullscreen window is active, nothing is requested.

## Licenses and third-party material

- Own code is MIT. Never copy code, icons, fonts or data from other projects; use a dependency, or write it yourself,
  and check every new dependency's license first. Third-party data and icons (Natural Earth, GeoNames, Tabler Icons)
  are listed with their licenses in `assets/README.md`; keep that list complete.
- Interface icons come from Tabler Icons (MIT) only.

## Code

- Prefer maintained crates over handwritten code. US English everywhere: code, comments, logs, UI strings, docs,
  commits.
- No `unwrap`, `expect` or `panic!` outside tests; `cargo clippy -- -D warnings` is clean; files stay under ~600 lines.
- Data lives in the XDG folders (`~/.config/rustweather`, `~/.local/state/rustweather`, ...). Settings changes stay
  backward compatible; a migration backs up first. UI state survives a restart.
- Flat design: no shadows, no gradients; buttons are icons with tooltips.

## Git and checks

- `master` is always green. Work on short branches (`feat/`, `fix/`, `refactor/`, `docs/`, `chore/`, ...) with
  Conventional Commits and merge them with `--no-ff`. Never force-push `master`.
- `scripts/check-all.sh` passes before a merge. Every user-visible change gets an entry under `## [Unreleased]` in
  `CHANGELOG.md` in the same commit. Releases are annotated tags `vX.Y.Z` with a GitHub release (`scripts/release.sh`).
