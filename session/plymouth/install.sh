#!/bin/sh
set -eu

if [ "$(id -u)" -ne 0 ]; then
    echo "Run this installer with sudo." >&2
    exit 1
fi

base=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
themes=/usr/share/plymouth/themes
if [ ! -f "$themes/spinner/entry.png" ] || ! command -v plymouth-set-default-theme >/dev/null; then
    echo "Install Debian's plymouth and plymouth-themes packages first." >&2
    exit 1
fi

install -d -m 755 "$themes/vantage"
install -m 644 "$base/vantage/vantage.plymouth" "$themes/vantage/vantage.plymouth"
install -m 644 "$base/vantage/wordmark.png" "$themes/vantage/watermark.png"
# Use the standard Plymouth spinner and its built-in password dialog artwork.
for image in "$themes"/spinner/throbber-*.png "$themes"/spinner/animation-*.png     "$themes/spinner/entry.png" "$themes/spinner/bullet.png" "$themes/spinner/lock.png"     "$themes/spinner/capslock.png" "$themes/spinner/keyboard.png" "$themes/spinner/keymap-render.png"
do
    install -m 644 "$image" "$themes/vantage/"
done

plymouth-set-default-theme vantage
update-initramfs -u -k all

# Keep bootloader changes opt-in for installations that already have a splash.
if [ "${1-}" = "--enable-grub-splash" ] && command -v update-grub >/dev/null; then
    install -d -m 755 /etc/default/grub.d
    cat > /etc/default/grub.d/60-vantage-splash.cfg <<'GRUB'
case " $GRUB_CMDLINE_LINUX_DEFAULT " in
    *' quiet '*) ;;
    *) GRUB_CMDLINE_LINUX_DEFAULT="$GRUB_CMDLINE_LINUX_DEFAULT quiet" ;;
esac
case " $GRUB_CMDLINE_LINUX_DEFAULT " in
    *' splash '*) ;;
    *) GRUB_CMDLINE_LINUX_DEFAULT="$GRUB_CMDLINE_LINUX_DEFAULT splash" ;;
esac
case " $GRUB_CMDLINE_LINUX_DEFAULT " in
    *' plymouth.ignore-serial-consoles '*) ;;
    *) GRUB_CMDLINE_LINUX_DEFAULT="$GRUB_CMDLINE_LINUX_DEFAULT plymouth.ignore-serial-consoles" ;;
esac
GRUB
    update-grub
fi
