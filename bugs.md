# Known Bugs

Tracked after the state-machine refactor (LoadState / SelectionState / ActiveDialog).

---

## BUG-1 — Mount error silently dropped when udisks2 event fires during dialog confirm

**File**: `src/handlers.rs` — `ConfirmMountDialog` handler (~line 125)  
**Severity**: High  

**Root cause**: `ConfirmMountDialog` fires the async D-Bus mount task unconditionally, but
only sets `ctx.operation_in_progress` when `self.selection_state` is
`SelectionState::Partition`. The udisks2 background subscription
(`src/actions/udisks_watch.rs`) emits `Message::RefreshDisks` on any disk event
(USB plug, PropertiesChanged, etc.) with a 500 ms debounce. If `RefreshDisks` is
processed after `ConfirmMountDialog` has already launched the async task, it resets
`selection_state = SelectionState::None`. When the mount subsequently fails,
`OperationFailed` hits the `SelectionState::None => {}` arm and the error is silently
discarded.

**Scenario**: User confirms mount dialog → mount task starts, spinner shown in
PartitionContext → unrelated USB event fires RefreshDisks → selection_state = None,
drives refreshed → mount fails → OperationFailed dropped → user sees no error message,
drive still not mounted.

**Fix**: Decouple `operation_in_progress` / `operation_error` from `SelectionState`
so they survive a refresh, or gate the async task launch on the selection state being
correct at the time of confirm.

---

## BUG-2 — Unmount error silently dropped when udisks2 event fires during dialog confirm

**File**: `src/handlers.rs` — `ConfirmUnmount` handler (~line 162)  
**Severity**: High  

**Root cause**: Same structural defect as BUG-1 in the unmount path. `ConfirmUnmount`
fires the D-Bus unmount task regardless of `selection_state`, but the spinner and error
routing are gated on `SelectionState::Partition`. A background `RefreshDisks` between
dialog open and confirm (or between confirm and async result) causes the same silent
error loss.

**Scenario**: User confirms unmount dialog → udisks2 PropertiesChanged fires RefreshDisks
→ selection_state = None → unmount fails → OperationFailed dropped → device remains
mounted with no explanation.

**Fix**: Same as BUG-1.

---

## BUG-3 — OperationFailed silently discards errors when selection_state is None

**File**: `src/handlers.rs` — `OperationFailed` handler (~line 374)  
**Severity**: Medium  

**Root cause**: The old code wrote `self.operation_error = Some(e)` unconditionally on
`AppModel`. The new code dispatches on `selection_state` and has a `SelectionState::None
=> {}` no-op arm. Any async D-Bus error that arrives after the selection has been
cleared (by RefreshDisks or by the user clicking away) is immediately dropped with no
recovery path.

**Old behaviour**: Error survived in `AppModel` until the next `clear_selection_state()`
call (i.e. until the user clicked a partition), so it was technically recoverable even
if rarely visible.

**New behaviour**: Error is gone forever once it hits the `None` arm.

**Fix**: Maintain a top-level `Option<String>` error field on `AppModel` for orphaned
operation errors, or display them as a toast/banner independent of selection state.

---

## CLEANUP-1 — Dead else-branch in view_error and view_drives

**File**: `src/app.rs` (~lines 283, 307)  
**Severity**: Low  

`view_error()` and `view_drives()` each begin with:
```rust
let LoadState::X = self.load_state else { return self.view_scanning() };
```
The `else` branch is dead code — `view()` already dispatches each `LoadState` variant
to exactly one helper and never calls these methods in an unexpected state.

Replace the guard with `unreachable!()` to document the invariant and catch future
misuse during development, or remove the guard entirely.

---

## CLEANUP-2 — OperationFailed and DismissError contain duplicate match blocks

**File**: `src/handlers.rs` (~lines 365–381)  
**Severity**: Low  

Both handlers contain an identical three-arm `match &mut self.selection_state` block
differing only in the mutation applied. Adding a new `SelectionState` variant would
require updating both independently, risking asymmetric error handling.

Extract a helper method (e.g. `fn set_selection_error(&mut self, e: Option<String>)`)
to consolidate both match expressions.
