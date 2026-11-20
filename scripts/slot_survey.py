#!/usr/bin/env python3
"""Read-only survey of all 64 Crave pattern slots.
Groups slots by payload hash to detect edit-buffer-echo behavior.
Saves full results to slot_survey.json. READ-ONLY: sends only 0x77 gets.
"""
import hashlib
import json
import sys
import time
import urllib.request

BASE = sys.argv[1] if len(sys.argv) > 1 else "http://localhost:8090"
OUT = sys.argv[2] if len(sys.argv) > 2 else "slot_survey.json"


def call(method, path, body=None):
    req = urllib.request.Request(
        BASE + path,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"Content-Type": "application/json"},
        method=method,
    )
    with urllib.request.urlopen(req, timeout=25) as r:
        return json.loads(r.read())


st = call("GET", "/api/state")
print("connected:", st.get("device_name"), "| sim:", st.get("is_sim"))
if st.get("is_sim") or not st.get("device_id"):
    print("not connected to hardware — aborting")
    sys.exit(1)

results = {}
groups = {}
for bank in range(8):
    row = []
    for pat in range(8):
        d = call("POST", "/api/dump", {"bank": bank, "pattern": pat})
        resp = d.get("responses") or []
        if not resp:
            row.append("--------")
            results[f"{bank}:{pat}"] = {"responses": []}
            continue
        raw = resp[0]
        payload = bytes(int(x, 16) for x in raw.split())[8:-1]
        h = hashlib.sha1(payload).hexdigest()[:8]
        row.append(h)
        results[f"{bank}:{pat}"] = {"hash": h, "raw": raw, "decoded": d.get("decoded")}
        groups.setdefault(h, []).append(f"B{bank+1}P{pat+1}")
        time.sleep(0.05)
    print(f"bank {bank+1}: " + " ".join(row))

print(f"\n{len(groups)} unique payloads across 64 slots:")
for h, slots in groups.items():
    p = results[slots[0].replace("B", "").replace("P", ":")] if False else None
    print(f"  {h}: {len(slots)} slots: {', '.join(slots[:12])}{' …' if len(slots)>12 else ''}")

with open(OUT, "w") as f:
    json.dump({"groups": groups, "slots": results}, f, indent=1)
print("saved:", OUT)
