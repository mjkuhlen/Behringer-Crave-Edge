#!/usr/bin/env python3
"""ALSA sequencer MIDI proxy / capture tool (v2 — no feedback loop).

Creates a virtual client named CRAVE, forwards app traffic to the real Crave
and device responses back to subscribed app ports ONLY (never to subs, which
would feed the device its own output). Logs SysEx + channel voice + subs.

Usage: python3 midi_proxy.py [capture_log_path]
"""
import ctypes
import ctypes.util
import re
import subprocess
import sys
import time

alsa = ctypes.CDLL(ctypes.util.find_library("asound") or "libasound.so.2")

SND_SEQ_OPEN_DUPLEX = 3
CAP_READ = 1 << 0
CAP_WRITE = 1 << 1
CAP_SUBS_READ = 1 << 5
CAP_SUBS_WRITE = 1 << 6
EVENT_SYSEX = 130
EVENT_PORT_SUBSCRIBED = 66
EVENT_PORT_UNSUBSCRIBED = 67
QUEUE_DIRECT = 253
# realtime types we forward but never log (flood)
QUIET_TYPES = {33, 36, 37, 42}  # CLOCK-ish, TICK, SENSING


class Addr(ctypes.Structure):
    _fields_ = [("client", ctypes.c_ubyte), ("port", ctypes.c_ubyte)]


class SeqTime(ctypes.Union):
    _fields_ = [("tick", ctypes.c_uint * 2), ("real", ctypes.c_uint * 2)]


class DataExt(ctypes.Structure):
    _fields_ = [("len", ctypes.c_uint), ("ptr", ctypes.c_void_p)]


class Data(ctypes.Union):
    _fields_ = [("raw", ctypes.c_ubyte * 12), ("ext", DataExt)]


class Event(ctypes.Structure):
    _fields_ = [
        ("type", ctypes.c_ubyte), ("flags", ctypes.c_ubyte),
        ("tag", ctypes.c_ubyte), ("queue", ctypes.c_ubyte),
        ("time", SeqTime), ("source", Addr), ("dest", Addr), ("data", Data),
    ]


for fn, args in [
    ("snd_seq_open", [ctypes.POINTER(ctypes.c_void_p), ctypes.c_char_p, ctypes.c_int, ctypes.c_int]),
    ("snd_seq_set_client_name", [ctypes.c_void_p, ctypes.c_char_p]),
    ("snd_seq_create_simple_port", [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_uint, ctypes.c_uint]),
    ("snd_seq_connect_to", [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int]),
    ("snd_seq_connect_from", [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int]),
    ("snd_seq_event_input", [ctypes.c_void_p, ctypes.POINTER(ctypes.POINTER(Event))]),
    ("snd_seq_event_output", [ctypes.c_void_p, ctypes.POINTER(Event)]),
    ("snd_seq_drain_output", [ctypes.c_void_p]),
    ("snd_seq_free_event", [ctypes.POINTER(Event)]),
    ("snd_seq_client_id", [ctypes.c_void_p]),
]:
    getattr(alsa, fn).argtypes = args


def find_real_crave():
    out = subprocess.run(["aconnect", "-l"], capture_output=True, text=True).stdout
    m = re.search(r"client (\d+): 'CRAVE' \[type=kernel", out)
    if not m:
        raise SystemExit("real CRAVE kernel client not found — is it plugged in?")
    return int(m.group(1))


def main():
    logpath = sys.argv[1] if len(sys.argv) > 1 else "midi_capture.log"
    real = find_real_crave()
    h = ctypes.c_void_p()
    if alsa.snd_seq_open(ctypes.byref(h), b"default", SND_SEQ_OPEN_DUPLEX, 0) < 0:
        raise SystemExit("snd_seq_open failed")
    alsa.snd_seq_set_client_name(h, b"CRAVE")
    port = alsa.snd_seq_create_simple_port(
        h, b"CRAVE MIDI 1", CAP_READ | CAP_WRITE | CAP_SUBS_READ | CAP_SUBS_WRITE, 0)
    if port < 0:
        raise SystemExit("create_port failed")
    me = alsa.snd_seq_client_id(h)
    alsa.snd_seq_connect_to(h, port, real, 0)
    alsa.snd_seq_connect_from(h, port, real, 0)
    print(f"proxy {me}:{port} <-> real CRAVE {real}:0 ; log: {logpath}", flush=True)

    log = open(logpath, "a", buffering=1)
    app_ports = set()  # (client, port) of app-side subscribers (wine)
    t0 = time.time()

    def send(ev, dest_client, dest_port):
        fwd = Event()
        ctypes.memmove(ctypes.byref(fwd), ctypes.byref(ev), ctypes.sizeof(Event))
        fwd.source.client = me
        fwd.source.port = port
        fwd.queue = QUEUE_DIRECT
        fwd.dest.client = dest_client
        fwd.dest.port = dest_port
        alsa.snd_seq_event_output(h, ctypes.byref(fwd))
        alsa.snd_seq_drain_output(h)

    while True:
        evp = ctypes.POINTER(Event)()
        if alsa.snd_seq_event_input(h, ctypes.byref(evp)) < 0:
            continue
        ev = evp.contents
        ts = time.time() - t0
        from_dev = ev.source.client == real

        if ev.type == EVENT_PORT_SUBSCRIBED:
            # data.raw: sender client/port, dest client/port
            sc, sp, dc, dp = ev.data.raw[0], ev.data.raw[1], ev.data.raw[2], ev.data.raw[3]
            if dc == me and dp == port and sc != real:
                app_ports.add((sc, sp))
            line = f"[{ts:9.3f}] SUB {sc}:{sp} -> {dc}:{dp}"
            print(line, flush=True); log.write(line + "\n")
        elif ev.type == EVENT_PORT_UNSUBSCRIBED:
            sc, sp, dc, dp = ev.data.raw[0], ev.data.raw[1], ev.data.raw[2], ev.data.raw[3]
            app_ports.discard((sc, sp))
            line = f"[{ts:9.3f}] UNSUB {sc}:{sp} -> {dc}:{dp}"
            print(line, flush=True); log.write(line + "\n")
        elif ev.type == EVENT_SYSEX:
            n = ev.data.ext.len
            data = bytes((ctypes.c_ubyte * n).from_address(ev.data.ext.ptr))
            direction = "DEV->APP" if from_dev else "APP->DEV"
            line = f"[{ts:9.3f}] {direction} SYSEX ({n}b): {data.hex(' ')}"
            print(line, flush=True); log.write(line + "\n")
            if from_dev:
                for (c, p) in list(app_ports):
                    send(ev, c, p)
            else:
                send(ev, real, 0)
        else:
            if ev.type not in QUIET_TYPES:
                direction = "DEV->APP" if from_dev else "APP->DEV"
                line = f"[{ts:9.3f}] {direction} type={ev.type} data={bytes(ev.data.raw).hex(' ')}"
                log.write(line + "\n")
            if from_dev:
                for (c, p) in list(app_ports):
                    send(ev, c, p)
            else:
                send(ev, real, 0)
        alsa.snd_seq_free_event(evp)


if __name__ == "__main__":
    main()
