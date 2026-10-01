"""AION example project — a Python service whose startup cost is driven by ``config.json``.

The bundled AION experiment switches ``startup.preload`` off and measures the difference:

    aion init
    aion experiment create service-startup-test --from service-startup-test.yaml
    aion experiment run service-startup-test
"""

import json
import sys
import time


def burn(milliseconds):
    """Busy work (not sleep) so an observer sees real CPU time."""
    deadline = time.perf_counter() + milliseconds / 1000.0
    acc = 0
    while time.perf_counter() < deadline:
        for value in range(20_000):
            acc = (acc + value * 2_654_435_761) % (2**64)
    return acc


def main():
    started = time.perf_counter()
    try:
        with open("config.json", "r", encoding="utf-8") as handle:
            config = json.load(handle)
    except (OSError, ValueError) as error:
        print("service: cannot read config.json: {}".format(error), file=sys.stderr)
        return 1

    startup = config.get("startup", {})
    if startup.get("preload", False):
        burn(startup.get("preload_entries", 30) * startup.get("preload_entry_ms", 4))
    burn(startup.get("init_ms", 60))

    elapsed_ms = (time.perf_counter() - started) * 1000.0
    print(
        "service ready in {:.3} ms (preload: {}, threads: {})".format(
            elapsed_ms,
            startup.get("preload"),
            config.get("worker", {}).get("threads"),
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
