#!/usr/bin/env python3
"""Summarize samply Firefox Profiler JSON into understandable Rust hot paths."""

from __future__ import annotations

import argparse
import gzip
import json
import re
import sys
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass(frozen=True)
class Frame:
    function: str
    file: str | None
    line: int | None
    lib: str | None
    address: int | None

    def normalized(self, *, collapse_generics: bool) -> "Frame":
        return Frame(
            normalize_rust_function(self.function, collapse_generics=collapse_generics),
            self.file,
            self.line,
            self.lib,
            self.address,
        )

    def display_name(self) -> str:
        if self.file and self.line:
            return f"{self.function} ({self.file}:{self.line})"
        if self.file:
            return f"{self.function} ({self.file})"
        return self.function

    def key(
        self, *, collapse_generics: bool = False
    ) -> tuple[str, str | None, int | None, str | None]:
        return (
            normalize_rust_function(self.function, collapse_generics=collapse_generics),
            self.file,
            self.line,
            self.lib,
        )

    def group_key(
        self,
        group_by: str,
        *,
        module_depth: int,
        repo_root: Path,
        collapse_generics: bool,
    ) -> tuple[str, str | None, int | None, str | None]:
        if group_by == "function":
            return self.key(collapse_generics=collapse_generics)
        function = normalize_rust_function(
            self.function, collapse_generics=collapse_generics
        )
        if group_by == "crate":
            return (crate_name(function), None, None, self.lib)
        if group_by == "module":
            return (module_name(function, module_depth), None, None, self.lib)
        if group_by == "file":
            return (file_name(self.file, repo_root), self.file, None, self.lib)
        raise ValueError(f"unsupported group_by: {group_by}")


@dataclass
class Symbol:
    function: str
    file: str | None
    line: int | None
    rva: int
    size: int


@dataclass(frozen=True)
class Stage:
    name: str
    start_ms: float
    end_ms: float

    def contains(self, sample_time_ms: float | None) -> bool:
        if sample_time_ms is None:
            return False
        return self.start_ms <= sample_time_ms <= self.end_ms


def load_json(path: Path) -> Any:
    if path.suffix == ".gz":
        with gzip.open(path, "rt", encoding="utf-8") as handle:
            return json.load(handle)
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def infer_syms_path(profile_path: Path) -> Path | None:
    candidates = []
    name = profile_path.name
    if name.endswith(".json.gz"):
        candidates.append(profile_path.with_name(name[: -len(".json.gz")] + ".syms.json"))
    if name.endswith(".json"):
        candidates.append(profile_path.with_name(name[: -len(".json")] + ".syms.json"))
    candidates.append(profile_path.with_suffix(".syms.json"))
    for candidate in candidates:
        if candidate.exists():
            return candidate
    return None


def build_symbol_maps(syms: Any | None) -> dict[str, list[Symbol]]:
    if not syms:
        return {}
    strings = syms.get("string_table", [])
    by_debug_name: dict[str, list[Symbol]] = {}
    for obj in syms.get("data", []):
        debug_name = obj.get("debug_name")
        if not debug_name:
            continue
        symbols = []
        for symbol in obj.get("symbol_table", []):
            function = string_at(strings, symbol.get("symbol"))
            file = None
            line = None
            frames = symbol.get("frames") or []
            if frames:
                # The last inline frame is the most source-local Rust frame.
                frame = frames[-1]
                function = string_at(strings, frame.get("function")) or function
                file = string_at(strings, frame.get("file"))
                line = frame.get("line")
            if function:
                symbols.append(
                    Symbol(
                        function=function,
                        file=file,
                        line=line,
                        rva=int(symbol.get("rva", 0)),
                        size=int(symbol.get("size", 0)),
                    )
                )
        symbols.sort(key=lambda item: item.rva)
        by_debug_name[debug_name] = symbols
    return by_debug_name


def string_at(strings: list[str], index: Any) -> str | None:
    if index is None:
        return None
    try:
        return strings[int(index)]
    except (IndexError, TypeError, ValueError):
        return None


def table_at(table: dict[str, Any], column: str, index: int) -> Any:
    values = table.get(column, [])
    if index < 0 or index >= len(values):
        return None
    return values[index]


