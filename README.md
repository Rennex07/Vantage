# Vantage

Vantage is a desktop environment for Debian 13 on x86-64. Its Wayland
compositor is built in Rust with Smithay. The shell uses the Contours
wallpaper, the Pi 4 wordmark, and a frosted dock. Click **VANTAGE** or press
**Super+Space** to search installed apps. Use **1**, **2**, and **3** in the
top bar to switch workspaces. **Console** and **Files** open from the dock.
Files uses Thunar with sortable details, breadcrumbs, and a frosted theme.

Click the overview icon beside the workspace buttons or press **Super+Tab**
to zoom out. Click a preview to switch windows. You can also use the arrow
keys or **Tab**, then **Enter**. **Esc** goes back to the desktop. Hold **Alt**
and tap **Tab** to move through the previews, then release **Alt** to switch.
Add **Shift** to cycle in reverse.

Drag a window edge or corner to resize. Drag the title bar to the left or
right edge to tile a window, or drag it to the top to maximize it. **Super+Left**
and **Super+Right** tile the focused window. **Super+Up** toggles maximization.
Repeating the same layout shortcut restores the window's floating size.
**Super+Down** minimizes. Restore a minimized window from the overview, or
click its dock icon for Console or Files.

## Build on Debian 13

Install a stable Rust toolchain and the build and runtime packages.

```sh
sudo apt install build-essential pkg-config libwayland-dev libxkbcommon-dev \
    libdrm-dev libgbm-dev libudev-dev libinput-dev libseat-dev \
    libegl-mesa0 libgl1-mesa-dri libxkbcommon-x11-0 libgtk-4-dev \
    libvte-2.91-gtk4-dev fonts-jetbrains-mono fonts-roboto thunar dbus-daemon
cargo build --locked --bins
```

## Install the login session

On Debian 13, install the desktop utilities alongside the build dependencies above.

```sh
sudo apt install network-manager blueman pavucontrol pulseaudio-utils \
    pipewire-pulse wireplumber lxpolkit swaylock
sudo install -Dm755 target/debug/vantage /usr/local/bin/vantage
sudo install -Dm755 target/debug/vantage-console /usr/local/bin/vantage-console
sudo install -Dm644 session/vantage.desktop /usr/share/wayland-sessions/vantage.desktop
```

Select **Vantage** at a display manager that supports Wayland sessions, such as
Debian's LightDM. Vantage then runs directly on the active seat. Remove the old
Vantage preview entry from your previous desktop's autostart if you used it.

Open **Quick settings** from the top-right controls. **Settings** also appears
in the app launcher. **Appearance** offers a warm Light theme with the Opal
wallpaper and seven Dark colors with matching Contours wallpaper and glass, as well as effects, solid
surfaces, and reduced motion. Appearance changes apply to the shell, Settings,
app search, and Console as they run. Files uses the palette at launch.
**Sound** adjusts volume, mute, and the default output
device directly, with **Advanced mixer** for input devices and per-app volume.
**Display** shows the current mode and adjusts software brightness for the
Vantage session. **Connectivity** opens NetworkManager for Wi-Fi and Blueman
for Bluetooth. The top panel reports battery and USB storage status when available.

Open **Quick settings → Power** to lock, suspend, shut down, or log out.
**Super+L** also locks the session. The lock screen dims to full-screen
always-on display after six seconds. Press a key or click to wake and use your
Debian password to unlock. Set a password first if Debian uses automatic login
with a locked account. In **Settings → Appearance**, choose full-screen
wallpaper, clock only, or off. **Suspend** requests sleep after the lock is
visible. **Shut down** and **Log out** prompt for confirmation.
**Alt+F4** closes the focused app.

## Install the boot splash

Run this from the source tree to install Vantage's Plymouth theme on Debian
with GRUB. It adds a GRUB configuration snippet and updates the initramfs.

```sh
sudo apt install plymouth plymouth-themes
sudo sh session/plymouth/install.sh --enable-grub-splash
```

Use **Esc** during the splash to view boot messages. If your bootloader already
passes the `splash` kernel argument, omit `--enable-grub-splash`.
The splash wordmark is rendered with Debian's Roboto Black face. To regenerate
it after updating the artwork, run
`target/debug/vantage --render-boot-wordmark session/plymouth/vantage/wordmark.png`.

## Run from a local terminal

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

Set `VANTAGE_FRAME_METRICS=1` when starting either backend to report median,
P95 and P99 frame preparation and submission times to stderr. Reports group
300 submitted frames, with a final partial group on normal shutdown. Native
mode also measures the time from submission to handling the DRM callback.
These timings describe CPU work and callback arrival. Hardware presentation
and missed refresh intervals still need separate measurement.

Use **Ctrl+Alt+Esc** to open the logout confirmation, or **Ctrl+Alt+F1–F12**
to switch virtual terminals. Console starts in your home directory and supports
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

Use the VirtualBox guest **Vantage (x86-64)** to test the installed session.
Use the nested preview for shell iteration and the standalone session for
login, input, display and device behavior. The compositor currently drives one
connected monitor.

## Run legacy X11 applications

The native session starts Xwayland when [xwayland-satellite](https://github.com/Supreeeme/xwayland-satellite)
is on its path. Install Debian's `xwayland` and `libxcb-cursor0` packages and
install xwayland-satellite 0.8.3 or a compatible newer version. Log out and
back in after installation. Start X11 apps from Vantage's launcher or Console.

## License

GPL-3.0-only. The Contours wallpaper and wordmark come from Vantage-OS.
