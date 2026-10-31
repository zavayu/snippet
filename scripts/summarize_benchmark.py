"""Summarize Snippet's JSONL timing traces using only the Python standard library."""
import argparse
import collections
import json
import math
from pathlib import Path
import statistics


def percentile(values, fraction):
    values = sorted(values)
    index = (len(values) - 1) * fraction
    lower = math.floor(index)
    upper = math.ceil(index)
    return values[lower] + (values[upper] - values[lower]) * (index - lower)


def final_content_event(record):
    return next((event for event in reversed(record["events"])
                 if event["name"] == "answer_first_content"), None)


def summarize(paths):
    records = [json.loads(line) for path in paths for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    groups = collections.defaultdict(list)
    for record in records:
        metadata = next((e["detail"] for e in record["events"] if e["name"] == "benchmark"), {})
        case = metadata.get("case", "app")
        context = metadata.get("chat_context", metadata.get("planning_context", next((e["detail"].get("num_ctx") for e in record["events"] if e["name"] == "answer_request"), "unknown")))
        thinking = metadata.get("thinking", next((e["detail"].get("thinking") for e in record["events"] if e["name"] == "answer_request"), "unknown"))
        groups[(record["model"], case, context, thinking)].append(record)
    lines = ["# Snippet timing results", "", "All durations are milliseconds. Percentiles interpolate between samples; small runs are exploratory, not reliable tail-latency estimates.", ""]
    lines += ["| Model | Case | Context | Thinking | Completed | Median total ms | Median first answer text ms |", "|---|---|---:|---|---:|---:|---:|"]
    for (model, case, context, thinking), samples in groups.items():
        good = [sample for sample in samples if sample["status"] == "complete"]
        total = f"{statistics.median(sample['total_ms'] for sample in good):.2f}" if good else ""
        first = [event["at_ms"] for sample in good if (event := final_content_event(sample))]
        first_text = f"{statistics.median(first):.2f}" if first else ""
        lines.append(f"| {model} | {case} | {context} | {thinking} | {len(good)}/{len(samples)} | {total} | {first_text} |")
    lines.append("")
    for (model, case, context, thinking), samples in groups.items():
        successful = [sample for sample in samples if sample["status"] == "complete"]
        lines += [f"## {model}: {case} (context {context}, thinking {thinking})", "", f"Completed {len(successful)}/{len(samples)} requests. Statuses: {dict(collections.Counter(s['status'] for s in samples))}.", ""]
        metrics = collections.defaultdict(list)
        for sample in successful:
            metrics["request total"].append(sample["total_ms"])
            for event in sample["events"]:
                name, detail = event["name"], event["detail"]
                if "duration_ms" in detail:
                    metrics[name].append(detail["duration_ms"])
                if name == "answer_first_content" and event is final_content_event(sample):
                    metrics["time to first answer text (from request start)"].append(event["at_ms"])
                if name == "answer.ollama":
                    for key in ["load_ms", "prompt_eval_ms", "eval_ms", "tokens_per_second", "prompt_eval_count", "eval_count"]:
                        if key in detail:
                            metrics[f"answer {key}"].append(detail[key])
                if name in ("search_decision.ollama", "query_planning.ollama"):
                    for key in ["load_ms", "prompt_eval_ms", "eval_ms"]:
                        if key in detail:
                            metrics[f"planner {key}"].append(detail[key])
        if metrics:
            lines += ["| Metric | Median | p95 | Min | Max |", "|---|---:|---:|---:|---:|"]
            for name, values in metrics.items():
                lines.append(f"| {name} | {statistics.median(values):.2f} | {percentile(values, .95):.2f} | {min(values):.2f} | {max(values):.2f} |")
            lines.append("")
        lines += ["Per-request timeline (events show their end time; spans include duration):", ""]
        for sample in samples:
            lines += [f"### {sample['request_id']} — {sample['status']} ({sample['total_ms']:.2f} ms)", "", "| Event | At ms | Duration ms | Completed |", "|---|---:|---:|---|"]
            for event in sample["events"]:
                detail = event["detail"]
                duration = f"{detail['duration_ms']:.2f}" if "duration_ms" in detail else ""
                lines.append(f"| {event['name']} | {event['at_ms']:.2f} | {duration} | {detail.get('completed', '')} |")
            lines.append("")
    lines += ["Stage timings are inclusive: do not add nested spans (for example web_search and you_search_request). Ollama engine metrics also overlap their enclosing request span.", "", "Time to first answer text measures received content, not first reasoning token or pixels painted by the UI. Failed requests are retained in timelines and excluded from successful latency aggregates.", ""]
    return "\n".join(lines)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("traces", nargs="+", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    report = summarize(args.traces)
    if args.output:
        args.output.write_text(report, encoding="utf-8")
        print(f"Saved {args.output}")
    else:
        print(report)
