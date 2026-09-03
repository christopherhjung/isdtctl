# api

Talk to ISDT battery chargers from Rust: the wire protocol, a Bluetooth Low
Energy backend, and a client that is generic over the transport.

Verified against a CM1620; other ISDT chargers share the command set. See
`PROTOCOL.md` at the repository root for the wire format, and the repository
README for the device list.

```toml
[dependencies]
api = "0.1"
```

## Bluetooth

```rust
use std::time::Duration;
use api::{Client, tokens};

let client_id = tokens::parse("9102782c5bfb5047a4533d071feb6eca")?;
let mut charger =
    Client::discover_bound(None, Duration::from_secs(10), Default::default(), client_id).await?;

let state = charger.work_state(0).await?;
println!("{} at {}%", state.state.label(), state.capacity_percent);
```

Binding is not optional. A charger disconnects any client that has not bound,
about five seconds in. The identifier is one you invent and then keep; the
charger cannot tell it back to you.

## Another transport

`Client` is generic over the `Link` trait and knows nothing about Bluetooth.
Implement `Link` for a serial port, a TCP bridge or a test double and every
typed command works over it. See `examples/custom_link.rs`.

Turn the `ble` feature off to drop the Bluetooth stack entirely and keep the
protocol and the trait:

```toml
api = { version = "0.1", default-features = false }
```

## Just the bytes

The packet layer does no I/O and needs no runtime, so it is usable on its own.

```rust
use api::{frame, Request, Response};

let bytes = Request::WorkState { channel: 0 }.encode().unwrap();
assert_eq!(bytes, [0xAA, 0x12, 0x02, 0xE6, 0x00, 0xFA]);

let mut decoder = frame::Decoder::new();
for data in decoder.push_notification(&[0x05, 0xAA, 0x21, 0x02, 0xEB, 0x00, 0x0E]) {
    println!("{:?}", Response::parse(&data));
}
```

## Care

`start_task` puts current through a battery. The calibration and firmware calls
change persistent device state, and an interrupted firmware write can leave a
charger unbootable. Nothing here second-guesses a request; the bounds the
vendor application enforces are exposed as constants but not applied.
