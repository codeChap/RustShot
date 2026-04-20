# RustShot

Fast Rust screenshot tool for Linux + X11 (i3-friendly). A from-scratch port of [Flameshot](https://github.com/flameshot-org/flameshot)'s core workflow: drag a region, annotate, save and/or copy to clipboard — driven by a long-running daemon so each PrintScreen is a fast IPC call instead of a cold binary start.

## Status

- **Linux + X11 only.** Developed and tested on i3wm. No Wayland, no Windows, no macOS.
- ~11 MB stripped release binary. Runtime deps: X11, and `xclip` (for the clipboard — `apt install xclip`).

## Features

- Drag-rect region selection with dimmed exterior
- Annotation tools: Pencil, Arrow, Rect, Ellipse, Blur, Auto-counter (numbered marker)
- Per-tool color (configurable palette) + stroke width
- Undo / redo
- Save to disk, copy to clipboard, or both
- DBus-driven daemon → instant overlay on hotkey
- Optional X11 cursor compositing via XFixes
- TOML config: defaults, palette, save dir, filename pattern

## Install

Requires a Rust toolchain (`rustup`).

```bash
git clone https://github.com/codeChap/RustShot.git
cd RustShot
cargo install --path .
```

This puts `rustshot` in `~/.cargo/bin/`.

## Usage

### Run the daemon

```bash
rustshot
```

Registers on the session DBus as `org.rustshot.RustShot` and waits for capture requests. Exits cleanly (status 0) if another instance already owns the bus name.

### Trigger captures

```bash
rustshot gui                 # interactive region select; auto-save to default path
rustshot gui -c              # auto-save + copy to clipboard
rustshot gui -p shot.png     # save to a specific path
rustshot gui -p shot.png -c  # save + clipboard
rustshot gui -c --no-save    # clipboard only, no disk write

rustshot full                # all monitors stitched, no UI
rustshot full -c             # all monitors → clipboard

rustshot screen              # the cursor's monitor, no UI
rustshot screen -n 1         # specific monitor by index
```

`-d MS` adds a delay before capture. `--no-save` skips the disk write (use with `-c` for clipboard-only flows).

### Overlay shortcuts

| Key            | Action                                 |
| -------------- | -------------------------------------- |
| Drag           | Select region (then enter Annotate)    |
| `1`–`6`        | Pencil, Arrow, Rect, Ellipse, Blur, Counter |
| `Ctrl+Z`/`Y`   | Undo / Redo                            |
| `Enter`        | Save                                   |
| `Ctrl+C`       | Copy to clipboard                      |
| `Esc`          | Cancel                                 |

The Auto-Counter tool single-clicks numbered bubbles that auto-increment per overlay session.

## i3 setup

Add to `~/.config/i3/config`:

```text
# RustShot overlay must float and lose i3 borders to fill the screen cleanly
for_window [class="rustshot"] floating enable, border none

# Autostart the daemon (or use systemd, see below)
exec --no-startup-id rustshot

# PrintScreen → drag region → save (auto-path) + clipboard
bindsym Print            exec --no-startup-id rustshot gui -c
bindsym $mod+Print       exec --no-startup-id rustshot screen -c
bindsym $mod+Shift+Print exec --no-startup-id rustshot full -c
```

Reload with `i3-msg reload`.

## systemd autostart (alternative to i3 `exec`)

```bash
mkdir -p ~/.config/systemd/user
cp data/systemd/rustshot.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now rustshot.service
```

The unit's `ExecStart` is `%h/.cargo/bin/rustshot`, matching `cargo install`.

## Configuration

`~/.config/rustshot/config.toml` — sample at [`data/sample-config.toml`](data/sample-config.toml). All sections + keys are optional; missing values fall back to defaults.

```toml
[defaults]
color = "#ff3232"                       # default annotation color (#rrggbb or #rrggbbaa)
width = 4.0                             # default stroke width
counter_radius = 16.0                   # auto-counter bubble radius
blur_sigma = 12.0                       # blur strength
initial_tool = "rect"                   # pencil | arrow | rect | ellipse | blur | counter
save_dir = "~/Pictures/screenshots"     # used when no -p flag is given
filename_pattern = "%Y%m%d-%H%M%S.png"  # strftime format (chrono::format)

[capture]
include_cursor = false                  # composite the X11 cursor (XFixes) into screenshots

[palette]
colors = [
  "#ff3232", "#ffc800", "#50c850", "#32b4dc",
  "#4664dc", "#c850c8", "#ffffff", "#000000",
]
```

The daemon reads the config at startup. To apply changes:

```bash
pkill -x rustshot && rustshot &
```

## DBus interface

Service `org.rustshot.RustShot` at object path `/`. Methods (Flameshot-CLI-style names):

| Method                                              | Args                                                          |
| --------------------------------------------------- | ------------------------------------------------------------- |
| `graphicCapture(s, u, s)`                           | `path`, `delay_ms`, `id`                                      |
| `graphicCaptureFlags(s, u, b, b, s)`                | `path`, `delay_ms`, `clipboard`, `no_save`, `id`              |
| `fullScreen(s, u, s)` / `fullScreenFlags(s, u, b, b, s)` | as above                                                  |
| `captureScreen(i, s, u, s)` / `captureScreenFlags(i, s, u, b, b, s)` | `screen_index` (-1 = cursor's screen) + above |

Empty `path` triggers auto-save to `defaults.save_dir` + `defaults.filename_pattern`. The `*Flags` variants accept the extended `clipboard` and `no_save` booleans.

## Architecture

```
src/
├── main.rs                     # mode dispatch (no args = daemon, else CLI client)
├── cli.rs                      # clap CLI definitions
├── client.rs                   # CLI → DBus proxy
├── daemon.rs                   # main thread runs UI loop; DBus listener on bg thread
├── dbus/mod.rs                 # zbus interface impl
├── config.rs                   # TOML config + defaults + path/color/tool helpers
├── error.rs                    # thiserror types
├── capture/
│   ├── mod.rs                  # Screen type
│   └── x11.rs                  # x11rb GetImage + xrandr + XFixes cursor composite
├── canvas/
│   ├── mod.rs                  # Annotation enum, Canvas state, undo/redo stacks
│   ├── geometry.rs             # Pos, Bounds
│   └── render.rs               # rasterize annotations onto image (tiny-skia + imageproc)
├── ui/
│   ├── mod.rs                  # UiRequest / UiResult channel types
│   ├── overlay.rs              # eframe overlay app, region select + tool dispatch
│   └── toolbar.rs              # bottom toolbar widget
└── export/
    ├── mod.rs
    ├── file.rs                 # PNG save (creates parent dirs)
    └── clipboard.rs            # shells out to xclip for persistent ownership
```

The X11/winit thread constraint (winit owns the main thread on Linux X11) drives the daemon's structure: the main thread runs `eframe::run_native` per overlay invocation, while the DBus listener runs on a background tokio thread and forwards capture requests over a `crossbeam_channel`.

The `Annotation` enum + `canvas::render` match is the central DRY point — adding a new tool is one new variant + one new arm. Tool dispatch in `ui/overlay.rs` is just a state machine over the `Draft` enum.

## Build from source

```bash
cargo build --release
./target/release/rustshot --version
```

Profile settings in `Cargo.toml` enable `lto = "thin"`, `codegen-units = 1`, and `strip = true`.

## License

GPL-3.0-or-later. See [`LICENSE`](LICENSE).

The embedded font in `assets/font.ttf` is a digit-only subset of DejaVuSans-Bold (Bitstream Vera license).
