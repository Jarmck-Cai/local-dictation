"""Run a bounded set of synthetic text cases; never read audio or print credentials."""
from __future__ import annotations

from datetime import datetime, timezone
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from validation.core import load_local_env, mixed_error_rate, percentile
from validation.providers import clean_text


def main():
    load_local_env()
    cases = json.loads((ROOT / "validation" / "fixtures" / "cleanup_cases.json").read_text(encoding="utf-8"))
    results = []
    for case in cases:
        result = clean_text(case["raw"], "claude", timeout=12)
        entry = {**case, "result": result,
                 "missing_protected_terms": [term for term in case["protected_terms"] if term not in result["text"]],
                 "reference_mixed_error_rate": mixed_error_rate(case["reference"], result["text"])}
        results.append(entry)
        print(json.dumps({"id": case["id"], "status": result["status"], "error": result["error"],
                          "elapsed_seconds": round(result["elapsed_seconds"], 3),
                          "missing_protected_terms": entry["missing_protected_terms"], "text": result["text"]},
                         ensure_ascii=False), flush=True)
        if result["error"] in {"http_401", "http_403", "http_404", "http_429", "missing_anthropic_api_key"}:
            break
    input_tokens = sum((item["result"]["usage"] or {}).get("input_tokens", 0) for item in results)
    output_tokens = sum((item["result"]["usage"] or {}).get("output_tokens", 0) for item in results)
    report = {"created_at": datetime.now(timezone.utc).isoformat(), "kind": "synthetic_text_only",
              "runs": results, "summary": {"count": len(results),
              "ok": sum(item["result"]["status"] == "ok" for item in results),
              "p50_seconds": percentile([item["result"]["elapsed_seconds"] for item in results], 0.5),
              "input_tokens": input_tokens, "output_tokens": output_tokens,
              "estimated_token_cost_usd": input_tokens * 0.1 / 1e6 + output_tokens * 0.5 / 1e6,
              "cost_basis": "Haiku 5.5 Standard <=100K input, no cache writes; estimate not invoice",
              "manual_review_required": True}}
    path = ROOT / "outputs" / "validation" / (datetime.now().strftime("%Y%m%d-%H%M%S") + "-cleanup.json")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps({"report": str(path), "summary": report["summary"]}, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    main()
