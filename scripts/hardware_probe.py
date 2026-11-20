#!/usr/bin/env python3
"""Read-only hardware probe for a real Crave (plus one audible note blip).

Finds the real CRAVE device, connects, requests globals (0x75), dumps
bank1/pattern1 (0x77, read-only), and plays a 150ms note to prove the write
path. Does NOT change any device settings or patterns.
"""
import json
import sys
import time
import urllib.request
import urllib.error

BASE = sys.argv[1] if len(sys.argv) > 1 else "http://localhost:8090"


def call(method, path, body=None):
    req = urllib.request.Request(
        BASE + path,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"Content-Type": "application/json"},
        method=method,
    )
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        return {"__error__": e.read().decode(), "__status__": e.code}


devs = call("GET", "/api/devices")["devices"]
print("devices:")
for d in devs:
    print(f"  {d['id']:>10}  {d['kind']:<6} {d['name']}  in={d['in_port']} out={d['out_port']}")

crave = next((d for d in devs if d["kind"] == "crave" and d["out_port"]), None)
if not crave:
    print("\nFAIL: no real Crave device with an output port found")
    sys.exit(1)
assert not any(d["name"].lower().startswith("craved") for d in devs), "self-ports leaked into list"
print(f"\nconnecting to {crave['name']} (id {crave['id']})…")
c = call("POST", "/api/connect", {"device_id": crave["id"]})
if "__error__" in c:
    print("FAIL connect:", c)
    sys.exit(1)
print("connected:", c["device"]["name"])
print("globals_raw (0x75 response):", c.get("globals_raw") or "NONE — device did not answer globals request")

print("\ndumping bank 1 pattern 1 (read-only 0x77)…")
d = call("POST", "/api/dump", {"bank": 0, "pattern": 0})
print("request:", d.get("request_hex"))
for r in d.get("responses", []):
    print("response:", r)
if d.get("decoded"):
    print("decoded (experimental layout):", d["decoded"]["name"], "len", d["decoded"]["length"])
else:
    print("no decode under experimental layout — raw bytes above are the ground truth")

print("\nwrite path: 150ms note C2 (you should hear a blip)…")
n = call("POST", "/api/note", {"note": 36, "velocity": 100, "on": True})
print("note on:", n.get("sent") or n)
time.sleep(0.15)
n = call("POST", "/api/note", {"note": 36, "velocity": 0, "on": False})
print("note off:", n.get("sent") or n)

ok = bool(c.get("globals_raw")) or bool(d.get("responses"))
print("\nRESULT:", "device responded — read path OK" if ok else "NO RESPONSES from device — investigate")
sys.exit(0 if ok else 2)