def resolve_symbol(symbols: list[Symbol], address: int | None) -> Symbol | None:
    if address is None:
        return None
    # Samply sidecars include exact known_addresses for frames, but range lookup
    # makes the parser useful across slightly different profile encodings.
    lo = 0
    hi = len(symbols)
    while lo < hi:
        mid = (lo + hi) // 2
        if symbols[mid].rva <= address:
            lo = mid + 1
        else:
            hi = mid
    for candidate in reversed(symbols[max(0, lo - 3) : lo + 1]):
        size = candidate.size or 1
        if candidate.rva <= address < candidate.rva + size:
            return candidate
    return None


class ProfileResolver:
    def __init__(self, profile: Any, syms: Any | None):
        self.profile = profile
        self.symbols = build_symbol_maps(syms)
        self.libs = profile.get("libs", [])

    def lib_name(self, thread: dict[str, Any], func_index: int | None) -> str | None:
        if func_index is None:
            return None
        func_table = thread.get("funcTable", {})
        resource_index = table_at(func_table, "resource", func_index)
        if resource_index is None:
            return None
        resource_table = thread.get("resourceTable", {})
        lib_index = table_at(resource_table, "lib", int(resource_index))
        if lib_index is None:
            return None
        try:
            lib = self.libs[int(lib_index)]
        except (IndexError, TypeError, ValueError):
            return None
        return lib.get("debugName") or lib.get("name")

    def fallback_function(self, thread: dict[str, Any], frame_index: int) -> str:
        frame_table = thread.get("frameTable", {})
        func_index = table_at(frame_table, "func", frame_index)
        if func_index is None:
            return "<unknown>"
        func_table = thread.get("funcTable", {})
        name_index = table_at(func_table, "name", int(func_index))
        return string_at(thread.get("stringArray", []), name_index) or "<unknown>"

    def frame(self, thread: dict[str, Any], frame_index: int) -> Frame:
        frame_table = thread.get("frameTable", {})
        address = table_at(frame_table, "address", frame_index)
        address = int(address) if address is not None else None
        func_index = table_at(frame_table, "func", frame_index)
        func_index = int(func_index) if func_index is not None else None
        lib_name = self.lib_name(thread, func_index)
        symbol = resolve_symbol(self.symbols.get(lib_name or "", []), address)
        if symbol:
            return Frame(symbol.function, symbol.file, symbol.line, lib_name, address)
        return Frame(self.fallback_function(thread, frame_index), None, None, lib_name, address)

    def stack_frames(self, thread: dict[str, Any], stack_index: int | None) -> list[Frame]:
        if stack_index is None:
            return []
        stack_table = thread.get("stackTable", {})
        frames = []
        seen = set()
        current = int(stack_index)
        while current is not None:
            if current in seen:
                break
            seen.add(current)
            frame_index = table_at(stack_table, "frame", current)
            if frame_index is None:
                break
            frames.append(self.frame(thread, int(frame_index)))
            prefix = table_at(stack_table, "prefix", current)
            current = int(prefix) if prefix is not None else None
        return frames


def is_repo_frame(frame: Frame, repo_root: Path) -> bool:
    if frame.file:
        try:
            Path(frame.file).resolve().relative_to(repo_root)
            return True
        except (OSError, ValueError):
            pass
    return frame.function.startswith(("firkin_", "firkin::", "e2b_adapter::", "fk::"))


def is_noise(frame: Frame) -> bool:
    prefixes = (
        "start",
        "_pthread_start",
        "thread_start",
        "std::rt::",
        "std::sys::backtrace::",
        "test::",
    )
    return frame.function.startswith(prefixes)


def crate_name(function: str) -> str:
    cleaned = strip_rust_wrappers(function)
    parts = cleaned.split("::")
    if not parts:
        return "<unknown>"
    first = parts[0]
    if first.startswith("<") and len(parts) > 1:
        return parts[1]
    return first or "<unknown>"


def module_name(function: str, depth: int) -> str:
    cleaned = strip_rust_wrappers(function)
    parts = [part for part in cleaned.split("::") if part and not part.startswith("{{")]
    if not parts:
        return "<unknown>"
    if parts[0].startswith("<") and len(parts) > 1:
        parts = parts[1:]
    depth = max(1, depth)
    return "::".join(parts[:depth])


