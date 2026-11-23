#!/usr/bin/env python3
"""End-to-end smoke test for craved against the built-in simulator.

Run: python3 scripts/smoke_test.py [base_url]
Exercises: device discovery, connect, settings SysEx, pattern dump/decode,
library CRUD, device write + dump roundtrip, playback, live notes, raw SysEx.
"""
import json
import sys
import time
import urllib.request
import urllib.error

BASE = sys.argv[1] if len(sys.argv) > 1 else "http://localhost:8090"
results = []


def call(method, path, body=None, timeout=25):
    req = urllib.request.Request(
        BASE + path,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"Content-Type": "application/json"},
        method=method,
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        return {"__error__": e.read().decode(), "__status__": e.code}


def check(name, cond, detail=""):
    results.append((name, bool(cond)))
    print(f"{'PASS' if cond else 'FAIL'}  {name}" + (f"  [{detail}]" if detail and not cond else ""))


# 1. health
h = call("GET", "/api/health")
check("health", h.get("ok") is True, str(h))

# 2. devices: simulator present, host ALSA ports visible from container
d = call("GET", "/api/devices")
devs = d.get("devices", [])
check("simulator listed", any(x["id"] == "sim-crave" for x in devs))
print("      devices:", ", ".join(f"{x['name']} ({x['kind']})" for x in devs))

# 3. connect to simulator
c = call("POST", "/api/connect", {"device_id": "sim-crave"})
check("connect sim", c.get("device", {}).get("id") == "sim-crave", str(c))
check("globals probe captured", bool(c.get("globals_raw")), str(c))

# 4. settings: change assign mode + accent threshold -> SysEx applied
s = call("GET", "/api/settings")["settings"]
s["assign_mode"] = 11  # MIDI Pitch Bend
s["accent_threshold"] = 80
u = call("PUT", "/api/settings", s)
names = [a["name"] for a in u.get("applied", [])]
check("settings sysex sent", "assign mode" in names and "accent threshold" in names, str(u))
check("settings persisted", call("GET", "/api/settings")["settings"]["assign_mode"] == 11)

# 5. dump seeded pattern B1 P1
dump = call("POST", "/api/dump", {"bank": 0, "pattern": 0})
dec = dump.get("decoded")
check("dump decoded", dec is not None, str(dump)[:200])
if dec:
    check("dump content", dec["length"] == 16 and dec["steps"][0]["note"] == 36 and dec["steps"][0]["accent"], dec["name"])
    check("raw sysex preserved", bool(dec.get("raw_sysex")))

    # 6. save to library, write to a different device slot, dump back (roundtrip)
    dec["name"] = "Smoke Test Roundtrip"
    dec["tempo"] = 140
    saved = call("POST", "/api/patterns", dec)
    pid = saved.get("id")
    check("library save", bool(pid), str(saved)[:200])
    w = call("POST", f"/api/patterns/{pid}/to-device", {"bank": 1, "pattern": 1, "confirm": True})
    check("device write", w.get("sent_hex", "").startswith("F0 00 20 32 00 01 05 78"), str(w)[:200])
    check("write verified by readback", w.get("verified") is True, str(w)[:200])
    back = call("POST", "/api/dump", {"bank": 1, "pattern": 1}).get("decoded")
    # tempo/swing/gate_length are GLOBAL on the device (knobs), not stored in
    # the pattern payload — only steps + length survive a device roundtrip.
    check(
        "write->dump roundtrip",
        back and back["steps"] == dec["steps"] and back["length"] == dec["length"],
        str(back)[:200],
    )
    # write without confirm must be refused
    nw = call("POST", f"/api/patterns/{pid}/to-device", {"bank": 1, "pattern": 1, "confirm": False})
    check("write requires confirm", "__error__" in nw)

    # 7. playback for ~1.2s then stop
    p = call("POST", "/api/play", {"pattern": saved, "channel": 1})
    check("play started", p.get("playing") is True, str(p))
    time.sleep(1.2)
    st = call("GET", "/api/state")
    check("playhead running", st.get("playing") is True, str(st))
    call("POST", "/api/stop", {})
    time.sleep(0.3)
    check("stopped", call("GET", "/api/state").get("playing") is False)

