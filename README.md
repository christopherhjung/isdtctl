# isdt-charger

Talk to ISDT battery chargers over Bluetooth Low Energy. Reimplements the
protocol from ISDT's own Android application and exposes every command and
query that application can send, including several its CM1620 screens never
reach. Verified against a CM1620.

`PROTOCOL.md` documents the wire format, and says where the Android app leaves
something undefined.

## Layout

Two crates, so the backend is usable without the terminal.

| Crate | What it is |
|---|---|
| [`isdt-charger`](crates/isdt-charger) | The backend: protocol, Bluetooth link, and a client generic over the transport |
| [`isdtctl`](crates/isdtctl) | The command line tool, built on that backend |

Nothing in the tool has privileged access to the backend. Anything `isdtctl`
does, your own program can do, and `crates/isdt-charger/examples/custom_link.rs`
shows the client running over a transport the crate knows nothing about.

## Install

```
cargo build --release
```

The binary lands at `target/release/isdtctl`.

On macOS the program needs Bluetooth permission. Grant it to your terminal
under System Settings, Privacy and Security, Bluetooth. Without it the system
stays silent rather than refusing, so run `cargo run --example probe` to tell a
permission problem apart from a charger that is not advertising.

## Binding comes first

A charger disconnects any client that has not bound, about five seconds after
connecting, so every command needs a client identifier. There is nothing to
look up: you invent one, the charger stores it, and it expects the same one
back on every later connection.

Put the charger into binding mode, then:

```
isdtctl scan                      # the BINDING column shows "waiting"
isdtctl -d <address> bind         # generates, binds, and saves the identifier
isdtctl tokens                    # what this host has stored
```

Identifiers are kept in `~/.config/isdtctl/tokens`. **Keep that file.** A
charger cannot tell you its identifier back, so losing it means putting the
charger into binding mode and binding again. Pass `--client-id` to use one from
somewhere else, such as the phone app's database.

If the charger was already bound by the Android app, take the identifier from
`/sdcard/Android/data/com.isdt.hubin.isdtapp/databases/isdt.db`, table
`devicedatatable`, column `uuid`.

## Interactive session

Binding and connecting cost a few seconds each time, so for anything
exploratory open a session instead:

```
isdtctl -d <address> shell
```

It connects once, binds once, and holds the link open. Every command works
inside it with the same syntax, with history and line editing. Add `--json` to
any read command for that line only. `exit`, `quit` or Ctrl-D leaves; Ctrl-C
just clears the current line. A failed command reports and returns you to the
prompt rather than ending the session.

```
CM1620 Der Neue> status
CM1620 Der Neue> charge --battery lipo --cells 4 --current-ma 1000
CM1620 Der Neue> stop
```

If the session sits idle it polls the charger every twenty seconds, so a lost
link is reported rather than discovered the next time you type.

## Use

```
isdtctl scan
isdtctl info
isdtctl status
isdtctl watch --interval 200
```

Start and stop tasks:

```
isdtctl charge --battery lipo --cells 4 --current-ma 2000
isdtctl storage --battery lipo --cells 4 --current-ma 1000
isdtctl discharge --battery lipo --cells 4 --current-ma 1000
isdtctl stop
```

Target voltage defaults to the chemistry's own value for the task. Override it
with `--volt-mv`. The tool applies the same range checks the app does; `--force`
skips them.

Requests are resent when a charger does not answer, because a charger swallows
the first control frame after a bind and drops the occasional packet besides.
The firmware operations and the reboot are sent exactly once instead.

Settings:

```
isdtctl limits
isdtctl limits --min-volt 12 --power 600
isdtctl onekey
isdtctl onekey --set --enabled --battery lipo --cells 4 --current-ma 2000
isdtctl name "Bench charger"
```

Smart batteries and power supplies:

```
isdtctl battgo state
isdtctl battgo write --current-ma 2000 --store-mv 3800 --full-mv 4200 --rest-days 3
isdtctl smartpower info
```

Anything the tool does not model:

```
isdtctl raw "e4 00"
```

`--json` gives machine-readable output for every read command.

## Library

```rust
use std::time::Duration;
use isdt_charger::{BatteryKind, Client, LinkType, TaskType};

let mut client = Client::discover(None, Duration::from_secs(10)).await?;
let state = client.work_state(0).await?;
println!("{} at {}%", state.state.label(), state.capacity_percent);

client.start_task(
    0, TaskType::Charge, BatteryKind::LiPo, LinkType::SerialOnly,
    2000, 4, 4200,
).await?;
```

The layers are separable. `frame` does framing and byte stuffing, `request` and
`response` do packet encoding and decoding with no input or output of their
own, `transport` does Bluetooth, and `Client` ties them together. If you have a
different link to the charger, the first three are usable on their own.

## Scope and risk

`SetTask` starts real current through a real battery. `Calibrate6`,
`Calibrate8`, `EraseApp` and `WriteApp` change persistent device state, and an
interrupted firmware write can leave a charger unbootable. Nothing here
second-guesses a request beyond the range checks noted above.

The protocol was read from a decompiled application, not from a vendor
specification. It has not been tested against physical hardware in this
repository; the test suite checks the encoders against the checksum literals
baked into the app, which pins the frame layouts but cannot confirm how a given
firmware behaves.

Speed controller packets (command words `0x51` to `0x73`) are out of scope.
Those devices are not chargers.
