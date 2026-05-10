#!/usr/bin/env python3
"""Record a Firkin command with samply and summarize the hot paths."""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def default_profile() -> Path:
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    return Path("target/profiles") / f"firkin-{stamp}.json"


def default_baseline_root() -> Path:
    return Path("target/profiles/baselines")


def sidecar_path(profile: Path, suffix: str) -> Path:
    name = profile.name
    if name.endswith(".json.gz"):
        stem = name[: -len(".json.gz")]
    else:
        stem = profile.stem
    return profile.with_name(f"{stem}-{suffix}.json")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Run samply record for a Firkin command, write a coarse stage sidecar, "
            "and summarize the recorded profile."
        )
    )
    parser.add_argument("--profile", type=Path, default=default_profile())
    parser.add_argument("--summary", type=Path)
    parser.add_argument("--stages", type=Path)
    parser.add_argument(
        "--save-baseline",
        metavar="NAME",
        help="copy summary/stage/profile paths into target/profiles/baselines/NAME-*",
    )
    parser.add_argument(
        "--baseline-root",
        type=Path,
        default=default_baseline_root(),
        help="root used by --save-baseline",
    )
    parser.add_argument("--top", type=int, default=20, help="rows to print per table")
    parser.add_argument("--repo-only", action="store_true", help="only show repo-local frames")
    parser.add_argument(
        "--group-by",
        choices=["function", "module", "crate", "file"],
        default="module",
        help="group frames for grouped tables",
    )
    parser.add_argument(
        "--module-depth",
        type=int,
        default=3,
        help="Rust path components for --group-by module",
    )
    parser.add_argument(
        "--thread",
        action="append",
        default=[],
        help="repeatable substring filter for the printed thread summary",
    )
    parser.add_argument(
        "--top-threads",
        type=int,
        default=8,
        help="thread rows to print from the profile metadata",
    )
    parser.add_argument(
        "--collapse-generics",
        action="store_true",
        help="collapse Rust generic arguments in printed and JSON function names",
    )
    parser.add_argument(
        "--no-presymbolicate",
        action="store_true",
        help="omit samply --unstable-presymbolicate",
    )
    parser.add_argument(
        "command",
        nargs=argparse.REMAINDER,
        help="command to profile, usually after --",
    )
    args = parser.parse_args()
    if args.command and args.command[0] == "--":
        args.command = args.command[1:]
    if not args.command:
        parser.error("missing command to profile; pass it after --")
    if args.top < 1:
        parser.error("--top must be at least 1")
    if args.top_threads < 0:
        parser.error("--top-threads must be non-negative")
    if args.module_depth < 1:
        parser.error("--module-depth must be at least 1")
    if args.save_baseline and not valid_baseline_name(args.save_baseline):
        parser.error("--save-baseline must use ASCII letters, numbers, - or _")
    args.summary = args.summary or sidecar_path(args.profile, "summary")
    args.stages = args.stages or sidecar_path(args.profile, "stages")
    return args


def valid_baseline_name(name: str) -> bool:
    return bool(name) and all(char.isascii() and (char.isalnum() or char in "-_") for char in name)


def command_line(args: argparse.Namespace) -> list[str]:
    command = ["samply", "record", "--save-only"]
    if not args.no_presymbolicate:
        command.append("--unstable-presymbolicate")
    command.extend(["-o", str(args.profile), "--"])
    command.extend(args.command)
    return command


