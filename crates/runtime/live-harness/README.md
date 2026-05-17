# Runtime Live Harness

This directory keeps signed Apple/VZ and E2B SDK compatibility coverage that is
not part of the default crates.io package test graph yet.

The harness is intentionally outside `crates/runtime/tests` because it currently
depends on local signing, runtime artifacts, and E2B SDK compatibility surfaces
that are not publishable as normal crate dev-dependencies. Do not hide these
files behind `cfg(any())`; either move them into a dedicated harness crate with
real dependencies or leave them here as explicit scaffolding.
