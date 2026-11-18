# Writing tests that catch bugs

A test earns its place only if a plausible bug would make it fail. Line
coverage does not measure that: once suite size is controlled for, coverage is
only weakly correlated with how many faults a suite detects (Inozemtseva &
Holmes, ICSE 2014). Mutation testing does measure it, and Google's study of
15 million mutants found surviving mutants are coupled to real faults that
later shipped (Petrović et al., ICSE 2021).

## The one question

Before writing or keeping a test, name the bug it catches:

> "If someone changed ___ in the code, this test would fail, and that change
> would be a real bug a user would notice."

If you cannot fill the blank with a realistic mistake, delete the test.

## Tests worth having

- **Regression tests for bugs we actually shipped.** Reproduce the report
  first, watch it fail, then fix. These have the best catch rate of anything.
- **Behaviour through the public surface.** Drive the RPC, the CLI, the state
  machine's public methods. Assert on observable outcomes (state, output,
  persisted data, messages sent), not on which private helper was called.
- **Boundaries and edge cases.** Empty, one, many; off-by-one at limits;
  unicode and char boundaries; zero/negative durations; paths with spaces;
  truncated or malformed input from another process or the network.
- **Ordering, concurrency and lifecycle.** Reconnects, cancellation mid-turn,
  shutdown while work is in flight, restart and resume, two clients racing.
  These break most often and are hardest to notice by hand.
- **Contracts between components.** Wire formats that other binaries,
  older clients or the cloud read. Test compatibility with a fixed fixture
  captured from the other side, not a round trip through our own types.
- **Invariants over many inputs.** Where an invariant is easy to state
  (parse(render(x)) == x, totals never negative, history window never exceeds
  its bound), prefer a property test or a loop over a table of hostile cases.
- **Security and data-loss paths.** Secrets never logged or sent, access
  checks deny by default, deletes and migrations do not lose user data.

## Tests to delete on sight

- **Restating the implementation.** Asserting a constant equals its literal,
  a default equals what `Default` returns, a getter returns what the setter
  stored, a `match` arm maps A to the A the code literally writes.
- **Round trips through derived code.** `serde` round trip of a plain
  `#[derive(Serialize, Deserialize)]` struct proves serde works. Keep only
  ones pinning a hand-written impl, a renamed/tagged field another program
  reads, or backwards compatibility with a captured payload.
- **Change detectors.** Tests that break on any refactor without a behaviour
  change: snapshotting exact strings of UI copy or log lines, asserting the
  exact sequence of internal calls on a mock, counting struct fields.
- **Source-text greps.** Reading `.rs`/`.tsx` files and asserting a string is
  or is not present. Lint, `clippy`, or the type system should own that.
- **Tautologies and smoke-only tests.** `assert!(true)`, constructing a value
  and asserting nothing, `is_ok()` on a function that cannot fail, asserting
  a freshly built empty list is empty.
- **Duplicates.** Five tests that drive the same branch with cosmetically
  different input. Keep the one with the most hostile input.
- **Testing the dependency.** Checking that `std`, `tokio`, `serde_json`,
  `url` or the OS behave as documented.

## Writing a good one

1. Name it after the behaviour and the condition, e.g.
   `reconnect_after_token_expiry_resends_pending_turn`.
2. Arrange the smallest real setup. Prefer real collaborators and in-memory
   fakes over mocks; mock only at process or network boundaries.
3. One behaviour per test; as many asserts as that behaviour needs.
4. Make failures readable: assert on the whole value (`assert_eq!` on the
   struct) so the diff shows what changed.
5. No sleeps for synchronisation. Wait on the event, use paused tokio time,
   or inject a clock. Sleeps cause most flaky tests and most slow ones.
6. Check it catches the bug: break the code on purpose (flip the condition,
   drop the line) and confirm the test fails. `cargo mutants -p <crate> -f
   <file>` automates this for a file you touched.

## CI budget

CI must finish in about five minutes. A test that needs more than a couple of
seconds needs a reason to be in the per-PR run; put long live or soak tests
behind `#[ignore]` and run them on main or nightly.

## Sources

- Petrović, Ivanković, Fraser, Just. *Does mutation testing improve testing
  practices?* ICSE 2021. https://arxiv.org/abs/2103.07189
- Petrović et al. *Practical Mutation Testing at Scale: A view from Google.*
  IEEE TSE. https://research.google/pubs/pub50658/
- Inozemtseva, Holmes. *Coverage is not strongly correlated with test suite
  effectiveness.* ICSE 2014. https://cs.ubc.ca/~rtholmes/papers/icse_2014_inozemtseva.pdf
- *Testing on the Toilet: Change-Detector Tests Considered Harmful.*
  https://testing.googleblog.com/2015/01/testing-on-toilet-change-detector-tests.html
- *Software Engineering at Google*, ch. 12 Unit Testing.
  https://abseil.io/resources/swe-book/html/ch12.html
- cargo-mutants. https://mutants.rs/
