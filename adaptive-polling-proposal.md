<!-- SPDX-License-Identifier: MIT
  Copyright (c) 2026 WinTapNetAdapterCx contributors -->
# Adaptive Polling and Driver Notification Proposal

## Goal

Reduce per-packet scheduler wakeups and software IPIs while allowing user mode
to control the latency and CPU cost of polling.

## Driver behavior

User mode enables this mode with an IOCTL.

- READ requests always complete immediately:
  - with one available captured frame; or
  - with `STATUS_NO_MORE_ENTRIES` when no frame is available.
- WRITE requests always complete immediately:
  - successfully when the frame is accepted into the injection queue; or
  - with `STATUS_DEVICE_BUSY` when the queue is full.
- A `WAIT_FOR_CHANGE` IOCTL may be pended until either:
  - the capture queue changes from empty to nonempty; or
  - the injection queue changes from full to nonfull.

The wait request includes an interest mask so user mode can wait for readable
data, writable space, or both. Its completion returns the conditions that are
currently satisfied.

## Race-free waiting

Registering the wait and checking the queue conditions must be atomic with
respect to queue transitions.

Under the appropriate queue lock, the driver:

1. Checks the requested conditions.
2. Completes the IOCTL immediately if a condition is already satisfied.
3. Otherwise registers the request as a cancellable pending wait.

Queue transitions claim pending wait requests while holding the lock, but
complete the requests after releasing it. Generation counters may be returned
with each completion to make state changes explicit and aid diagnostics.

This level-sensitive check prevents a notification from being lost when the
queue changes immediately before user mode submits the wait.

## User-mode behavior

User mode controls the adaptive polling interval:

1. Submit batches of READ and WRITE operations without requesting an I/O-ring
   completion wait.
2. Drain completions and continue processing while progress is being made.
3. Poll for up to a configurable number of microseconds after progress stops.
4. Submit `WAIT_FOR_CHANGE` and block only after the polling interval expires.
5. Resume polling when the wait completes.

The polling interval can adapt to recent traffic. Sustained traffic favors
polling for low latency, while idle periods quickly transition to a blocking
wait to avoid wasting CPU.

## Expected performance effect

During active polling, I/O-ring submissions use a minimum completion count of
zero, so individual packet completions do not wake a waiting thread.

When idle, completion of `WAIT_FOR_CHANGE` may still cause `KeSetEvent` and a
software IPI, but this occurs once per idle-to-active transition rather than
once per packet. This should substantially reduce the time attributed to
`HalpInterruptSendIpi` in packet completion paths.

Batching remains important because repeatedly creating and completing one empty
READ request at a time can become expensive even without scheduler wakeups.

## Compatibility

The new behavior is enabled per control handle through an IOCTL. Existing
pending READ and WRITE behavior remains available for clients that do not
request adaptive polling mode.
