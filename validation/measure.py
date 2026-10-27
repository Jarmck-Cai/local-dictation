from __future__ import annotations

import subprocess
import threading
import time

import psutil


class Meter:
    """Process CPU time and sampled resident memory; CPU % is whole-machine normalized."""

    def __enter__(self):
        self.process = psutil.Process()
        self.start_rss = self.peak_rss = self.process.memory_info().rss
        times = self.process.cpu_times()
        self.start_cpu = times.user + times.system
        self.started = time.perf_counter()
        self.stopped = threading.Event()

        def sample():
            while not self.stopped.wait(0.02):
                self.peak_rss = max(self.peak_rss, self.process.memory_info().rss)

        self.thread = threading.Thread(target=sample, daemon=True)
        self.thread.start()
        return self

    def __exit__(self, *args):
        elapsed = time.perf_counter() - self.started
        times = self.process.cpu_times()
        cpu_seconds = times.user + times.system - self.start_cpu
        end_rss = self.process.memory_info().rss
        self.stopped.set()
        self.thread.join()
        self.result = {"wall_seconds": elapsed, "cpu_seconds": cpu_seconds,
                       "cpu_machine_percent": 100 * cpu_seconds / max(elapsed, 1e-9) / (psutil.cpu_count() or 1),
                       "rss_start_mib": self.start_rss / 1048576,
                       "rss_end_mib": end_rss / 1048576,
                       "rss_peak_mib": max(self.peak_rss, end_rss) / 1048576}


def gpu_snapshot() -> dict | None:
    try:
        output = subprocess.check_output(
            ["nvidia-smi", "--query-gpu=name,memory.used,utilization.gpu", "--format=csv,noheader,nounits"],
            text=True, timeout=3, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        name, used, utilization = output.splitlines()[0].rsplit(",", 2)
        return {"scope": "whole_device_not_this_process", "name": name.strip(),
                "memory_used_mib": float(used), "utilization_percent": float(utilization)}
    except (OSError, ValueError, subprocess.SubprocessError):
        return None
