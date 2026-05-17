# firkin-cli

`firkin-cli` provides the `fk` development CLI for exercising the Rust
containerization library from a shell.

Current commands:

- `fk pull IMAGE` resolves and pulls an OCI image bundle into the local cache.
- `fk run IMAGE [-- COMMAND...]` runs an image through the implicit VM path.
- `fk clear` reports Firkin-owned runtime storage and legacy TMPDIR artifacts;
  add `--yes` to delete them and `--include-caches` to include rebuildable
  caches.
- `fk debug preflight` checks host prerequisites that the Rust runtime depends on.
- `fk benchmark catalog` prints the stable agent-sandbox metric catalog,
  including the required P0 dashboard metrics, units, metric groups, and
  requirement levels.
- `fk benchmark autoscale-contract` prints the product/autoscale scorecard
  metrics, ownership rows, and current measurement coverage. This is the
  prereq board for the `browser + database + CLI` agent-computer path.
- `fk benchmark coverage` prints live measurement coverage for the required P0
  scorecard metrics. This intentionally distinguishes exact signed-live
  measurements from proxy measurements and metrics that still need live
  harnesses.
- `fk benchmark suites [SUITE]` prints first-class suite definitions. The
  default list covers `agent-core`, `startup`, `disk`, `memory`, `cpu`,
  `pressure`, `network`, `pod`, `agent-control`, `cleanup`, `isolation`,
  `cache`, `density`, `power`, `abuse`, `agent-realism`, `agent-computer`,
  and `autoscale`.
- `fk benchmark write-scorecard SAMPLES_JSON ARTIFACT_JSON` validates raw
  `BenchmarkSample` JSON and writes an agent scorecard artifact with
  p50/p90/p95/p99/max summaries for every required P0 metric. Use
  `--min-samples N` to require multiple samples per metric.
- `fk benchmark write-autoscale-scorecard SAMPLES_JSON ARTIFACT_JSON` validates
  raw `BenchmarkSample` JSON and writes the autoscale efficiency scorecard for
  the browser/database/CLI product path.
- `fk benchmark write-agent-computer-scorecard SAMPLES_JSON ARTIFACT_JSON`
  validates raw `BenchmarkSample` JSON and writes the five-metric product-path
  scorecard without requiring autoscale pressure/controller rows.
- `fk benchmark validate-scorecard ARTIFACT_JSON` verifies a saved scorecard
  artifact still satisfies the P0 metric shape and sample-count rules. Add
  `--require-snappy` to fail when a trusted row misses the snappy target board.
- `fk benchmark validate-autoscale-scorecard ARTIFACT_JSON` verifies a saved
  autoscale scorecard artifact still satisfies autoscale metric shape and
  sample-count rules. Add `--require-promotable` to fail when any dashboard row
  still has promotion blockers such as `unit_validated_only` coverage. Add
  `--require-snappy` to fail when a promotable-looking artifact still misses
  the autoscale snappy targets.
- `fk benchmark validate-agent-computer-scorecard ARTIFACT_JSON` verifies a
  saved product-path scorecard artifact still satisfies metric shape and
  sample-count rules. Add `--require-promotable` to fail when proxy or
  non-final product-path measurements remain. Add `--require-snappy` to fail
  when real browser + database + CLI evidence misses the product-path snappy
  targets.
- `fk benchmark validate-soak ARTIFACT_JSON` verifies production soak evidence,
  including the referenced benchmark artifact and cleanup evidence.
- `fk benchmark report-scorecard ARTIFACT_JSON` prints the dashboard summary
  lines for an already validated scorecard artifact.
- `fk benchmark report-autoscale-scorecard ARTIFACT_JSON` prints the dashboard
  summary lines for an already validated autoscale scorecard artifact.
- `fk benchmark report-agent-computer-scorecard ARTIFACT_JSON` prints the
  dashboard summary lines for an already validated agent-computer scorecard
  artifact.
- `fk benchmark report-agent-computer-traces ARTIFACT_JSON` prints phase and
  derived product-path metric summaries from either a raw trace sidecar or a
  product-pod proof artifact with a `traces` array.
- `fk benchmark report lifecycle ARTIFACT_JSON` prints p50/p90/p95/p99/max
  summaries from a lifecycle benchmark evidence artifact.
- `fk benchmark report decision ARTIFACT_JSON` prints the same artifact with
  min/mean/MAD/CV plus sample-confidence tiers such as `superfast_iteration`,
  `fast_iteration`, `baseline_checkpoint`, `p95_decision_grade`, and
  `p99_decision_grade`.
- `fk substrate latency-targets` prints the initial production-substrate latency
  and overhead manifest. Latency covers lifecycle paths such as warm snapshot
  restore and first stdout byte; overhead is tracked separately so VM/container
  resource use is not confused with Firkin's control-plane tax.
- `fk substrate acceptance-checklist` prints stable production-substrate
  acceptance IDs with current evidence status. This is the command-line audit
  surface for the goal in `docs/specs/rust_rewrite/07-production-substrate-goal.md`.
- `fk e2b host` serves the host-backed local E2B control plane and domain
  proxy, persisting state under `$FIRKIN_STATE_DIR/e2b` or
  `~/.firkin/state/e2b` by default. It preflights the SDK-visible
  `{port}-{sandboxID}.domain` hostname
  against the bound proxy listener and prints `E2B_API_URL`, `E2B_DOMAIN`, and
  sandbox connection exports. Without TLS, the local development path exports
  `E2B_SANDBOX_URL` as a direct proxy override. `--proxy-tls-cert` plus
  `--proxy-tls-key` serves the sandbox proxy over HTTPS from PEM material and
  prints `E2B_CA_CERT_FILE` plus `E2B_SANDBOX_RESOLVE_ADDR`, preserving the
  SDK's generated `https://{port}-{sandboxID}.domain` URL while resolving that
  host to the local proxy listener. `--api-key` or `FIRKIN_E2B_API_KEY` enables
  local SDK `x-api-key` enforcement on the control plane.