def file_name(file: str | None, repo_root: Path) -> str:
    if not file:
        return "<unknown-file>"
    path = Path(file)
    try:
        return str(path.resolve().relative_to(repo_root))
    except (OSError, ValueError):
        return file


def strip_rust_wrappers(function: str) -> str:
    # Keep trait impls recognizable, but strip common closure/hash suffix noise.
    function = re.sub(r"::h[0-9a-f]{16}$", "", function)
    function = function.replace("::{{closure}}", "")
    if function.startswith("<") and " as " in function:
        impl = function[1:].split(" as ", 1)[0]
        receiver = impl.split(" as ", 1)[0]
        if "::" in receiver:
            return re.sub(r"<[^>]*>", "", receiver)
    return function


def normalize_rust_function(function: str, *, collapse_generics: bool) -> str:
    function = strip_rust_wrappers(function)
    if collapse_generics:
        function = remove_angle_groups(function)
        function = re.sub(r"\[[^\]]+\]", "[]", function)
        function = function.replace("::{{closure}}", "")
    return function


def remove_angle_groups(text: str) -> str:
    output = []
    depth = 0
    for char in text:
        if char == "<":
            depth += 1
            continue
        if char == ">" and depth:
            depth -= 1
            continue
        if depth == 0:
            output.append(char)
    return "".join(output)


def thread_label(thread: dict[str, Any]) -> str:
    name = thread.get("name") or thread.get("processName") or "<unnamed>"
    return f"{name} pid={thread.get('pid')} tid={thread.get('tid')}"


def thread_matches(thread: dict[str, Any], filters: list[re.Pattern[str]]) -> bool:
    if not filters:
        return True
    label = thread_label(thread)
    return any(pattern.search(label) for pattern in filters)


def load_stages(path: Path | None) -> list[Stage]:
    if not path:
        return []
    data = load_json(path)
    raw_stages = data.get("stages", data if isinstance(data, list) else [])
    stages = []
    for item in raw_stages:
        try:
            name = str(item["name"])
            start_ms = float(item.get("start_ms", item.get("start")))
            end_ms = float(item.get("end_ms", item.get("end")))
        except (KeyError, TypeError, ValueError):
            continue
        if end_ms < start_ms:
            continue
        stages.append(Stage(name=name, start_ms=start_ms, end_ms=end_ms))
    return stages


def should_include(
    frame: Frame,
    *,
    include: re.Pattern[str] | None,
    exclude: re.Pattern[str] | None,
    repo_only: bool,
    repo_root: Path,
    show_noise: bool,
) -> bool:
    text = frame.display_name()
    if not show_noise and is_noise(frame):
        return False
    if repo_only and not is_repo_frame(frame, repo_root):
        return False
    if include and not include.search(text):
        return False
    if exclude and exclude.search(text):
        return False
    return True


