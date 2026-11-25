# Vantage input test

This Rust and Smithay test opens a black screen with a white arrow cursor. Hold
left, right, or middle mouse buttons to show a circle, square, or triangle. The
shape follows the cursor. Press Escape to exit.

The test currently uses Smithay's winit backend on a minimal Xorg server. It
does not require a desktop environment, but it is not yet a direct DRM backend
or a standalone Wayland session.

On the configured Debian virtual machine, tty1 logs in automatically. Press F1
to start the test. The launcher uses `startx` and the F1 binding is installed
in the user's Bash configuration. The VM uses the standard Debian kernel for
the virtual display.
