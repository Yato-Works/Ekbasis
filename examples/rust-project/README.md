# examples/rust-project — the AION demo target

A tiny dependency-free Rust service (`bloom`) whose startup cost is driven by `config.toml`.
It exists to be *measured*:

```powershell
# from this directory
cargo build --release
target/release/bloom.exe          # prints the startup time it observed
```

## Run the bundled experiment

Copy this project into a scratch git repository (AION measures commits, so the project has to be
committed first), then:

```powershell
git init; git add -A; git -c user.name=me -c user.email=me@example.com commit -m "initial"
aion init
aion experiment create bloom-startup-test --from bloom-startup-test.yaml
aion experiment run bloom-startup-test
```

The experiment turns `startup.preload` off in `config.toml`, commits that change on a branch
called `aion/bloom-startup-test`, then measures both states and compares them:

```text
metric            baseline    experiment  difference          verdict
startup (mean)    182.4 ms    62.1 ms     -65.9% (120.3 ms)   improved
peak memory       3.1 MiB     3.0 MiB     -3.2% (102.4 KiB)   improved
```

Nothing in that table was predicted: every number comes from processes AION actually started.

## Files

| file | purpose |
| --- | --- |
| `src/main.rs` | the service (burns CPU according to the config) |
| `config.toml` | the configuration the experiment edits |
| `bloom-startup-test.yaml` | the AION experiment definition |

> On Unix, change `command.run` in the spec to `./target/release/bloom`.
