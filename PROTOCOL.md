# ISDT charger BLE protocol

Extracted from the ISDT Android app (`com.isdt.hubin.isdtapp`, version 1.3.8,
`versionCode` 89) in `app2/`. Everything below is read out of that app's
`com.isdt.hubin.isdtapp.ble` package and the CM1620 activity classes. Where the
app leaves something undefined, this document says so rather than guessing.

The other decompiled application in `app/` is the Aptoide store client. It has
no Bluetooth code at all and no CM1620 reference.

## GATT layer

The charger advertises service `0000FFF0-0000-1000-8000-00805F9B34FB` and
exposes three characteristics under it.

| Characteristic | Role |
|---|---|
| `0000FFF6` | notify, and write on the 20 byte channel |
| `0000FFF7` | write once the MTU is negotiated past 140 bytes |
| `0000FFF8` | write, used for the app's version handshake |

Notifications always arrive on FFF6, with descriptor `00002902` enabled.
The app requests an MTU of 143 or 163 and falls back to 20 when the charger
refuses. Writes go to FFF6 at 20 bytes, otherwise to FFF7.

Every GATT write and every notification is length prefixed:

```
[n] [payload; n]
```

On the 20 byte channel a frame longer than 16 bytes is split across writes of
up to 19 payload bytes each. The receiver concatenates payloads and runs the
frame parser over the stream.

## Frame layer

```
AA  ADDR  LEN  DATA[LEN]  CHK
```

* `AA` is a single sync byte, not stuffed, opening the frame.
* `ADDR` is `0x12` host to charger and `0x21` charger to host.
* `LEN` counts `DATA` only. `DATA[0]` is the command word.
* `CHK` is `(ADDR + LEN + sum(DATA)) & 0xFF`.
* Every byte after the opening sync byte is stuffed: a literal `0xAA` goes out
  as `AA AA`.

The receiver's state machine skips odd-numbered sync bytes, so a lone `0xAA`
followed by a non-sync byte resynchronises to the address field, and a doubled
`0xAA` decodes to one data byte.

All multi-byte integers are little endian.

## Binding, and the five second rule

Observed on a CM1620, firmware as shipped in 2026:

**A charger disconnects any client that has not bound, about five seconds after
the link comes up.** Before that it answers `0x00` and ignores everything else.
This is not visible anywhere in the app, because the app binds within its first
second, but it is the single thing that stops an unbound client working.

Binding is one frame. The host invents a random 16 byte token and sends it as
`0x18`. The charger stores it and thereafter expects the same token from
whoever connects, on **every** connection, not just the first. The reply is one
status byte: `0x00` accepted, anything else refused. A refusal observed in
practice is `0xFF`.

There is no key exchange. The charger never sends the token back, so a lost
token can only be replaced by putting the charger into binding mode and binding
again.

A charger advertises whether it is waiting to be bound, in its name:

```text
ISDT 1 CM1620␣␣ <NAME>
^^^^ ^ ^^^^^^^^ ^^^^^^^^
tag  | model    owner's name
     1 = waiting to be bound, 0 = already bound
```

Some units wrap this inside another string, for example
`Phy BLE-Uart [ISDT0CM1620  Der Neue]`, so locate the `ISDT` tag rather than
assuming it starts at offset zero.

## Requests, host to charger

`LEN` is the payload length including the command word.

