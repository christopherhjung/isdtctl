# gui

A desktop window for ISDT battery chargers, built on [gpui](https://crates.io/crates/gpui).

Finds chargers, binds them, and keeps several connected at once. Live telemetry
and task control per charger: state and progress, input and output, cell
voltages with internal resistance, and buttons to start a charge, a storage
task or a discharge.

```
isdtgui
```

## Finding and binding

The window opens on the discovery panel and scans. Each charger in range gets a
row with a badge saying where it stands:

| Badge | Meaning |
|---|---|
| waiting to bind | In binding mode, so Bind will work |
| known | This host already has an identifier for it, so Connect will work |
| not bound | Neither; put it into binding mode first |
| name unreadable | The advertised name arrived truncated, so its state is unknown. Bind is still offered, and the charger refuses harmlessly if it is not waiting |
| connected | Already open in a tab |

Binding generates an identifier, stores it, and connects. A charger only
accepts a new identifier while it is in binding mode, which is set from the
charger itself.

Identifiers live in the same file `isdtcli` uses, so a charger bound in either
place is known to both.

## Several at once

Each connected charger gets a tab across the top, with a dot showing its state:
green connected, brighter green working, amber connecting, red lost. Each has
its own settings and its own controls, and a failure on one leaves the others
alone. `+ Add charger` reopens discovery.

## Building on macOS

gpui generates bindings against the system SDK, and on a machine with only the
Command Line Tools installed clang needs to be told where it is:

```
export SDKROOT="$(xcrun --show-sdk-path)"
export BINDGEN_EXTRA_CLANG_ARGS="-isysroot $SDKROOT"
cargo build -p gui
```

## How it is put together

gpui owns the main thread and runs its own executor; the Bluetooth backend
wants tokio. Rather than marrying the two, the charger lives on its own thread
with its own runtime, and the halves exchange messages: the window sends
commands, the session sends updates. Neither touches the other's state.

The session reconnects on its own, so the window never has to.

`ISDT_GUI_TRACE=1` prints every update to stderr, which is how to check the
charger half without reading pixels.
