# Vantage

Vantage is a desktop environment for Debian 13 on x86-64. Its Wayland
compositor is built in Rust with Smithay. The shell uses the Contours
wallpaper, a navy status bar, and a frosted dock. Click the indicators at
the top right to see network and USB storage status. Open **Console** or **Files** from the dock, or use **1**,
**2**, and **3** to switch workspaces. Dock apps animate from the clicked
tile into their window. Files opens your home directory in Thunar with
a sortable details list, a breadcrumb path, and a frosted Vantage theme.

## Build on Debian 13

Install a stable Rust toolchain and the build and runtime packages.

```sh
sudo apt install build-essential pkg-config libwayland-dev libxkbcommon-dev \
    libdrm-dev libgbm-dev libudev-dev libinput-dev libseat-dev \
    libegl-mesa0 libgl1-mesa-dri libxkbcommon-x11-0 libgtk-4-dev \
    libvte-2.91-gtk4-dev fonts-jetbrains-mono thunar dbus-daemon
cargo build --locked --bins
```

## Run the standalone session

Log in as a regular user at a local Linux virtual terminal and run:

```sh
./target/debug/vantage --tty
```

The session uses libseat to access graphics and input devices on the
active seat. It checks the primary GPU first, then other GPUs on the seat,
and selects one connected output and a supported mode. On startup it
reports the DRM device, output, GLES renderer, logical CPUs available
to the process, and Wayland socket. Hardware rendering depends on the
installed driver and graphics device. A renderer line containing `llvmpipe`
indicates software rendering.

Use **Ctrl+Alt+Esc** to leave the session, or **Ctrl+Alt+F1–F12** to switch
virtual terminals. Console starts in your home directory and supports
scrollback and **Ctrl+Shift+C** / **Ctrl+Shift+V**. Type `exit` to close
the Console window.

## Run the nested preview

Launch from an existing graphical session:

```sh
./target/debug/vantage --nested
```

Starting without an option also opens the nested preview. After rebuilding,
restart the running compositor to load the updated shell. A Wayland
client can use the socket printed by either backend. For example, if
Vantage prints `WAYLAND_DISPLAY=wayland-1`:

```sh
WAYLAND_DISPLAY=wayland-1 weston-terminal
```

## License

GPL-3.0-only. The Contours image comes from the Vantage-OS simulator's wallpaper.
