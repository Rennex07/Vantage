#!/bin/sh

case "${1-}" in
    prereqs) exit 0 ;;
esac

case " $(cat /proc/cmdline) " in
    *' nosplash '*) exit 0 ;;
    *' splash '*) ;;
    *) exit 0 ;;
esac

[ -n "${rootmnt-}" ] || exit 0
grep -Eq '^Theme[[:space:]]*=[[:space:]]*vantage[[:space:]]*$' \
    "$rootmnt/etc/plymouth/plymouthd.conf" 2>/dev/null || exit 0
plymouth --ping || exit 0

# Use the installed system's local time while Plymouth still has its initramfs fonts.
boot_time=$(chroot "$rootmnt" /usr/bin/date +%H:%M 2>/dev/null) || exit 0
plymouth display-message --text="$boot_time" || true
