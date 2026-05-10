# Firkin Benchmark Artifacts

`docs/artifacts/` is for benchmark proof and sprint records that are useful to
inspect from the repo.

Markdown is the durable operator format. Commit `docs/artifacts/*.md` records
when they capture a current sprint, proof summary, command transcript, residual
risk list, or next benchmark command.

Generated HTML proof pages are local preview artifacts. `docs/artifacts/*.html`
is ignored so regenerated proof pages do not churn the working copy. Keep HTML
only when a task specifically asks for an HTML proof page; otherwise write the
operator record as markdown.

Live benchmark evidence JSON stays under `target/` or another generated output
root and is not committed here.