Default storage is deliberately outside `TMPDIR`: durable runtime artifacts use
`~/.firkin/state`, and rebuildable caches use `~/.firkin/cache`. Library callers
can pass explicit roots through `firkin_runtime::FirkinStorageConfig`; operators
can relocate the defaults with `FIRKIN_STATE_DIR` and `FIRKIN_CACHE_DIR`.

Live Apple/VZ tests are ignored by ordinary `cargo test` because the test binary
must be signed with `signing/vz.entitlements` before it can use
Virtualization.framework. Use `just live-apple-vz-benchmark-suite` for the
signed representative benchmark path, or
`scripts/run-signed-live-runtime-test.sh` for one exact ignored live test.
Use `scripts/run-firkin-decision-baseline.sh` for the release-mode baseline
workflow: it builds `fk`, runs the signed-live doctor, captures an `agent-core`
artifact, saves a named baseline, and writes lifecycle plus decision reports
next to the JSON. Set `FIRKIN_BASELINE_NO_BUILD=1` only when the signed live
test binary for that suite already exists and the run should reuse it instead
of rebuilding the test harness. To inspect the run shape before building or
launching signed-live tests, run:

```bash
FIRKIN_BASELINE_PREFLIGHT_ONLY=1 scripts/run-firkin-decision-baseline.sh agent-core
```

The preflight prints sample tier, duration, repeat counts, output paths,
product/autoscale proof settings, and density levels without mutating state.
Use `scripts/run-firkin-ramdisk-decision-baseline.sh` for storage-isolated
attribution. It copies `FIRKIN_STATE_DIR` to an APFS RAM disk, co-locates state,
benchmark roots, and evidence there, runs the normal release baseline workflow,
then copies the evidence back under `target/firkin-live-evidence/ramdisk`.
Before allocating a RAM disk or building release, run:

```bash
FIRKIN_RAMDISK_PREFLIGHT_ONLY=1 scripts/run-firkin-ramdisk-decision-baseline.sh agent-core
```

The preflight prints the requested size, current state size, benchmark disk
floor, extra live working-set headroom, recommended size, and whether the
requested RAM disk is obviously too small. The default representative shape
needs more than 32GiB on the current state; use the preflight before bumping
`FIRKIN_RAMDISK_SIZE_GIB`.
For the `agent-computer` suite, the same script also writes a product-pod
artifact manifest (`<baseline>.product-pod-artifacts.txt`) and an HTML proof
page (`<baseline>.product-pod-ready-deck-proof.html`) next to the baseline JSON
so the ready-deck JSON, trace summaries, repeat count, and rerun/report commands
are inspectable from one place. Open those first when checking an agent-computer
baseline. By default it also writes a sibling autoscale scorecard artifact plus
report, structural validation, and promotability output;
set `FIRKIN_BASELINE_AUTOSCALE_PROOF=0` to skip that slower signed-live proof,
or `FIRKIN_BASELINE_AUTOSCALE_REPEATS=<n>` to change its sample count.
`FIRKIN_BASELINE_SAMPLE_TIER` sets the default run duration and repeat count:
`superfast_iteration` is `3`, `fast_iteration` is `5`,
`baseline_checkpoint` is `10`, `p50_p90_decision_grade` is `30`, and
`p95_decision_grade` is `100`.
Explicit duration/repeat env vars still win. Set
`FIRKIN_BASELINE_AGENT_COMPUTER_MIN_SAMPLES=<n>` or
`FIRKIN_BASELINE_AUTOSCALE_MIN_SAMPLES=<n>` when validation should require a
different sample floor. The script writes both structural validation and
`--require-promotable` output for agent-computer and autoscale scorecards.
Density sweeps are also explicit inputs:
`FIRKIN_BASELINE_SHELL_DENSITY_LEVELS` defaults to `1,2`, while
`FIRKIN_BASELINE_PRODUCT_POD_READY_DECK_DENSITY_LEVELS` and
`FIRKIN_BASELINE_PRODUCT_POD_PRESTARTED_AGENT_SLOT_DENSITY_LEVELS` default to
`1,2,4`. Use `1,2,4,8` when checking the snappy density target and
`1,2,4,8,16,24,32` when finding the product-density knee instead of the cheaper
smoke shape.

| Env var | Metric | Boundary | Snappy target |
| --- | --- | --- | ---: |
| `FIRKIN_BASELINE_SHELL_DENSITY_LEVELS` | `density.max_active_before_retained_shell_first_stdout_p95_doubles` | retained-shell dispatch to first stdout | `>=8` |
| `FIRKIN_BASELINE_PRODUCT_POD_READY_DECK_DENSITY_LEVELS` | `density.max_agent_computers_before_ready_p95_doubles` | full browser + database + CLI product computer add/start to ready | `>=8` |
| `FIRKIN_BASELINE_PRODUCT_POD_PRESTARTED_AGENT_SLOT_DENSITY_LEVELS` | `density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles` | already-running slot checkout acceptance, excluding container add/start | `>=8` |

Use `docs/specs/firkin-dummy-fast-slas.md` as the optimization target board:
it defines the snappy-first public scorecard, phase budgets, density targets,
disk/cleanup guardrails, and dynamic footprint policy.

The CLI is intentionally thin. Runtime ownership stays in `firkin-core`, while
this crate handles argument parsing and user-facing command dispatch.