def summarize(
    profile: Any,
    syms: Any | None,
    *,
    repo_root: Path,
    include: re.Pattern[str] | None,
    exclude: re.Pattern[str] | None,
    repo_only: bool,
    show_noise: bool,
    group_by: str,
    module_depth: int,
    collapse_generics: bool,
    thread_filters: list[re.Pattern[str]],
    stages: list[Stage],
) -> dict[str, Any]:
    resolver = ProfileResolver(profile, syms)
    leaf = Counter()
    inclusive = Counter()
    grouped_leaf = Counter()
    grouped_inclusive = Counter()
    stage_counters: dict[str, dict[str, Counter]] = {
        stage.name: {
            "leaf": Counter(),
            "inclusive": Counter(),
            "grouped_leaf": Counter(),
            "grouped_inclusive": Counter(),
        }
        for stage in stages
    }
    stage_totals = Counter()
    total_samples = 0
    total_weight = 0.0
    thread_summaries = []
    for thread in profile.get("threads", []):
        if not thread_matches(thread, thread_filters):
            continue
        samples = thread.get("samples", {})
        stacks = samples.get("stack", [])
        weights = samples.get("weight") or [1] * len(stacks)
        times = samples.get("time") or [None] * len(stacks)
        thread_weight = 0.0
        for stack_index, weight, sample_time in zip(stacks, weights, times):
            weight = float(weight or 1.0)
            total_samples += 1
            total_weight += weight
            thread_weight += weight
            frames = resolver.stack_frames(thread, stack_index)
            filtered = [
                frame
                for frame in frames
                if should_include(
                    frame,
                    include=include,
                    exclude=exclude,
                    repo_only=repo_only,
                    repo_root=repo_root,
                    show_noise=show_noise,
                )
            ]
            if filtered:
                # stackTable row points at the leaf and prefix walks toward root.
                leaf[filtered[0].key(collapse_generics=collapse_generics)] += weight
                grouped_leaf[
                    filtered[0].group_key(
                        group_by,
                        module_depth=module_depth,
                        repo_root=repo_root,
                        collapse_generics=collapse_generics,
                    )
                ] += weight
                for key in {
                    frame.key(collapse_generics=collapse_generics) for frame in filtered
                }:
                    inclusive[key] += weight
                for key in {
                    frame.group_key(
                        group_by,
                        module_depth=module_depth,
                        repo_root=repo_root,
                        collapse_generics=collapse_generics,
                    )
                    for frame in filtered
                }:
                    grouped_inclusive[key] += weight
                for stage in stages:
                    if not stage.contains(float(sample_time) if sample_time is not None else None):
                        continue
                    counters = stage_counters[stage.name]
                    stage_totals[stage.name] += weight
                    counters["leaf"][
                        filtered[0].key(collapse_generics=collapse_generics)
                    ] += weight
                    counters["grouped_leaf"][
                        filtered[0].group_key(
                            group_by,
                            module_depth=module_depth,
                            repo_root=repo_root,
                            collapse_generics=collapse_generics,
                        )
                    ] += weight
                    for key in {
                        frame.key(collapse_generics=collapse_generics)
                        for frame in filtered
                    }:
                        counters["inclusive"][key] += weight
                    for key in {
                        frame.group_key(
                            group_by,
                            module_depth=module_depth,
                            repo_root=repo_root,
                            collapse_generics=collapse_generics,
                        )
                        for frame in filtered
                    }:
                        counters["grouped_inclusive"][key] += weight
        thread_summaries.append(
            {
                "name": thread.get("name") or thread.get("processName") or "<unnamed>",
                "pid": thread.get("pid"),
                "tid": thread.get("tid"),
                "samples": len(stacks),
                "weight": thread_weight,
            }
        )

    stage_rows = []
    for stage in stages:
        counters = stage_counters[stage.name]
        stage_weight = float(stage_totals[stage.name])
        stage_rows.append(
            {
                "name": stage.name,
                "start_ms": stage.start_ms,
                "end_ms": stage.end_ms,
                "duration_ms": stage.end_ms - stage.start_ms,
                "weight": stage_weight,
                "leaf": rows_from_counter(counters["leaf"], stage_weight),
                "inclusive": rows_from_counter(counters["inclusive"], stage_weight),
                "grouped_leaf": rows_from_counter(counters["grouped_leaf"], stage_weight),
                "grouped_inclusive": rows_from_counter(
                    counters["grouped_inclusive"], stage_weight
                ),
            }
        )

    return {
        "meta": {
            "product": profile.get("meta", {}).get("product"),
            "interval_ms": profile.get("meta", {}).get("interval"),
            "symbolicated": profile.get("meta", {}).get("symbolicated"),
            "total_samples": total_samples,
            "total_weight": total_weight,
            "threads": thread_summaries,
            "thread_filters": [pattern.pattern for pattern in thread_filters],
            "collapse_generics": collapse_generics,
        },
        "leaf": rows_from_counter(leaf, total_weight),
        "inclusive": rows_from_counter(inclusive, total_weight),
        "grouped_leaf": rows_from_counter(grouped_leaf, total_weight),
        "grouped_inclusive": rows_from_counter(grouped_inclusive, total_weight),
        "stages": stage_rows,
    }


def rows_from_counter(counter: Counter, total_weight: float) -> list[dict[str, Any]]:
    rows = []
    for (function, file, line, lib), weight in counter.most_common():
        pct = (weight / total_weight * 100.0) if total_weight else 0.0
        rows.append(
            {
                "weight": weight,
                "percent": pct,
                "function": function,
                "file": file,
                "line": line,
                "lib": lib,
            }
        )
    return rows


