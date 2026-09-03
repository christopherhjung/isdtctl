# cli

Command line control for ISDT battery chargers over Bluetooth Low Energy.

```
cargo install --path crates/cli
```

```
isdtcli scan
isdtcli -d <address> bind
isdtcli -d <address> shell
```

The binary is `isdtcli`. Everything is built on the `api` crate, which is
where to go if you want this from your own program rather than from a terminal.

See the repository README for binding, the interactive session, and the full
command list.