# 8. live note
n = call("POST", "/api/note", {"note": 48, "velocity": 110, "on": True})
check("note on", n.get("sent", "").startswith("90"), str(n))
n = call("POST", "/api/note", {"note": 48, "velocity": 0, "on": False})
check("note off", n.get("sent", "").startswith("80"), str(n))

# 9. raw sysex identity request
r = call("POST", "/api/sysex", {"hex": "F0 7E 7F 06 01 F7"})
check("raw sysex", r.get("sent") == "F0 7E 7F 06 01 F7", str(r))

# 10. library list + delete
lst = call("GET", "/api/patterns")["patterns"]
check("library list", any(p["name"] == "Smoke Test Roundtrip" for p in lst))
if pid:
    check("library delete", call("DELETE", f"/api/patterns/{pid}").get("deleted") is True)

# 11. ratchet: pattern-level live-performance param survives library roundtrip
rp = {
    "schema_version": 1, "id": "", "name": "Ratchet Test", "length": 4, "tempo": 160,
    "swing": 0, "gate_length": 50, "ratchet": 3,
    "steps": [{"note": 48 + i, "rest": False, "accent": False, "glide": False, "gate": 50, "velocity": 100} for i in range(4)]
             + [{"note": 36, "rest": True, "accent": False, "glide": False, "gate": 50, "velocity": 100} for _ in range(28)],
}
rps = call("POST", "/api/patterns", rp)
check("ratchet saved", rps.get("ratchet") == 3, str(rps)[:120])
rpl = call("GET", f"/api/patterns/{rps['id']}")
check("ratchet roundtrip", rpl.get("ratchet") == 3, str(rpl)[:120])
pr = call("POST", "/api/play", {"pattern": rpl, "channel": 1})
check("ratchet playback starts", pr.get("playing") is True, str(pr))
time.sleep(0.7)
call("POST", "/api/stop", {})

# 12. song mode: chain two patterns with repeats + transpose, play, CRUD
sp1 = call("POST", "/api/patterns", {**rp, "name": "Song Part A", "ratchet": 1})
sp2 = call("POST", "/api/patterns", {**rp, "name": "Song Part B", "ratchet": 1})
song = {
    "schema_version": 1, "id": "", "name": "Smoke Song", "loop_song": False,
    "entries": [
        {"pattern_id": sp1["id"], "repeats": 4, "transpose": 0},
        {"pattern_id": sp2["id"], "repeats": 4, "transpose": 12},
    ],
}
ss = call("POST", "/api/songs", song)
check("song saved", bool(ss.get("id")) and len(ss.get("entries", [])) == 2, str(ss)[:160])
check("song listed", any(x["id"] == ss["id"] for x in call("GET", "/api/songs")["songs"]))
ps = call("POST", "/api/play-song", {"song_id": ss["id"]})
check("song playback starts", ps.get("playing") is True and ps.get("entries") == 2, str(ps))
time.sleep(1.0)
check("song still playing mid-chain", call("GET", "/api/state").get("playing") is True)
call("POST", "/api/stop", {})
time.sleep(0.3)
su = call("PUT", f"/api/songs/{ss['id']}", {**ss, "name": "Smoke Song Renamed"})
check("song update", su.get("name") == "Smoke Song Renamed", str(su)[:120])
check("song delete", call("DELETE", f"/api/songs/{ss['id']}").get("deleted") is True)
call("DELETE", f"/api/patterns/{sp1['id']}")
call("DELETE", f"/api/patterns/{sp2['id']}")
call("DELETE", f"/api/patterns/{rps['id']}")
# empty song must be refused
check("empty song refused", "__error__" in call("POST", "/api/play-song", {"song": {**song, "entries": []}}))

# 13. modulation engine: config, start/stop, ASSIGN routing
mg = call("GET", "/api/modulation")
check("mod config has 2 slots", len(mg.get("config", {}).get("slots", [])) == 2, str(mg)[:120])
mc = mg["config"]
mc["slots"][0] = {**mc["slots"][0], "enabled": True, "shape": "sine", "rate_hz": 5.0,
                  "sync_beats": None, "depth": 40, "center": 64, "target": "cc1"}