| Command | LEN | Payload after the command word | Reply |
|---|---|---|---|
| `0x00` identify | 1 | none | `0x01`, and `0x13` on capable units |
| `0x18` bind | 17 | 16 byte client identifier | `0x19` |
| `0x40` BattGo info | 2 | channel | `0x41` |
| `0x42` BattGo OEM profile | 2 | channel | `0x43` |
| `0x44` BattGo live state | 2 | channel | `0x45` |
| `0x46` BattGo write settings | 11 | channel, current u32, store mV u16, full mV u16, rest days u8 | `0x47` |
| `0x48` BattGo read settings | 2 | channel | `0x49` |
| `0xBA` smart power info | 1 | none | `0xBB` |
| `0xBC` smart power write | 6 | `C3 A5` guard, setting u8, value u16 | `0xBD` |
| `0xBE` smart power settings | 1 | none | `0xBF` |
| `0xC0` set name | 17 | 16 byte name, zero padded | `0xC1` |
| `0xD0` set limits | 7 | min input mV u16, max input power mW u32 | `0xD1` |
| `0xD2` set one-key launch | 10 | enabled u8, chemistry u8, cells u8, full mV u16, current mA u32 | `0xD3` |
| `0xD4` read one-key launch | 1 | none | `0xD5` |
| `0xDC` calibrate 8 cells | 23 | channel, mode, 8 × mV u16, input mV u16, output mV u16 | `0xDD` |
| `0xDE` calibrate 6 cells | 19 | channel, mode, 6 × mV u16, input mV u16, output mV u16 | `0xDF` |
| `0xE0` hardware info | 1 | none | `0xE1` |
| `0xE2` read limits | 1 | none | `0xE3` |
| `0xE4` electrical | 2 | channel | `0xE5` |
| `0xE6` work state | 2 | channel | `0xE7` |
| `0xE8` temperature | 2 | channel | `0xE9` |
| `0xEA` set task | 12 | channel, task, chemistry, link, current mA u32, cells u8, full mV u16 | `0xEB` |
| `0xF0` enter bootloader | 2 | `AC` guard | `0xF1` |
| `0xF2` erase firmware | 10 | cpu u8, address u32, size u32 | `0xF3` |
| `0xF4` write firmware | 134 | cpu u8, address u32, 128 data bytes | `0xF5` |
| `0xF6` verify firmware | 15 | `35` sub-command, cpu u8, address u32, size u32, checksum u32 | `0xF7` |
| `0xFA` internal resistance | 2 | channel | `0xFB` |
| `0xFC` reboot | 2 | `CA` guard | `0xFD` |

Every reply command word is the request's plus one, with the sole exception of
`0x00`.

The app's dispatch table also recognises command words `0x13`, `0x51` through
`0x73`, and `0xB1`. Those in the `0x51` to `0x73` range belong to ISDT's
electronic speed controllers and are not charger traffic. `0x13` collides:
the app routes it to a speed controller name packet even though a charger
Bluetooth-mode packet carries the same word.

## Responses, charger to host

### `0xE5` electrical

Two widths, chosen by frame length. Frames of 35 bytes or more carry 32 bit
voltages and sixteen cells; shorter frames carry 16 bit voltages and eight.

| Field | Width | Unit |
|---|---|---|
| channel | u8 | |
| input voltage | u16 or u32 | mV |
| input current | u32 | mA |
| output voltage | u16 or u32 | mV |
| charge or discharge current | u32 | mA |
| cell voltages | 8 or 16 × u16 | mV |

The app computes input power as `mV × mA / 1e6` watts.

### `0xE7` work state

| Field | Width | Unit |
|---|---|---|
| channel | u8 | |
| state | u8 | see the state table |
| progress | u8 | percent, the app clamps its gauge to 100 |
| capacity delivered | u32 | mAh |
| energy delivered | u32 | mWh |
| elapsed | u32 | **milliseconds** |
| chemistry | u8 | see the chemistry table |
| cells in series | u8 | |
| link type | u8 | see the link table |
| target voltage | u16 | mV per cell |
| task current | u32 | mA |
| batteries in job | u16 | count |
| batteries done | u16 | count |
| input cutoff | u16 | mV |
| output power ceiling | u32 | mW |
| fault mask | u16 | bit field, absent on older firmware |

### `0xE9` temperature

Channel, charger temperature as a **signed** byte in degrees Celsius, probe
temperature as a signed byte, then a fan byte the app parses and never
displays. The fan byte's scale is therefore not established by the app.

### `0xFB` internal resistance

Channel then eight `u16` readings in **tenths of a milliohm**. The app divides
by ten before display and treats anything above 6553 milliohms as no reading.