def print_table(title: str, rows: list[dict[str, Any]], limit: int) -> None:
    print(title)
    print("-" * len(title))
    if not rows:
        print("(no matching samples)")
        print()
        return
    for row in rows[:limit]:
        location = ""
        if row["file"] and row["line"]:
            location = f"  {row['file']}:{row['line']}"
        elif row["file"]:
            location = f"  {row['file']}"
        print(f"{row['weight']:>8.1f} {row['percent']:>6.2f}%  {row['function']}{location}")
    print()


def print_threads(threads: list[dict[str, Any]], limit: int) -> None:
    print("Top threads")
    print("-----------")
    for thread in sorted(threads, key=lambda item: item["weight"], reverse=True)[:limit]:
        print(
            f"{thread['weight']:>8.1f}  samples={thread['samples']:<6} "
            f"{thread['name']} pid={thread['pid']} tid={thread['tid']}"
        )
    print()


def summary_from_path(path: Path, args: argparse.Namespace) -> dict[str, Any]:
    data = load_json(path)
    if {"leaf", "inclusive", "grouped_leaf", "grouped_inclusive"}.issubset(data):
        return data
    syms_path = args.syms or infer_syms_path(path)
    syms = load_json(syms_path) if syms_path else None
    include = re.compile(args.include) if args.include else None
    exclude = re.compile(args.exclude) if args.exclude else None
    return summarize(
        data,
        syms,
        repo_root=args.repo_root.resolve(),
        include=include,
        exclude=exclude,
        repo_only=args.repo_only,
        show_noise=args.show_noise,
        group_by=args.group_by,
        module_depth=args.module_depth,
        collapse_generics=args.collapse_generics,
        thread_filters=[re.compile(item) for item in args.thread],
        stages=[],
    )


