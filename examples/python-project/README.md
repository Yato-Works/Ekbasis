# examples/python-project — the AION demo target (Python)

A dependency-free Python service whose startup cost is driven by `config.json`.

```console
$ python main.py
service ready in 181.532 ms (preload: True, threads: 4)
```

## Run the bundled experiment

```console
$ git init && git add -A && git -c user.name=me -c user.email=me@example.com commit -m "initial"
$ aion init
$ aion experiment create service-startup-test --from service-startup-test.yaml
$ aion experiment run service-startup-test
```

AION branches from `main`, writes `startup.preload = false` into `config.json` (JSON keys are
addressed as `startup.preload`), commits that as an alternative timeline, then builds/runs/measures
both states and compares them.

`command.run` is `python main.py` — no build step is needed, so only the run phase is measured.