def write_stage_sidecar(
    path: Path,
    *,
    profile: Path,
    summary: Path,
    command: list[str],
    samply_command: list[str],
    started_at: str,
    ended_at: str,
    elapsed_ms: float,
    returncode: int,
) -> None:
    payload = {
        "schema": "firkin.samply.stages.v1",
        "time_unit": "ms",
        "time_origin": "samply_profile_start",
        "profile": str(profile),
        "summary": str(summary),
        "command": command,
        "samply_command": samply_command,
        "stages": [
            {
                "name": "command",
                "started_at": started_at,
                "ended_at": ended_at,
                "start_ms": 0.0,
                "end_ms": elapsed_ms,
                "elapsed_ms": elapsed_ms,
                "returncode": returncode,
                "outcome": "ok" if returncode == 0 else "failed",
            }
        ],
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def run_samply_hot(args: argparse.Namespace) -> dict[str, Any]:
    script = Path(__file__).resolve().with_name("samply-hot.py")
    command = [
        sys.executable,
        str(script),
        str(args.profile),
        "--json",
        "--top",
        str(args.top),
        "--group-by",
        args.group_by,
        "--module-depth",
        str(args.module_depth),
        "--repo-root",
        str(Path.cwd()),
        "--stages",
        str(args.stages),
    ]
    if args.repo_only:
        command.append("--repo-only")
    if args.collapse_generics:
        command.append("--collapse-generics")
    for thread in args.thread:
        command.extend(["--thread", thread])
    if args.top_threads:
        command.extend(["--top-threads", str(args.top_threads)])
    result = subprocess.run(command, check=False, text=True, capture_output=True)
    if result.returncode != 0:
        if result.stdout:
            sys.stdout.write(result.stdout)
        if result.stderr:
            sys.stderr.write(result.stderr)
        raise RuntimeError(f"samply-hot.py failed with exit code {result.returncode}")
    return json.loads(result.stdout)

def print_rows(title: str, rows: list[dict[str, Any]], limit: int) -> None:
    print(title)
    print("-" * len(title))
    if not rows:
        print("(no matching samples)")
        print()
        return
    for row in rows[:limit]:
        location = ""
        if row.get("file") and row.get("line"):
            location = f"  {row['file']}:{row['line']}"
        elif row.get("file"):
            location = f"  {row['file']}"
        print(f"{row['weight']:>8.1f} {row['percent']:>6.2f}%  {row['function']}{location}")
    print()


def print_thread_rows(threads: list[dict[str, Any]], filters: list[str], limit: int) -> None:
    if limit == 0:
        return
    lowered_filters = [item.lower() for item in filters]
    filtered = [
        thread
        for thread in threads
        if not lowered_filters
        or any(item in str(thread.get("name", "")).lower() for item in lowered_filters)
    ]
    filtered.sort(key=lambda thread: float(thread.get("weight") or 0.0), reverse=True)
    title = "Top threads"
    print(title)
    print("-" * len(title))
    if not filtered:
        print("(no matching threads)")
        print()
        return
    for thread in filtered[:limit]:
        name = thread.get("name") or "<unnamed>"
        pid = thread.get("pid")
        tid = thread.get("tid")
        samples = thread.get("samples")
        weight = float(thread.get("weight") or 0.0)
        print(f"{weight:>8.1f} samples={samples} pid={pid} tid={tid}  {name}")
    print()


def print_human_summary(args: argparse.Namespace, summary: dict[str, Any], syms: str | None) -> None:
    meta = summary.get("meta", {})
    print(f"profile: {args.profile}")
    print(f"summary: {args.summary}")
    print(f"stages: {args.stages}")
    print(f"symbols: {syms or '(none)'}")
    print(
        f"samples: {meta.get('total_samples', 0)}  "
        f"weight: {float(meta.get('total_weight') or 0.0):.1f}  "
        f"interval_ms: {meta.get('interval_ms')}"
    )
    print()
    print_thread_rows(meta.get("threads", []), args.thread, args.top_threads)
    print_rows(
        f"Top grouped leaf frames ({args.group_by})",
        summary.get("grouped_leaf", []),
        args.top,
    )
    print_rows(
        f"Top grouped inclusive frames ({args.group_by})",
        summary.get("grouped_inclusive", []),
        args.top,
    )
    print_rows("Top leaf frames", summary.get("leaf", []), args.top)
    print_rows("Top inclusive frames", summary.get("inclusive", []), args.top)
    for stage in summary.get("stages", []):
        print_rows(
            f"Stage {stage['name']} grouped inclusive ({args.group_by})",
            stage.get("grouped_inclusive", []),
            args.top,
        )
        print_rows(
            f"Stage {stage['name']} inclusive frames",
            stage.get("inclusive", []),
            args.top,
        )


def save_baseline(args: argparse.Namespace) -> dict[str, str] | None:
    if not args.save_baseline:
        return None
    args.baseline_root.mkdir(parents=True, exist_ok=True)
    prefix = args.baseline_root / args.save_baseline
    paths = {
        "summary": prefix.with_name(f"{prefix.name}-summary.json"),
        "stages": prefix.with_name(f"{prefix.name}-stages.json"),
        "profile": prefix.with_name(f"{prefix.name}-profile.json"),
    }
    shutil.copy2(args.summary, paths["summary"])
    shutil.copy2(args.stages, paths["stages"])
    if args.profile.exists():
        shutil.copy2(args.profile, paths["profile"])
    return {key: str(path) for key, path in paths.items()}


def main() -> int:
    args = parse_args()
    args.profile.parent.mkdir(parents=True, exist_ok=True)
    samply_command = command_line(args)

    started_at = utc_now()
    start = time.monotonic()
    result = subprocess.run(samply_command, check=False)
    elapsed_ms = (time.monotonic() - start) * 1000.0
    ended_at = utc_now()

    write_stage_sidecar(
        args.stages,
        profile=args.profile,
        summary=args.summary,
        command=args.command,
        samply_command=samply_command,
        started_at=started_at,
        ended_at=ended_at,
        elapsed_ms=elapsed_ms,
        returncode=result.returncode,
    )

    if not args.profile.exists():
        print(f"error: profile was not written: {args.profile}", file=sys.stderr)
        return result.returncode or 1

    try:
        summary = run_samply_hot(args)
    except RuntimeError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    summary["firkin_profile_wrapper"] = {
        "schema": "firkin.profile.wrapper.v1",
        "stages": str(args.stages),
        "command_returncode": result.returncode,
        "repo_only": args.repo_only,
        "group_by": args.group_by,
        "module_depth": args.module_depth,
        "thread_filters": args.thread,
        "top_threads": args.top_threads,
        "collapse_generics": args.collapse_generics,
        "presymbolicate": not args.no_presymbolicate,
    }
    args.summary.parent.mkdir(parents=True, exist_ok=True)
    args.summary.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    baseline_paths = save_baseline(args)
    print_human_summary(args, summary, summary.get("input", {}).get("syms"))
    if baseline_paths:
        print("Saved profiling baseline")
        print("------------------------")
        for key, path in baseline_paths.items():
            print(f"{key}: {path}")
        print()
    return result.returncode


if __name__ == "__main__":
    sys.exit(main())