def diff_rows(
    before: list[dict[str, Any]], after: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    def row_key(row: dict[str, Any]) -> tuple[Any, ...]:
        return (row.get("function"), row.get("file"), row.get("line"), row.get("lib"))

    before_by_key = {row_key(row): row for row in before}
    after_by_key = {row_key(row): row for row in after}
    rows = []
    for key in sorted(set(before_by_key) | set(after_by_key)):
        before_row = before_by_key.get(key, {})
        after_row = after_by_key.get(key, {})
        before_weight = float(before_row.get("weight", 0.0))
        after_weight = float(after_row.get("weight", 0.0))
        before_percent = float(before_row.get("percent", 0.0))
        after_percent = float(after_row.get("percent", 0.0))
        function, file, line, lib = key
        rows.append(
            {
                "weight_before": before_weight,
                "weight_after": after_weight,
                "weight_delta": after_weight - before_weight,
                "percent_before": before_percent,
                "percent_after": after_percent,
                "percent_delta": after_percent - before_percent,
                "function": function,
                "file": file,
                "line": line,
                "lib": lib,
            }
        )
    rows.sort(key=lambda row: abs(row["weight_delta"]), reverse=True)
    return rows


def print_diff(rows: list[dict[str, Any]], limit: int) -> None:
    print("Hot path diff")
    print("-------------")
    if not rows:
        print("(no matching rows)")
        print()
        return
    for row in rows[:limit]:
        location = ""
        if row["file"] and row["line"]:
            location = f"  {row['file']}:{row['line']}"
        elif row["file"]:
            location = f"  {row['file']}"
        print(
            f"{row['weight_before']:>8.1f} -> {row['weight_after']:<8.1f} "
            f"{row['weight_delta']:>+8.1f} "
            f"{row['percent_before']:>6.2f}% -> {row['percent_after']:>6.2f}% "
            f"{row['function']}{location}"
        )
    print()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Summarize samply Firefox Profiler JSON into Rust hot paths."
    )
    parser.add_argument("profile", type=Path, nargs="?", help="samply profile JSON or JSON.GZ")
    parser.add_argument(
        "--diff",
        nargs=2,
        metavar=("BEFORE_JSON", "AFTER_JSON"),
        type=Path,
        help="compare two samply-hot summary JSON files or raw profile JSON files",
    )
    parser.add_argument(
        "--syms",
        type=Path,
        help="samply --unstable-presymbolicate sidecar; inferred by default",
    )
    parser.add_argument("--top", type=int, default=20, help="rows to print per table")
    parser.add_argument("--json", action="store_true", help="emit machine-readable JSON")
    parser.add_argument("--repo-only", action="store_true", help="only show repo-local frames")
    parser.add_argument("--show-noise", action="store_true", help="show runtime/test launcher noise")
    parser.add_argument("--include", help="regex applied to function/location")
    parser.add_argument("--exclude", help="regex applied to function/location")
    parser.add_argument(
        "--thread",
        action="append",
        default=[],
        help="regex applied to thread name/process/pid/tid; repeatable",
    )
    parser.add_argument(
        "--top-threads",
        type=int,
        default=0,
        help="print the N hottest threads before frame tables",
    )
    parser.add_argument(
        "--collapse-generics",
        action="store_true",
        help="collapse monomorphized Rust generic paths in function/module grouping",
    )
    parser.add_argument(
        "--stages",
        type=Path,
        help="stage timeline JSON with stages[].name/start_ms/end_ms in profile-relative ms",
    )
    parser.add_argument(
        "--diff-table",
        choices=["grouped_inclusive", "grouped_leaf", "inclusive", "leaf"],
        default="grouped_inclusive",
        help="summary table to compare for --diff",
    )
    parser.add_argument(
        "--group-by",
        choices=["function", "module", "crate", "file"],
        default="module",
        help="group frames for grouped tables; function preserves exact rows",
    )
    parser.add_argument(
        "--module-depth",
        type=int,
        default=3,
        help="number of Rust path components for --group-by module",
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path.cwd(),
        help="repo root for --repo-only; defaults to cwd",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.diff:
        before = summary_from_path(args.diff[0], args)
        after = summary_from_path(args.diff[1], args)
        rows = diff_rows(before[args.diff_table], after[args.diff_table])
        output = {
            "before": str(args.diff[0]),
            "after": str(args.diff[1]),
            "table": args.diff_table,
            "rows": rows,
        }
        if args.json:
            print(json.dumps(output, indent=2, sort_keys=True))
            return 0
        print(f"before: {args.diff[0]}")
        print(f"after: {args.diff[1]}")
        print(f"table: {args.diff_table}")
        print()
        print_diff(rows, args.top)
        return 0

    if not args.profile:
        raise SystemExit("profile is required unless --diff is used")
    profile = load_json(args.profile)
    syms_path = args.syms or infer_syms_path(args.profile)
    syms = load_json(syms_path) if syms_path else None
    include = re.compile(args.include) if args.include else None
    exclude = re.compile(args.exclude) if args.exclude else None
    stages = load_stages(args.stages)
    summary = summarize(
        profile,
        syms,
        repo_root=args.repo_root.resolve(),
        include=include,
        exclude=exclude,
        repo_only=args.repo_only,
        show_noise=args.show_noise,
        group_by=args.group_by,
        module_depth=args.module_depth,
        collapse_generics=args.collapse_generics,
        thread_filters=[re.compile(item) for item in args.thread],
        stages=stages,
    )
    summary["input"] = {
        "profile": str(args.profile),
        "syms": str(syms_path) if syms_path else None,
        "stages": str(args.stages) if args.stages else None,
    }
    if args.json:
        print(json.dumps(summary, indent=2, sort_keys=True))
        return 0

    meta = summary["meta"]
    print(f"profile: {args.profile}")
    print(f"symbols: {syms_path or '(none)'}")
    print(
        f"samples: {meta['total_samples']}  weight: {meta['total_weight']:.1f}  "
        f"interval_ms: {meta.get('interval_ms')}"
    )
    if not syms_path:
        print("warning: no .syms.json sidecar found; output may show raw addresses")
    print()
    if args.top_threads:
        print_threads(meta["threads"], args.top_threads)
    print_table(
        f"Top grouped leaf frames ({args.group_by})",
        summary["grouped_leaf"],
        args.top,
    )
    print_table(
        f"Top grouped inclusive frames ({args.group_by})",
        summary["grouped_inclusive"],
        args.top,
    )
    print_table("Top leaf frames", summary["leaf"], args.top)
    print_table("Top inclusive frames", summary["inclusive"], args.top)
    for stage in summary["stages"]:
        print_table(
            f"Stage {stage['name']} grouped inclusive ({args.group_by})",
            stage["grouped_inclusive"],
            args.top,
        )
        print_table(
            f"Stage {stage['name']} inclusive frames",
            stage["inclusive"],
            args.top,
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
