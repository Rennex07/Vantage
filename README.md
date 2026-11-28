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
    pipewire-pulse wireplumber lxpolkit
sudo install -Dm755 target/debug/vantage /usr/local/bin/vantage
sudo install -Dm755 target/debug/vantage-console /usr/local/bin/vantage-console
sudo install -Dm644 session/vantage.desktop /usr/share/wayland-sessions/vantage.desktop
sudo install -Dm644 session/pam/vantage-lock /etc/pam.d/vantage-lock
```

Select **Vantage** at a display manager that supports Wayland sessions, such as
Debian's LightDM. Vantage then runs directly on the active seat. Remove the old
Vantage preview entry from your previous desktop's autostart if you used it.

Open **Quick settings** from the top-right controls. Open **Settings** there or
from the app launcher for a window with a searchable category sidebar. Use
**Date & Time** to change the system time zone. **Appearance** offers a warm Light theme with the Opal
wallpaper and seven Dark colors with matching Contours wallpaper and glass, as well as effects, solid
surfaces, and reduced motion under **Accessibility**. Appearance changes apply to the shell, Settings,
app search, and Console as they run. Files uses the palette at launch.
**Sound** adjusts volume, mute, and the default output
device directly, with **Advanced mixer** for input devices and per-app volume.
**Display** shows the current mode and adjusts software brightness for the
Vantage session. **Connectivity** opens NetworkManager for Wi-Fi and Blueman
for Bluetooth. The top panel reports battery and USB storage status when available.

Open **Quick settings → Power** to lock or shut down. **F9** and **Super+L**
also lock the session. After five seconds, the wallpaper dims as the clock
moves down and grows. While the always-on display is active, press a key or
left double-click to wake it and show the password field. The wake key is
consumed. Enter the password after the field appears. Vantage verifies the password with Debian PAM. Set a Debian
password before locking an account configured for automatic login. In
**Settings → Lock Screen**, choose full-screen wallpaper, clock only, or off.
**Shut down** prompts for confirmation.
**Alt+F4** closes the focused app.

## Install the boot splash

Run this from the source tree to install Vantage's Plymouth theme on Debian
with GRUB. It adds a GRUB configuration snippet and updates the initramfs.

```sh
sudo apt install plymouth plymouth-themes
sudo sh session/plymouth/install.sh --enable-grub-splash
```

The graphical splash shows the Vantage wordmark and the local time once the
system disk is mounted. Use **Esc** during the splash to view boot messages.
If your bootloader already passes `splash` and suppresses routine console
output, omit `--enable-grub-splash`.
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
and selects one connected output. Vantage tries the output's preferred mode
first, then other modes by resolution and refresh rate, and falls back if the
driver cannot activate a mode. To prefer a particular startup resolution,
set `VANTAGE_DISPLAY_SIZE=1920x1080` in the session environment. Vantage
reports when it cannot use the requested size and tries the remaining modes.
Changes to this setting take effect at the next session start.

The dock and top controls scale up on tall,
high-resolution outputs. On startup it
reports the DRM device, output, GLES renderer, logical CPUs available
to the process, and Wayland socket. Hardware rendering depends on the
installed driver and graphics device. A renderer line containing `llvmpipe`
indicates software rendering.

Native mode selects solid shell material on known software renderers by
default. Accelerated SVGA3D retains compositor GLES frost. On systems with
`vmwgfx`, Vantage-launched GTK4 clients use Cairo by default to avoid virtual
GPU rendering defects; an explicit `GSK_RENDERER` overrides this choice.
Set `VANTAGE_GRAPHICS=full` to compare with the configured effects, or
`VANTAGE_GRAPHICS=safe` to force solid material in either backend.

Set `VANTAGE_FRAME_METRICS=1` to report median, P95 and P99 CPU times for
buffer binding, scene drawing, and submission. Native mode also reports DRM
submit-to-callback latency and how many frames took more CPU time than one mode
interval. Native DRM sequence gaps are reported for continuous animations,
excluding idle periods and cancelled submissions. The blur diagnostics count
cache hits, recomputations and Gaussian/downsample passes. Reports group 300
samples, with a final partial group on normal shutdown. Use the same mode and
interaction sequence when comparing profiles. These are CPU and DRM sequence
measurements, not GPU execution timings or hardware presentation timestamps.

Full effects use detailed frost and reflections; Balanced reduces blur
resolution and simplifies reflections and grain; Reduced uses solid surfaces.
The blur cache conservatively reuses a complete unchanged single-panel
backdrop and invalidates on client commits, visual changes and lock transitions.

The dock raises or restores matching windows across workspaces and shows which
apps are running. Repeated clicks cycle matching windows. Right-click an app
to select a window by title and workspace, cycle windows, or request a new
window (application-dependent). The menu lists up to eight matching windows
when space permits. Use **Applications** to pin and unpin apps. Minimize and
restore use a reversible shrink/fade transition unless reduced motion,
fullscreen, or another incompatible transition requires an immediate change.
In the native session, open **Alerts** or press **Super+N** to review
notifications grouped by app label. Mute suppresses banners for that label
without deleting history; Unmute all clears these session-only preferences.
Do Not Disturb hides banners, and the notification center is private while
locked. App labels are client-supplied, not authenticated identities.

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
