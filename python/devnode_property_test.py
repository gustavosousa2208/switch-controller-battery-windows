"""Read/write/clear the Windows battery properties on the controller devnode.

The controller devnode is auto-discovered by VID/PID (057E/2009) unless
--instance is given.

Actions:
  list   - print matching controller devnodes
  read   - read the battery properties (level / charging / timestamp)
  set    - write a test value (--value, default 42) - requires elevation
  clear  - delete the battery properties - requires elevation

Note: devnode properties are soft and get cleared automatically when the
device re-enumerates (e.g. reconnect) - which is why the publisher re-asserts
them periodically.
"""

from __future__ import annotations

import argparse
import sys

try:
    import hid  # noqa: F401  (hidapi; imported here for a friendly error)
except ImportError:
    sys.exit("hidapi is missing. Install it with: python -m pip install hidapi")

import switch_protocol as sp


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--action", required=True,
                    choices=["list", "read", "set", "clear"])
    ap.add_argument("--value", type=int, default=42,
                    help="test battery percentage for 'set' (default 42)")
    ap.add_argument("--instance", default=None,
                    help="devnode instance id (default: auto-discover)")
    args = ap.parse_args()

    found = sp.find_controller_instances()
    if args.action == "list":
        if not found:
            print("no matching controller devnodes found (VID 057E PID 2009)")
            return 1
        for item in found:
            print(item)
        return 0

    instance = args.instance or (found[0] if found else None)
    if instance is None:
        print("no matching controller devnode found; pass --instance")
        return 1
    if len(found) > 1 and not args.instance:
        print("multiple controllers found; using the first "
              "(override with --instance)")

    print(f"target: {instance}")

    if args.action == "read":
        devinst = sp.locate(instance)
        if devinst is None:
            print("devnode not found")
            return 1
        for prop_id, name in ((sp.BATTERY_PID_LEVEL, "level"),
                              (sp.BATTERY_PID_CHARGING, "charging"),
                              (sp.BATTERY_PID_TIMESTAMP, "timestamp")):
            cr, ptype, size, data = sp.read_prop(devinst,
                                                 sp.battery_key(prop_id))
            print(f"PID {prop_id} ({name}): cr=0x{cr:02x} type=0x{ptype:x} "
                  f"value={data!r}")
        return 0

    if args.action == "set":
        devinst, crs = sp.write_battery(instance, args.value, False)
        if devinst is None:
            print("devnode not found")
            return 1
        crs_text = ", ".join("0x%02x" % c for c in crs)
        print(f"wrote level={args.value}% charging=False (crs={crs_text})")
        return 0 if all(cr == 0 for cr in crs) else 1

    if args.action == "clear":
        if not sp.clear_battery(instance):
            print("devnode not found")
            return 1
        print("battery properties cleared")
        return 0

    return 0


if __name__ == "__main__":
    sys.exit(main())
