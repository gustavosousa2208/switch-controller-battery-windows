# Switch-mode controller battery in Windows Settings

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
   arriving, the tool sends the standard `0x03 0x30` subcommand itself. No
   Steam, game, or companion app is needed for battery data to flow, and it
   re-sends the request automatically if the stream stops.
3. **Publish** - it writes the canonical Windows battery property schema on
   the controller's Bluetooth devnode:

   | Property `{104EA319-6EE2-4701-BD47-8DDBF425BBE5}` | Type | Meaning |
   |---|---|---|
   | PID 2 | `Byte` | battery percent |
   | PID 3 | `Boolean` | charging |
   | PID 7 | `FileTime` | last update |

   Windows Settings displays the value for the paired controller entry.

Verified end to end with an 8BitDo Lite 2 in Switch mode on Windows 11
(build 26200): the badge shows the real level, updates live, disappears when
the controller powers off and returns when it reconnects.

## Requirements

- Windows 10 or 11
- Python 3.8+ with [hidapi](https://pypi.org/project/hidapi/):
  `python -m pip install hidapi`
- A controller in Switch / Pro Controller mode. Developed with the
  **8BitDo Lite 2**; other controllers that emulate the Nintendo Switch Pro
  Controller should work identically.
- Administrator rights only for the scheduled task (writing devnode
  properties requires elevation).

## Install (recommended): hidden logon task

From an **elevated** PowerShell in this folder:

```powershell
powershell -ExecutionPolicy Bypass -File install_task.ps1
```

This registers a `SwitchControllerBattery` task that:

- starts at logon (30 s delay) and runs `pythonw.exe` hidden (no console),
- uses highest privileges (required for the property writes),
- **retries every minute if the publisher is not running** (watchdog), so a
  crash or kill is repaired within ~1 minute,
- logs to `logs\publisher.log` (rotated at 1 MB).

The installer also verifies the watchdog by killing the process once and
watching it come back (skip with `-SkipRestartTest`).

Uninstall:

```powershell
powershell -ExecutionPolicy Bypass -File install_task.ps1 -Remove
```

## Manual use

```powershell
python battery_publisher.py     # elevated: keeps the Settings badge updated
python battery_reader.py        # no elevation needed: prints + logs samples
```

`battery_reader.py` writes `battery_samples.csv` and is a quick way to verify
what the controller reports.

## Diagnostics

`devnode_property_test.py` inspects the battery properties on the controller
devnode (auto-discovered by VID/PID):

```powershell
python devnode_property_test.py --action list
python devnode_property_test.py --action read
python devnode_property_test.py --action set --value 42   # elevation needed
python devnode_property_test.py --action clear            # elevation needed
```

## Limitations / notes

- Bluetooth **classic** (HID) controllers only; the tool targets the classic
  Switch input protocol.
- The percent comes from the controller's coarse level code (0..8) with a
  linear mapping; the controller itself updates it slowly.
- Devnode properties are soft: they are cleared when the device
  re-enumerates, which is why the publisher re-asserts them (heartbeat every
  5 s) and clears them on disconnect.
- Writing devnode properties and registering a privileged task may be flagged
  by some antivirus/EDR suites; all code here is small and readable.
- If the full-report request is rejected by a particular driver stack, the
  publisher falls back to passive mode (it still tracks battery whenever
  `0x30`/`0x21` reports arrive).

## Credits

- Protocol documentation:
  [dekuNukem/Nintendo_Switch_Reverse_Engineering](https://github.com/dekuNukem/Nintendo_Switch_Reverse_Engineering)
  and SDL's `SDL_hidapi_switch.c`.
- Not affiliated with Nintendo or 8BitDo.

## License

MIT - see [LICENSE](LICENSE).
