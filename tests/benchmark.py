#!/usr/bin/env python3
"""Measure an owned release engine in isolated simulation; no real network use.

This is a reproducible idle-state sample, not a real-network or 14-day soak test.
CPU is expressed as a percentage of one logical core (not divided by CPU count).
"""
from __future__ import annotations
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import statistics
import time
from e2e import Harness, PROJECT, require, verify_child


def sample(process) -> dict:
    if os.name == "nt":
        from ctypes import wintypes
        class Counters(ctypes.Structure):
            _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + [(name, ctypes.c_size_t) for name in ["PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage", "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage", "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage", "PrivateUsage"]]
        counters = Counters()
        counters.cb = ctypes.sizeof(counters)
        psapi = ctypes.WinDLL("psapi", use_last_error=True)
        psapi.GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.c_void_p, wintypes.DWORD]
        require(bool(psapi.GetProcessMemoryInfo(int(process._handle), ctypes.byref(counters), counters.cb)), "Cannot measure owned engine memory")
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
        values = [wintypes.FILETIME() for _ in range(4)]
        require(bool(kernel.GetProcessTimes(int(process._handle), *(ctypes.byref(v) for v in values))), "Cannot measure owned engine CPU")
        ticks = sum((v.dwHighDateTime << 32) | v.dwLowDateTime for v in values[2:])
        return {"rss_bytes": counters.WorkingSetSize, "private_bytes": counters.PrivateUsage, "cpu_seconds": ticks / 10_000_000}
    if Path(f"/proc/{process.pid}/stat").exists():
        fields = Path(f"/proc/{process.pid}/stat").read_text().rsplit(")", 1)[1].split()
        return {"rss_bytes": int(fields[21]) * os.sysconf("SC_PAGE_SIZE"), "cpu_seconds": (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")}
    raise AssertionError("Native measurement supports Windows and Linux")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=PROJECT / "target/release")
    parser.add_argument("--seconds", type=int, default=600)
    args = parser.parse_args()
    require(30 <= args.seconds <= 86400, "Sample duration must be 30..86400 seconds")
    test = Harness(args.bin_dir, args.seconds + 120)
    try:
        test.start()
        test.cli("account", "select", "demo")
        test.cli("resume")
        time.sleep(5)
        verify_child(test.process, test.engine_exe)
        initial = sample(test.process)
        start = time.monotonic()
        samples = []
        print("Measuring isolated release engine; no campus requests", flush=True)
        while time.monotonic() - start < args.seconds:
            time.sleep(min(2, max(0.01, args.seconds - (time.monotonic() - start))))
            verify_child(test.process, test.engine_exe)
            samples.append(sample(test.process))
        elapsed = time.monotonic() - start
        cpu = 100 * (samples[-1]["cpu_seconds"] - initial["cpu_seconds"]) / elapsed
        peak = max(s["rss_bytes"] for s in samples)
        with test.engine_exe.open("rb") as source:
            hasher = hashlib.sha256()
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                hasher.update(chunk)
            engine_hash = hasher.hexdigest()
        report = {"mode": "release_simulation_idle", "seconds": elapsed, "samples": len(samples), "cpu_percent_one_core": cpu, "peak_rss_mib": peak / 1048576, "median_rss_mib": statistics.median(s["rss_bytes"] for s in samples) / 1048576, "target_rss_mib": 30, "target_cpu_percent": 0.1, "meets_sample_targets": peak < 30 * 1048576 and cpu < 0.1, "real_network": False, "gui_included": False, "executable": str(test.engine_exe), "engine_sha256": engine_hash, "platform": os.name}
        test.stop()
        destination = test.data_dir / "benchmark.json"
        destination.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(report, indent=2), flush=True)
        print(str(destination), flush=True)
    finally:
        test.close()


if __name__ == "__main__":
    main()