### `0xE3` limits

Four `u32` values: input power ceiling in mW, output power ceiling in mW,
then two current values in mA. Observed on a CM1620: 1050000, 1000000, 1000,
20000. The app feeds the third into the *lower* bound of its current picker and
the fourth into the upper, so despite the field name the third is a floor, not
a ceiling.

### `0xE1` hardware info

Eight byte device identifier, then hardware, bootloader and firmware versions
as four bytes each, then an optional ten byte display name and an optional
eight byte part number. Older firmware ends the frame after the versions.

### `0x01` identify

The app reads this as a region byte followed by an eight byte part number
(`IsdtPackBleTest`). A CM1620 answers 13 bytes, which do not fit that reading.
Observed across three connections, with only the marked bytes changing:

```text
01  04  00  e8 03 00 00  70 a9 38 e4 c2 84
    ^^  ^^  ^^^^^^^^^^^  ^^^^^^^^^^^^^^^^^
    |   |   1000, 2000, 5000 across runs    six bytes, stable, look like
    |   |   milliseconds since connect      the device address
    |   second byte, always zero
    first byte, always 4
```

The counter is the reason this frame is useful: it says how long the charger
has been waiting for the bind that has not arrived, and the link drops shortly
after it reaches 5000. The layout matches `IsdtPackBleMode`
(mode, pair mode, 32 bit period) plus six trailing bytes, rather than
`IsdtPackBleTest`. This library keeps the raw payload so nothing is lost.

### Acknowledgements

`0xC1`, `0xD1`, `0xD3`, `0xF1` carry one status byte. `0xDD`, `0xDF`, `0x47`,
`0xF3`, `0xF7` carry a channel or CPU index and a status byte. `0xF5` carries
a CPU index, the address written and a status byte. Zero means success
throughout. `0xEB` carries a channel and a task error code, where zero means
the charger accepted the task; the app has no table for the other values and
prints them as a bare number.

### `0xBB`, `0xBF` smart power

A CM1620 never sends these. Voltages are in tenths of a volt, since the
station screen divides by ten. The currents are never displayed by the app, so
their unit is not established. The `warningFlags` and `errorFlags` bytes have
getters with no callers anywhere in the app, so their bits have no known
meaning.

## Enumerations

### Task type

`0` charge, `1` storage, `2` discharge, `3` stop.

### Chemistry

The index into the app's chemistry picker list. Both directions of the task and
work-state packets use this scale.

| Code | Chemistry | Full mV | Store mV | Cutoff mV | Max cells |
|---|---|---|---|---|---|
| 0 | LiHv | 4350 | 3850 | 3400 | 6 |
| 1 | LiPo | 4200 | 3800 | 3300 | 6 |
| 2 | LiIon | 4100 | 3700 | 3200 | 6 |
| 3 | LiFe | 3650 | 3300 | 2900 | 6 |
| 4 | Pb | 2400 | none | 1800 | 12 |
| 5 | NiMH/Cd | none | none | 900 | none |
| 6 | ULiHv | 4450 | none | none | 6 |

The CM1620 charge sheet only offers LiHv, LiPo, LiFe and ULiHv, so codes 2, 4
and 5 arrive only from the charger. For code 5 the app sends the target field
as a `-Δ` millivolt delta rather than an absolute voltage.

The app carries a second, unrelated chemistry table
(`auto, LiHv, LiIon, LiFe, NiZn, none, NiMH/Cd`). No CM1620 code path uses it.

### Link type

`0` nothing connected, `1` main leads only, `2` balance connector only,
`3` both. The charger reports all four and the app gates its start button on
them. The app only ever *sends* `1`, on every chemistry.

### Charger state

The twelve cases of the CM1620 state switch, with the app's own wording.

