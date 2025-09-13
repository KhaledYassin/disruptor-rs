### Goal

- Elevate MPMC throughput to match or exceed `crossbeam-channel` under fair, comparable conditions.
- Keep correctness, simplicity, and configurability (waiting strategies) intact.

### Key Findings (from code + benches)

- "MPMC disruptor" bench is a broadcast fan-out: each consumer processes every event. In the disruptor builder, `with_barrier(|b| for _ in 0..CONSUMER_COUNT { b.handle_events_mut(…); })` wires N identical consumers behind the same barrier, all gated by the sequencer `cursor`. That is a fan-out graph (each event handled N times), not work-sharing.
- Crossbeam MPMC bench measures a load-balanced queue: each batch is received exactly once by a single consumer.
- Disruptor consumers perform extra per-item assertions (`assert_eq!(*event, sequence)`), while the crossbeam bench uses `black_box(batch)` without per-item checks. This skews results significantly against the disruptor.
- `MultiProducerSequencer` is fundamentally sound (generation-based `AvailableSequenceBuffer`, CAS cursor advance with bounded retries/backoff, wrap-point pressure against the min gating sequence). Remaining costs are mostly contention (CAS on `cursor`), scanning for contiguity, and repeated min-sequence reductions over all consumer sequences.
- Producer waiting under wrap pressure is pure spin/yield; consumers are strategy-driven. There’s no producer-side strategy to block/park under sustained backpressure (may be fine for max-throughput, but we should be explicit per benchmark).

### What “fair apples-to-apples” looks like

- Compare crossbeam’s MPMC work-sharing against a disruptor "worker pool" topology where N workers split the stream; each event is processed exactly once by exactly one worker.
- Remove per-item assertions from hot paths in benchmarks; replace with `black_box` or guard them behind a feature flag.
- Keep broadcast fan-out as a separate benchmark group (because it’s an important disruptor use case), but don’t compare it to crossbeam MPMC directly.

### Plan of Record

1. Benchmark corrections and structure

- Add two disruptor MPMC groups:
  - mpmc_worker_pool: load-balanced processing (each event processed once).
  - mpmc_fanout: broadcast (each event processed N times) for visibility.
- Unify work done by consumers:
  - Replace per-item `assert_eq!` with `black_box` in throughput benches; keep correctness asserts in unit/integration tests.
  - Add a cargo feature `bench_check` to optionally retain asserts for sanity runs.
- Align batch semantics:
  - For disruptor worker pool, process contiguous sequences in batches (same as crossbeam sends Vec batches) and `black_box` the slice window.
- Keep existing SPSC groups, but also remove per-item asserts there for throughput comparability.

2. Implement a proper Worker Pool (exactly-once consumption)

- Introduce a `work` module with a `WorkProcessor<W, D, T>` that claims the next sequence to process using a shared `work_cursor: AtomicSequence`:
  - Core loop (per worker):
    - `let next = work_cursor.increment_and_get();`
    - `barrier.wait_for(next)` to ensure publishers have advanced `cursor` ≥ `next`.
    - Read `event` at `next`, invoke handler, then set this worker’s sequence to `next`.
  - Producer gating remains the min over all worker sequences; once all workers have advanced beyond a slot, producer wrap may overwrite.
- Builder additions:
  - `.with_worker_pool(|pool| { pool.handle_work(handler_factory()); … })` or `.with_balanced_consumers(n, handler_factory)` to spawn N workers in the same pool.
  - Ensure workers’ sequences are added as gating sequences of the sequencer (already done for processors), so capacity pressure is based on processed progress.
- Safety & visibility:
  - Keep Acquire/Release semantics identical to current processors; visibility relies on `publish` Release + consumer Acquire.

3. MultiProducerSequencer hot-path refinements

- `next(n)` wrap wait:
  - Maintain spin-then-yield policy (good for peak throughput). Add cfg-gate to optionally escalate to short sleeps for power efficiency test cases.
  - Micro-optimize `Utils::get_minimum_sequence` (tight for-loops, avoid iterator overhead; potential small unroll for ≤8 consumers).
- `publish(low, high)` cursor advance:
  - Keep the fast-path check `is_available(cursor+1)` to avoid unnecessary scans.
  - Retain bounded CAS attempts with short exponential backoff (present). Tweak constants via `const` for tuning without logic churn.
  - Inline `highest_published_sequence` loop and use unchecked indexing in the tight loop (already mostly done) to reduce bound checks.
- Consider an optional "progress hint" (non-atomic) that caches recent contiguous end to reduce rescans under heavy contention; only if profiling shows it helps.

4. Processor throughput polish

- Skip `barrier.signal()` for BusySpin strategy (it’s a no-op); keep calls but #[inline(always)] to ensure it disappears for BusySpin.
- Add #[inline(always)] to hot methods (`get`, `get_mut`, `wait_for`, min-reduction, publish paths) where safe.
- Optionally add a "batched handler" trait variant that receives a range `[from..=to]` to reduce per-item call overhead in benchmarks; retain current per-item trait for API stability.

5. Instrumentation and profiling hooks

- Add a `metrics` feature to collect:
  - CAS attempts/successes on `cursor` advance
  - Producer wrap waits (iterations, yields)
  - Min-reduction counts
  - Highest-contiguous scan length
- Use `criterion` custom measurements to report these alongside throughput.

6. Validation, tests, and regression protection

- Unit tests for worker-pool exactly-once semantics under concurrency (multiple workers, out-of-order publish, wrap-around).
- Stress tests: high contention (many producers, small buffer), ensuring no deadlocks and cursor monotonicity.
- Preserve existing broadcast tests; add worker-pool test variants.

7. Rollout & PRs (incremental)

- PR1: Bench fixes (remove asserts under default bench, add fanout vs worker-pool groups, no functional changes).
- PR2: Worker pool API + implementation (minimal tune, correctness first).
- PR3: Micro-optimizations (min-reduction, inlining, small constants, optional producer waiting cfg).
- PR4: Metrics feature + detailed Criterion output.
- PR5+: Targeted tuning based on profiles (scan limits, backoff constants, optional progress hint).

### Expected outcomes

- With apples-to-apples benchmarking (work-sharing + no per-item asserts), disruptor worker pool should be competitive with `crossbeam-channel` for large batches and BusySpin on dedicated cores.
- Fan-out case will remain slower per event than work-sharing by design, but it’s a distinct use case and should be benchmarked separately.

### Notes and open questions

- Thread pinning and affinity (platform dependent) may affect peak results; consider optional pinning in benches behind a feature flag.
- Prefetching: if we decide to use a prefetch helper crate, gate it behind a feature and verify portability.
- Do not relax memory orderings without a written proof of correctness; the current Acquire/Release scheme is conservative and safe.

### Minimal code touch map (for implementation phase)

- New: `src/work.rs` (WorkProcessor, WorkerPool types)
- Update: `src/builder.rs` (builder path for worker pools)
- Update: `benches/throughput.rs` (bench groups + fairness)
- Update: `src/utils.rs` (fast min reduction)
- Optional: `src/sequencer.rs` (tiny tunables, inlining), `src/waiting.rs` (cfg’d producer waiting policy)