mu = call("PUT", "/api/modulation", mc)
check("mod config applied", mu["config"]["slots"][0]["enabled"] is True, str(mu)[:120])
ms = call("POST", "/api/modulation/start", {})
check("mod starts", ms.get("running") is True, str(ms))
time.sleep(0.5)
check("mod running in state", call("GET", "/api/state").get("mod_running") is True)
ra = call("POST", "/api/modulation/route-assign", {"slot": 0})
check("route-assign sets CC1 mode", ra.get("assign_mode") == 12, str(ra))
check("assign persisted in settings", call("GET", "/api/settings")["settings"]["assign_mode"] == 12)
call("POST", "/api/modulation/stop", {})
check("mod stopped", call("GET", "/api/state").get("mod_running") is False)
# restore default mod config (disabled slots) so later runs start clean
mc["slots"][0]["enabled"] = False
call("PUT", "/api/modulation", mc)

# 13b. patch notebook: seeded recipe + user patch CRUD
patches = call("GET", "/api/patches").get("patches", [])
check("patch recipe seeded", any(p.get("is_recipe") and p.get("name") == "Crave — first sequenced note" for p in patches), str(patches)[:160])
patch = {
    "schema_version": 1, "id": "", "name": "Smoke Patch", "device_kind": "crave",
    "notes": "Smoke-test analog setup",
    "cables": [{"from": "VCO OUT", "to": "VCF IN", "color": "", "notes": "starter route"}],
    "controls": [{"control": "Clock source", "value": "USB/MIDI"}],
    "pattern_ids": [], "song_ids": [],
    "checklist": [{"text": "Set MIDI channel", "done": False}],
}
sp = call("POST", "/api/patches", patch)
check("patch saved", bool(sp.get("id")) and sp.get("cables", [{}])[0].get("to") == "VCF IN", str(sp)[:160])
pl = call("GET", f"/api/patches/{sp['id']}")
check("patch loaded", pl.get("notes") == "Smoke-test analog setup", str(pl)[:160])
pu = call("PUT", f"/api/patches/{sp['id']}", {**sp, "name": "Smoke Patch Renamed", "checklist": [{"text": "Set MIDI channel", "done": True}]})
check("patch updated", pu.get("name") == "Smoke Patch Renamed" and pu.get("checklist", [{}])[0].get("done") is True, str(pu)[:160])
check("patch deleted", call("DELETE", f"/api/patches/{sp['id']}").get("deleted") is True)

# 13c. multi-device API: per-device routes, select, targeted disconnect
st = call("GET", "/api/state")
check("state lists devices", any(x["id"] == "sim-crave" for x in st.get("devices", [])) and st.get("selected") == "sim-crave", str(st)[:160])
dp = call("POST", "/api/d/sim-crave/play", {"pattern": rp, "channel": 1})
check("per-device play", dp.get("playing") is True and dp.get("device") == "sim-crave", str(dp))
time.sleep(0.4)
ds = call("GET", "/api/d/sim-crave/state")
check("per-device state playing", ds.get("playing") is True, str(ds)[:120])
call("POST", "/api/d/sim-crave/stop", {})
check("per-device stop", call("GET", "/api/d/sim-crave/state").get("playing") is False)
ck = call("POST", "/api/d/sim-crave/clock/start", {"bpm": 120})
check("clock start", ck.get("clock_running") is True, str(ck))
time.sleep(0.3)
check("clock running in state", call("GET", "/api/d/sim-crave/state").get("clock_running") is True)
call("POST", "/api/d/sim-crave/clock/stop", {})
check("clock stop", call("GET", "/api/d/sim-crave/state").get("clock_running") is False)
check("select device", call("POST", "/api/select", {"device_id": "sim-crave"}).get("selected") == "sim-crave")
check("unknown device rejected", "__error__" in call("POST", "/api/select", {"device_id": "nope"}))
dd = call("POST", "/api/disconnect", {"device_id": "sim-crave"})
check("targeted disconnect", dd.get("disconnected") == "sim-crave" and not any(x["id"] == "sim-crave" for x in call("GET", "/api/state").get("devices", [])), str(dd)[:120])

# 14. clear-all on the simulator: backup-first, wipe, verify (slow: ~90s)
call("POST", "/api/connect", {"device_id": "sim-crave"})
ca = call("POST", "/api/clear-all", {"confirm": False})
check("clear-all requires confirm", "__error__" in ca)
ca = call("POST", "/api/clear-all", {"confirm": True, "backup_first": True}, timeout=300)
check("clear-all backed up 64", ca.get("backed_up") == 64, str(ca)[:200])
check("clear-all cleared 64", ca.get("cleared") == 64 and not ca.get("failed"), str(ca)[:200])
after = call("POST", "/api/dump", {"bank": 0, "pattern": 0}).get("decoded")
check("cleared slot is device default",
      after and after["length"] == 1 and not after["steps"][0]["rest"]
      and after["steps"][0]["note"] == 48 and all(s["rest"] for s in after["steps"][1:]),
      str(after)[:120])
