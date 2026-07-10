#!/usr/bin/env python3
"""Aktualisiert den eingebetteten models.dev-Katalog-Snapshot.

Laedt https://models.dev/api.json (oder liest --input <datei>), reduziert
das JSON auf die von folio genutzten Felder und schreibt es kompakt nach
src-tauri/src/ai/models-dev-snapshot.json.

Reduktion (Stand 2026-07-04, Original ~2,9 MB -> ~1,3 MB):
  Provider: id, name, env, api, doc
  Modelle:  id, name, reasoning, tool_call, attachment, limit, cost,
            release_date
Unbekannte/weitere Felder werden bewusst verworfen; der Rust-Parser ist
tolerant gegen fehlende Felder (alles ausser id optional).
"""

import argparse
import json
import sys
import urllib.request
from pathlib import Path

API_URL = "https://models.dev/api.json"
SNAPSHOT = Path(__file__).resolve().parent.parent / "src-tauri" / "src" / "ai" / "models-dev-snapshot.json"

PROVIDER_FIELDS = ["id", "name", "env", "api", "doc"]
MODEL_FIELDS = [
    "id", "name", "reasoning", "tool_call", "attachment",
    "limit", "cost", "release_date",
]


def reduce_catalog(full: dict) -> dict:
    out = {}
    for pid, p in full.items():
        rp = {k: p[k] for k in PROVIDER_FIELDS if k in p and p[k] is not None}
        rp["models"] = {
            mid: {k: m[k] for k in MODEL_FIELDS if k in m and m[k] is not None}
            for mid, m in p.get("models", {}).items()
        }
        out[pid] = rp
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--input", help="lokale api.json statt Download verwenden")
    args = ap.parse_args()

    if args.input:
        raw = Path(args.input).read_text(encoding="utf-8")
    else:
        with urllib.request.urlopen(API_URL, timeout=30) as resp:
            raw = resp.read().decode("utf-8")

    reduced = reduce_catalog(json.loads(raw))
    SNAPSHOT.parent.mkdir(parents=True, exist_ok=True)
    SNAPSHOT.write_text(
        json.dumps(reduced, separators=(",", ":"), ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    n_models = sum(len(p["models"]) for p in reduced.values())
    size_kb = SNAPSHOT.stat().st_size // 1024
    print(f"{SNAPSHOT}: {len(reduced)} Provider, {n_models} Modelle, {size_kb} KiB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
