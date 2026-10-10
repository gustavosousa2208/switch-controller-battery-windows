"""Sample the controller battery from HID reports (no elevation needed).

Prints one decoded sample per second and appends rows to battery_samples.csv.
The tool asks the controller for full reports (0x03/0x30) itself, so battery
data flows even on a fresh connection where only button reports (0x3f) would
otherwise arrive.
"""

from __future__ import annotations

import csv
import sys
import time
from datetime import datetime
from pathlib import Path

try:
    import hid  # noqa: F401  (hidapi; imported here for a friendly error)
except ImportError:
    sys.exit("hidapi is missing. Install it with: python -m pip install hidapi")

import switch_protocol as sp


def main() -> int:
    print("looking for a Switch-mode controller (VID 057e, PID 2009)...")
    dev = sp.open_controller()
    if dev is None:
        sys.exit("controller HID interface not found")

    counter = 0
    if sp.request_full_mode(dev, counter):
        counter = (counter + 1) & 0x0F
        print("requested full reports (subcommand 0x03/0x30)")

    log_name = "battery_samples.csv"
    is_new = not Path(log_name).exists()
    print(f"sampling to {log_name}; Ctrl+C to stop\n")

    with open(log_name, "a", newline="", encoding="ascii") as fh:
        writer = csv.writer(fh)
        if is_new:
            writer.writerow(["timestamp", "report_id", "battery_byte",
                             "level", "percent", "charging"])
        last_print = 0.0
        try:
            while True:
                report = dev.read(128)
                if report:
                    raw = bytes(report)
                    if len(raw) >= 3 and raw[0] in sp.BATTERY_REPORT_IDS:
                        battery_byte = raw[2]
                        level = (battery_byte & 0xE0) >> 4
                        percent = sp.level_to_percent(level)
                        charging = bool(battery_byte & 0x01)
                        now = time.monotonic()
                        if now - last_print >= 1.0:
                            last_print = now
                            stamp = datetime.now().isoformat(timespec="seconds")
                            print(f"{stamp} battery: {percent}% "
                                  f"(level {level}/8)"
                                  f"{' charging' if charging else ''}")
                            writer.writerow([stamp, hex(raw[0]),
                                             f"0x{battery_byte:02x}",
                                             level, percent, int(charging)])
                            fh.flush()
                time.sleep(0.002)
        except KeyboardInterrupt:
            print("\nstopped")
        except OSError as exc:
            print(f"\ncontroller disconnected or read failed: {exc}")
            print("reconnect the controller and run again")
    dev.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