check("backup survives in library",
      any(p["id"] == "sim-crave-b1-p1" for p in call("GET", "/api/patterns")["patterns"]))
# optional backup: clear again with backup_first=false
ca2 = call("POST", "/api/clear-all", {"confirm": True, "backup_first": False}, timeout=300)
check("clear-all without backup", ca2.get("cleared") == 64 and ca2.get("backed_up") == 0 and not ca2.get("failed"), str(ca2)[:200])
# restore-all: write the sim backups back, verify content returns
ra = call("POST", "/api/restore-all", {"confirm": True}, timeout=300)
check("restore-all restored 64", ra.get("restored") == 64 and not ra.get("failed") and not ra.get("missing"), str(ra)[:200])
restored = call("POST", "/api/dump", {"bank": 0, "pattern": 0}).get("decoded")
check("restored slot matches backup", restored and restored["steps"][0]["note"] == 36 and not restored["steps"][0]["rest"], str(restored)[:120])
# all-rest patterns store fine (the old "step 1 forced active" rule was an
# artifact of the pre-nibble-packing codec); verify a full all-rest roundtrip
allrest = {
    "schema_version": 1, "id": "", "name": "allrest-test", "length": 8, "tempo": 120,
    "swing": 0, "gate_length": 50,
    "steps": [{"note": 36, "rest": True, "accent": False, "glide": False, "gate": 50, "velocity": 100, "ratchet": 0} for _ in range(32)],
}
ar = call("POST", "/api/patterns", allrest)
w = call("POST", f"/api/patterns/{ar['id']}/to-device", {"bank": 2, "pattern": 2, "confirm": True})
check("all-rest write verified", w.get("verified") is True, str(w)[:200])
arback = call("POST", "/api/dump", {"bank": 2, "pattern": 2}).get("decoded")
check("all-rest roundtrip keeps step 0 rest",
      arback and all(s["rest"] for s in arback["steps"][:8]), str(arback)[:120])
call("DELETE", f"/api/patterns/{ar['id']}")
# cleanup: remove simulator backup entries from the shared library
for b in range(1, 9):
    for p in range(1, 9):
        call("DELETE", f"/api/patterns/sim-crave-b{b}-p{p}")
call("POST", "/api/disconnect", {})

# 12. delete-all: snapshot the shared library, wipe, verify, ALWAYS restore
import os
import tempfile
snap = call("GET", "/api/patterns")["patterns"]
fd, snap_path = tempfile.mkstemp(suffix=".json", prefix="craved_lib_snap_")
os.close(fd)
with open(snap_path, "w") as f:
    json.dump(snap, f)
try:
    r = call("POST", "/api/patterns/delete-all", {"confirm": False})
    check("delete-all requires confirm", "__error__" in r)
    r = call("POST", "/api/patterns/delete-all", {"confirm": True})
    check("delete-all deleted everything", r.get("deleted") == len(snap), str(r))
    check("library empty after delete-all", call("GET", "/api/patterns")["patterns"] == [])
finally:
    restore_failures = []
    for p in snap:
        r = call("POST", "/api/patterns", p)
        if r.get("id") != p["id"]:
            restore_failures.append(p["id"])
    os.unlink(snap_path)
check("library restored after test",
      len(call("GET", "/api/patterns")["patterns"]) == len(snap) and not restore_failures,
      f"failures: {restore_failures}")

# 13. web UI served
try:
    with urllib.request.urlopen(BASE + "/", timeout=10) as r:
        html = r.read().decode()
    check("web ui served", "CRAVED" in html and "app.js" in html)
except Exception as e:
    check("web ui served", False, str(e))

# 12. disconnect (only the simulator — leave any real hardware connected)
call("POST", "/api/disconnect", {"device_id": "sim-crave"})
check("disconnect", not any(x["id"] == "sim-crave" for x in call("GET", "/api/state").get("devices", [])))

failed = [n for n, ok in results if not ok]
print(f"\n{len(results) - len(failed)}/{len(results)} passed")
sys.exit(1 if failed else 0)
