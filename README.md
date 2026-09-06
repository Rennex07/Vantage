# Vantage

An open-source embedded OS designed for DIY smartphones and handhelds. Built on ESP-IDF, FreeRTOS, and LVGL, featuring dynamic ELF application loading (.vpk) and real-time window management

Language: C/C++

## Dependencies
* [ESP-IDF](https://dl.espressif.com/dl/eim/index.html) - Main framework
* [LVGL](https://github.com/lvgl/lvgl) - GUI
* [LovyanGFX](https://github.com/lovyan03/LovyanGFX) - Drivers
* [cJSON](https://github.com/Davegamble/cjson) - JSON handler (for saving files I guess..)

## Desktop simulator

The SDL simulator in `sim/` now follows the device UI model. It provides the Home screen, animated app launches,
Back/Home/Recents window management, the on-screen keyboard, Wi-Fi connection
flow, file manager, text editor, create/rename/delete operations, a VPK install
flow, installed-app visibility, and the OTA status flow.

Build it from the repository root:

```powershell
cmake -S sim -B build
cmake --build build --config Debug --target vantage_sim
.\build\Debug\vantage_sim.exe
```

The simulator deliberately keeps side effects separate from a real device:

- Its virtual `/int` and `/sdcard` drives are folders below `sim_data/`.
- Wi-Fi results and connection state are deterministic; it never changes the
  computer's wireless connection. Credentials are stored only in
  `sim_data/system/wifi.txt`.
- The VPK flow exercises installation, registry, Home, and lifecycle UI. An
  ESP ELF is not executable on a desktop; an installed package opens a native
  desktop stand-in until the app is rebuilt for the simulator ABI.
- OTA is represented visually and never writes firmware or changes a boot
  partition.
