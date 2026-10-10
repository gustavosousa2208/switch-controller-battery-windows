"""Publish a Switch-mode controller's battery to Windows Settings.

Reads the battery byte from the controller's HID reports, switches the
controller to full report mode itself (subcommand 0x03/0x30 - no Steam or
other host app required) and keeps the canonical Windows battery properties
updated on the controller's Bluetooth devnode, so Windows Settings ->
Bluetooth & devices shows the level next to the controller entry.

Requires elevation (CM_Set_DevNode_PropertyW). Stop with Ctrl+C; exiting
clears the properties so no stale value is left behind. Use install_task.ps1
to run this hidden at logon.

The controller devnode is auto-discovered by VID/PID (057E/2009); use
--instance to override.
"""

from __future__ import annotations

import argparse
import ctypes
import pathlib
import sys
import time
from datetime import datetime

try:
    import hid  # noqa: F401  (hidapi; imported here for a friendly error)
except ImportError:
    sys.exit("hidapi is missing. Install it with: python -m pip install hidapi")

import switch_protocol as sp


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--instance", default=None,
                    help="Bluetooth devnode instance id (default: auto-discover)")
    ap.add_argument("--heartbeat", type=float, default=5.0,
                    help="seconds between property re-asserts (default 5)")
    ap.add_argument("--refresh", type=float, default=30.0,
                    help="seconds without 0x30 reports before re-requesting "
                         "full mode (0 disables; default 30)")
    ap.add_argument("--log",
                    default=str(pathlib.Path(__file__).parent /
                                "logs" / "publisher.log"),
                    help="log file ('' disables; default logs/publisher.log)")
    args = ap.parse_args()

    if not ctypes.windll.shell32.IsUserAnAdmin():
        print("WARNING: not elevated; property writes will fail.")
        print("Run this from an elevated console (or install the logon task).")

    if args.log:
        log_path = pathlib.Path(args.log)
        try:
            log_path.parent.mkdir(parents=True, exist_ok=True)
            if log_path.exists() and log_path.stat().st_size > 1_000_000:
                log_path.replace(log_path.parent / (log_path.name + ".1"))
            log_fh = open(log_path, "a", encoding="utf-8", buffering=1)
            sys.stdout = sp.Tee(sys.stdout, log_fh)
            sys.stderr = sp.Tee(sys.stderr, log_fh)
            print(f"log file: {log_path}")
        except OSError as exc:
            print(f"log file unavailable ({exc}); console only",
                  file=sys.stderr)

    instance_id = args.instance
    if instance_id is None:
        found = sp.find_controller_instances()
        if found:
            instance_id = found[0]
            if len(found) > 1:
                print("multiple matching controllers found; using the first:")
                for item in found:
                    print(f"  {item}")
    print(f"controller devnode: {instance_id or '(not found yet)'}")
    print("publisher running; Ctrl+C to stop and clear\n")

    def ensure_instance():
        nonlocal instance_id
        if instance_id is None:
            found = sp.find_controller_instances()
            if found:
                instance_id = found[0]
                print(f"controller devnode: {instance_id}")
        return instance_id

    last_written = None      # (percent, charging)
    last_write_time = 0.0
    dev = None
    seen_battery = False
    last_hint = 0.0
    last_not_found_log = None
    packet_counter = 0
    last_0x30_seen = 0.0

    while True:
        try:
            if dev is None:
                dev = sp.open_controller()
                if dev is None:
                    now = time.monotonic()
                    if (last_not_found_log is None
                            or now - last_not_found_log >= 60):
                        last_not_found_log = now
                        print("controller not found; retrying every 3 s")
                    time.sleep(3)
                    continue
                seen_battery = False
                last_hint = time.monotonic()
                last_0x30_seen = time.monotonic()
                if args.refresh:
                    ok = sp.request_full_mode(dev, packet_counter)
                    packet_counter = (packet_counter + 1) & 0x0F
                    print("requested full mode (0x03/0x30)" if ok else
                          "full-mode request rejected; continuing passively "
                          "(will retry)")

            report = dev.read(128)
            if report:
                raw = bytes(report)
                if len(raw) >= 3 and raw[0] in sp.BATTERY_REPORT_IDS:
                    seen_battery = True
                    if raw[0] == 0x30:
                        last_0x30_seen = time.monotonic()
                    battery_byte = raw[2]
                    level = (battery_byte & 0xE0) >> 4
                    percent = sp.level_to_percent(level)
                    charging = bool(battery_byte & 0x01)
                    now = time.monotonic()
                    if (percent, charging) != last_written:
                        instance = ensure_instance()
                        devinst, crs = sp.write_battery(instance, percent,
                                                        charging)
                        stamp = datetime.now().strftime("%H:%M:%S")
                        if devinst is None:
                            print(f"{stamp} devnode not found; write skipped")
                        else:
                            ok = all(cr == 0 for cr in crs)
                            crs_text = ", ".join("0x%02x" % c for c in crs)
                            print(f"{stamp} src=0x{raw[0]:02x} "
                                  f"level={level}/8 -> {percent}%"
                                  f"{' charging' if charging else ''} "
                                  f"written={'ok' if ok else 'FAILED'} "
                                  f"(crs={crs_text})")
                        last_written = (percent, charging)
                        last_write_time = now
                elif not seen_battery:
                    now = time.monotonic()
                    if now - last_hint >= 30:
                        last_hint = now
                        print("waiting for battery reports; the full-mode "
                              "request has been sent")

            # Heartbeat: re-assert the last published value so soft devnode
            # properties (cleared on device re-arrival) stay alive.
            now_hb = time.monotonic()
            if (last_written is not None
                    and now_hb - last_write_time >= args.heartbeat):
                instance = ensure_instance()
                if instance is not None:
                    _, crs = sp.write_battery(instance, *last_written)
                    if not all(cr == 0 for cr in crs):
                        stamp = datetime.now().strftime("%H:%M:%S")
                        crs_text = ", ".join("0x%02x" % c for c in crs)
                        print(f"{stamp} re-assert FAILED (crs={crs_text})")
                    last_write_time = now_hb

            # Refresh watchdog: if the full (0x30) stream stopped, ask again.
            if (dev is not None and args.refresh
                    and time.monotonic() - last_0x30_seen >= args.refresh):
                ok = sp.request_full_mode(dev, packet_counter)
                packet_counter = (packet_counter + 1) & 0x0F
                stamp = datetime.now().strftime("%H:%M:%S")
                print(f"{stamp} no 0x30 reports for {args.refresh:.0f}s; "
                      f"re-requested full mode "
                      f"({'ok' if ok else 'write rejected'})")
                last_0x30_seen = time.monotonic()

            time.sleep(0.005)

        except KeyboardInterrupt:
            print("\nstopping (Ctrl+C)")
            break
        except OSError as exc:
            print(f"\ncontroller disconnected or read failed: {exc}")
            try:
                dev.close()
            except Exception:
                pass
            dev = None
            last_written = None  # write immediately after reconnect
            try:
                instance = ensure_instance()
                if instance and sp.clear_battery(instance):
                    print("cleared battery properties (no stale value)")
                else:
                    print("devnode not found while clearing (skipped)")
            except Exception as exc2:
                print(f"clear failed: {exc2}")
            print("waiting for the controller to come back...")
            time.sleep(1)

    try:
        instance = ensure_instance()
        if instance and sp.clear_battery(instance):
            print("cleared battery properties on exit")
        else:
            print("devnode not found on exit (nothing to clear)")
    except Exception as exc:
        print(f"cleanup failed: {exc}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
