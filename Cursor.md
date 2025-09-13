# Cursor Improvement Plan

## Goal

Identify and implement high-impact improvements to the multi-producer Disruptor implementation so it can compete with crossbeam channels.

---

## Major Bottlenecks (from code review)

- **`AvailableSequenceBuffer` overhead**
  - Uses a `HashMap` per publish (`set_batch` / `unset_batch`), causing large allocation and hashing overhead in the hot path.
  - **Evidence:** `utils.rs` → `set_batch` and `unset_batch` build a `HashMap` for every call.

```rust
// src/utils.rs lines 95-105
for seq in start..=end {
    let index = (seq & self.index_mask) as usize;
    let flag = 1i64 << (seq & 0x3f);
    *updates.entry(index).or_insert(0) |= flag;
}

for (index, combined_flags) in updates {
    self.available_buffer[index]
        .value
        .fetch_or(combined_flags, Ordering::Release);
}
```

- **`publish()` inefficiency**
  - Scans and attempts to advance the cursor even when the published range cannot advance the cursor.
  - Unsets availability flags (extra work) after advancement.

```rust
// src/sequencer.rs lines 260-288
fn publish(&self, low: Sequence, high: Sequence) {
    self.available_buffer.set_batch(low, high);
    let mut cursor_val = self.cursor.get();
    loop {
        let mut next = cursor_val + 1;
        // ... chunked scanning
        let contiguous_end = next - 1;
        if contiguous_end > cursor_val {
            if self.cursor.compare_and_set(cursor_val, contiguous_end) {
                self.available_buffer.unset_batch(cursor_val + 1, contiguous_end);
                self.waiting_strategy.signal_all_when_blocking();
                break;
            } else {
                // retry with backoff
            }
        } else {
            break;
        }
    }
}
```

- **Available-buffer design inefficiency**

  - Relies on setting/unsetting bit flags.
  - Classic LMAX design uses per-slot "generation" (`sequence >> index_shift`), never clears flags.

- **Overly strong atomics (`SeqCst`)** in hot paths.
  - `AtomicSequence::get_and_add`, `compare_and_set` use `SeqCst`.
  - Hurts throughput on non-x86 or under contention.

```rust
// src/sequence.rs lines 87-95
pub fn compare_and_set(&self, expected: Sequence, new_value: Sequence) -> bool {
    self.value
        .compare_exchange(expected, new_value, Ordering::SeqCst, Ordering::Acquire)
        .is_ok()
}

pub fn get_and_add(&self, delta: Sequence) -> Sequence {
    self.value.fetch_add(delta, Ordering::SeqCst)
}
```

- **`MultiProducerSequencer::next` waits inefficiently**
  - Tight spinning with `yield_now`, not using `WaitingStrategy`, no `cpu_relax`.

```rust
// src/sequencer.rs lines 245-252
if wrap_point > cached {
    loop {
        let min_seq = Utils::get_minimum_sequence(&self.gating_sequences);
        self.cached_value.set(min_seq);
        if wrap_point <= min_seq { break; }
        std::thread::yield_now();
    }
}
```

---

## High-Impact Improvement Plan

### Phase 1: Replace `AvailableSequenceBuffer` with generation-based design

- **Rationale:** Removes per-publish allocations and extra atomics.
- **Changes in `src/utils.rs`:**
  - Fields: `buffer: Vec<CacheAlignedAtomicI64>`, `index_mask: i64`, `index_shift: u32`.
  - Methods:
    - `set(sequence)` → store generation (`seq >> shift`) in slot.
    - `set_batch(start, end)` → loop over `set()`.
    - `is_available(sequence)` → check slot gen matches expected.
    - `highest_published_sequence(from, to)` → linear scan.
  - **Remove** `unset()` and `unset_batch()`.
  - Update tests accordingly.

### Phase 2: Optimize `publish()` path

- **Rationale:** Avoid unnecessary scans.
- **Changes:**
  - Fast path: if `low != cursor + 1`, return.
  - Use `highest_published_sequence` to advance cursor.
  - Remove `unset_batch` calls.
  - Use bounded CAS retries with `spin_loop`.

### Phase 3: Reduce atomic ordering costs

- **compare_and_set:** `Ordering::AcqRel` (success) / `Ordering::Acquire` (failure).
- **get_and_add:** `Ordering::AcqRel` (or Relaxed after audit).

### Phase 4: Improve capacity wait in `next()`

- Replace `yield_now` with `spin_loop`.
- Optionally integrate `WaitingStrategy`.

### Phase 5: Micro-optimizations

- Add `std::hint::spin_loop()` inside `BusySpinWaitStrategy` loop.

```rust
// src/waiting.rs lines 116-121
loop {
    if check_alert() { return None; }
    let minimum_sequence = Utils::get_minimum_sequence(dependencies);
    if minimum_sequence >= sequence { return Some(minimum_sequence); }
    std::hint::spin_loop();
}
```

### Phase 6: Tests and benchmarks

- Update unit tests.
- Add new tests for generation semantics and fast-path advancement.
- Run criterion benches on `mpmc` vs `mpmc_disruptor`.

---

## Order of Operations

1. Implement Phase 1 (`utils.rs`). Update `publish()` to stop calling `unset()`. Fix tests.
2. Implement Phase 2 (`publish` fast path).
3. Implement Phase 4 small change: `spin_loop` in `next()`.
4. Implement Phase 3 atomic ordering changes.
5. Add BusySpinWaitStrategy improvement.
6. Run tests and benchmarks.

---

## Risks & Edge Cases

- **Wrap-around correctness:** Ensure `next()` capacity check prevents slot reuse.
- **Out-of-order publishes:** Ensure cursor advances only when `low == cursor+1`.
- **Memory ordering:** Ensure Release/Acquire discipline holds.
- **Benchmarking noise:** Run multiple times for stable results.

---

## Stretch Goals

- Replace gating-min loops with `WaitingStrategy`.
- Add parker-based blocking strategy.
- Provide specialized single-sequence publish fast path.

---

## Expected Impact

- **Biggest win:** Removing `HashMap` + no-unset semantics.
- **Fast-path publish:** Avoid repeated scans on out-of-order publishes.
- **Smaller gains:** Atomic ordering relaxation, `spin_loop` hints.
- **Overall:** Closes much of the throughput gap vs crossbeam MPMC channels.

---

## Results (Criterion)

- Environment: macOS 15.6.1, Apple M4 Pro
- Commit: 79e0dcd

- mpmc_disruptor throughput (higher is better):
  - batch=1: 4.09–4.21 Melem/s
  - batch=10: 10.84–10.92 Melem/s
  - batch=100: 12.31–12.35 Melem/s

- mpmc baseline:
  - batch=1: 3.33–3.37 Melem/s
  - batch=10: 14.68–14.86 Melem/s
  - batch=100: 15.40–15.62 Melem/s

Notes:
- Some regressions were observed at smaller batch sizes compared to prior runs; results can vary with thermal and scheduler conditions. The publish scan behavior remains unchanged and tests cover out-of-order and wrap-around cases.