| Code | Meaning | App label |
|---|---|---|
| 0 | idle | Standby |
| 1 | waking a flat pack | Activating |
| 2 | current ramping | Charging |
| 3 | constant current | Charging |
| 4 | constant voltage | Charging |
| 5 | balancing at target | Fast charge completed |
| 6 | trickle balancing | Charge completed |
| 7 | charging to storage | Charging |
| 8 | discharging to storage | Storing |
| 9 | storage reached | Stored |
| 10 | discharging | Discharging |
| 11 | discharge finished | Discharged |

ISDT's cell-slot chargers use a different eleven-entry table
(`no battery, battery exist, battery reversed, charging, charged, discharging,
discharged, storing, stored, cycling, cycled`). A CM1620 does not.

### Fault mask

The 16 bit field is a mask, not an enumeration. The app sets one string per
set bit and joins them.

| Bit | Meaning |
|---|---|
| 0 | output overcurrent |
| 1 | output overvoltage |
| 2 | input overvoltage |
| 3 | input undervoltage |
| 4 | input unstable voltage |
| 5 | temperature anomaly |
| 6 | charging timeout |
| 7 | linking state destroyed |
| 8 | battery cell overvoltage |
| 9 | battery reversed |
| 10 | balance charging unsupported |
| 11 | battery node linking error |
| 12 | output no battery |
| 13, 14, 15 | unnamed |

Discharge stations, speed controllers and the P30 use different arrays for the
same field. The table above is the charger set the CM1620 uses.

### Calibration mode

`0` store the supplied reference voltages, `255` restore the factory constants.

## Polling

The app's CM1620 screen rotates six requests, one per 150 millisecond tick, so
a full pass takes about 900 milliseconds:

1. electrical, channel 0
2. temperature, channel 0
3. work state, channel 0
4. limits
5. internal resistance, channel 0
6. internal resistance, channel 1

User-initiated writes jump the queue. One command runs once per screen entry
and then removes itself: identify, which reads the part number. Hardware info
is sent once by the connection state machine, not by the poller.

## Ranges the app enforces

These are user interface limits, not protocol limits. The charger may accept
more.

| Setting | Range | Step |
|---|---|---|
| work current | 100 to 5000 mA | 500 mA on the wheel |
| cells in series | 2 to 16 | 1 |
| target voltage | chemistry nominal minus 600 mV to plus 50 mV | 10 mV |
| input power ceiling | 100 to 1100 W | 50 W |
| input undervoltage cutoff | 11 to 70 V | 1 V |
| device name | 1 to 16 bytes | |

## Observed on hardware, not in the app

* The five second unbound disconnect described above.
* A refused bind answers status `0xFF`.
* Writing to `0000FFF7` with response never completes on a CM1620, even though
  the characteristic advertises the write property. Use `0000FFF6`.
* **A charger answers almost nothing unless notifications are enabled on
  `0000FFF7` as well as `0000FFF6`.** Every reply then arrives on FFF6 and FFF7
  never fires, but without that second subscription a bound CM1620 replies only
  to `0x00`. The app enables both, which is why this is invisible in its code.
* **A charger silently swallows the first control frame after a bind.** A task
  set sent straight after the bind reply draws nothing at all, not even a
  rejection; the byte-for-byte identical frame is acknowledged on the next
  attempt. Chunk size, pacing and payload make no difference. The app hides
  this because it keeps a user command at the head of its queue and resends it
  every tick until acknowledged, so resending is the protocol's normal
  behaviour rather than a workaround.
* A charger drops the odd packet otherwise too, so treat one silence as a lost
  frame rather than a failure.
* These units are Bluetooth-to-serial bridges, and the app never sends two
  packets closer together than 150 ms.
* Once a charger drops the link, CoreBluetooth leaves every later call pending
  forever rather than failing it, including `is_connected`. Bound every
  operation with a timeout.

## Things the app does not define

* Task error codes other than zero. They are printed as a bare number.
* The bits of `warningFlags` and `errorFlags` in the smart power frame.
* Smart power setting types other than `1`, which is output voltage in tenths
  of a volt. No other value appears anywhere in the app.
* What link type values other than `1` mean as an outbound setting.
* The scale of the fan byte in the temperature frame.
* What the two values in each line-chart sample pair represent.
