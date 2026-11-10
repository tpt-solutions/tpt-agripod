# ESP32 leaf-node firmware

Status: **not started.** Blocked on the Espressif toolchain (`espup`) and on hardware.

This directory will hold a standalone `no_std` crate built with `esp-hal`. It is excluded from the edge Cargo workspace (see [ADR 0001](../docs/adr/0001-monorepo.md)) because it needs a different target and toolchain.

Planned contents (`todo.md`, Phase 2, "ESP32 leaf tier"):

- Modbus RTU slave over RS-485, with the same frame parser as `edge/crates/tpt-agripod-actuators`
- Moisture ADC input, solenoid and pump outputs
- Hardware watchdog: all valves fail closed when the master heartbeat is lost
- Compiled-in flood guard: maximum valve-open time, changed only by reflashing ([ADR 0004](../docs/adr/0004-esp32-leaf-tier.md))

The Phase 0 skeleton item is not ticked. A crate that does not build here would be a false tick.
