# switch-battery

Show the battery level of a Nintendo-Switch-mode controller (e.g. an 8BitDo
Lite 2) in **Windows Settings -> Bluetooth & devices**, right on its
"Pro Controller" entry - the same style of battery badge that Bluetooth LE
keyboards and headsets get. No firmware changes, no kernel driver: everything
runs in user mode and is fully reversible.

![Windows Settings showing Pro Controller at 42%](screenshots/settings-battery.png)

## How it works

1. **Read** - the tool talks to the controller's HID interface (the Nintendo
   "Pro Controller" protocol used by Switch-mode controllers, VID `057E`,
   PID `2009`) and reads the battery byte from HID report `0x30` (full) or
   `0x21` (subcommand reply).
2. **Request full reports** - if only simple button reports (`0x3F`) are
   arriving, the tool sends the standard `0x03 0x30` subcommand itself, so no
   Steam, game or companion app is needed. It re-sends the request
   automatically if the stream stops (mode revert, reconnection).
3. **Publish** - it writes the canonical Windows battery property schema on
   the controller's Bluetooth devnode (found automatically by VID/PID):

   | Property `{104EA319-6EE2-4701-BD47-8DDBF425BBE5}` | Type | Meaning |
   |---|---|---|
   | PID 2 | `Byte` | battery percent |
   | PID 3 | `Boolean` | charging |
   | PID 7 | `FileTime` | last update |

   Windows Settings displays the value for the paired controller entry, live.

Verified end to end with an 8BitDo Lite 2 in Switch mode on Windows 11
(build 26200): the badge shows the real level, updates live, disappears when
the controller powers off and returns when it reconnects. Other controllers
that emulate the Nintendo Switch Pro Controller should work identically.

## Quick start (single executable)

Build (Rust 1.75+; produces one self-contained exe, no runtime needed):

```powershell
cargo build --release
```

The result is `target\release\switch-battery.exe`.

From an **elevated** terminal:

```powershell
.\switch-battery.exe install     # register the hidden logon task and start it
.\switch-battery.exe status      # task state + current battery property
.\switch-battery.exe uninstall   # remove the task
```

- `install` registers a `SwitchControllerBattery` scheduled task: starts at
  logon, runs hidden, uses highest privileges, and **retries every minute
  while the publisher is not running** (watchdog). It also verifies the
  watchdog once by killing the process and watching it come back
  (`--no-test` skips that check).
- Logs go to `logs\publisher.log` next to the executable (rotated at 1 MB).
- Manual foreground run: `.\switch-battery.exe run` (Ctrl+C stops and clears).

## Commands

```
switch-battery run [--instance ID] [--heartbeat S] [--refresh S] [--log PATH] [--hidden]
switch-battery install [--no-test]
switch-battery uninstall
switch-battery status
switch-battery read [--instance ID]       # raw property read (diagnostic)
switch-battery set VALUE [--instance ID]  # write a test value (elevated)
switch-battery clear [--instance ID]      # delete the properties (elevated)
```

## Python reference tools

The original implementation lives in `python/` and still works (requires
Python 3.8+ and `pip install hidapi`):

- `python\battery_publisher.py` - the publisher (elevated for writes)
- `python\battery_reader.py` - sampler / CSV logger (no elevation needed)
- `python\devnode_property_test.py` - property diagnostics
- `python\install_task.ps1` - schedules the Python variant as the
  `SwitchControllerBatteryPython` task

The Rust executable replaces all of them for end users.

## Limitations / notes

- Bluetooth **classic** (HID) controllers only.
- The percent is derived from the controller's coarse level code (0..8) with
  a linear mapping; the controller itself updates it slowly.
- Devnode properties are soft: they are cleared when the device
  re-enumerates, which is why the publisher re-asserts them every 5 s
  (heartbeat) and clears them on disconnect.
- Writing devnode properties and registering a privileged task may be flagged
  by some antivirus/EDR suites; the code here is small, readable and open.
- If the full-report request is rejected by a particular driver stack, the
  publisher falls back to passive mode (it still tracks battery whenever
  `0x30`/`0x21` reports arrive, e.g. while a game or Steam negotiates them).

## Credits

- Protocol documentation:
  [dekuNukem/Nintendo_Switch_Reverse_Engineering](https://github.com/dekuNukem/Nintendo_Switch_Reverse_Engineering)
  and SDL's `SDL_hidapi_switch.c`.
- Not affiliated with Nintendo or 8BitDo.

## License

MIT - see [LICENSE](LICENSE).
