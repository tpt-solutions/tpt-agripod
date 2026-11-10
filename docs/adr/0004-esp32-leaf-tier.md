# 0004: ESP32 as a leaf tier behind the Rust edge node

Status: Accepted
Date: 2026-11-11

## Context

Field wiring needs local I/O (moisture ADC, solenoid and pump outputs) close to the sensors. A leaf microcontroller can fail independently of the edge PC, and it must fail safe on its own.

## Decision

The ESP32 runs `no_std` firmware (`esp-hal`) as a Modbus RTU slave on RS-485. It has a hardware watchdog that closes all valves when the master heartbeat is lost, and a hard-coded maximum valve-open time as an independent flood guard. The edge node remains the only place where contracts are evaluated.

## Consequences

- Two safety layers exist: the solver on the edge and the guard on the leaf. The guard's limits are compiled in and changed only by reflashing.
- Firmware needs an Espressif toolchain (`espup`). CI needs an ESP32 target build job.
- OTA updates are out of scope for v1. The update procedure is a documented manual flash.
