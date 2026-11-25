"""Conflict-safe cleanup utility for `best-driver` temporary Chromium profiles and orphan processes.

Safety invariants:
1. Only ever targets directories directly inside the system temp directory whose basename
   starts with `best_driver_profile_` (never touches user-supplied `user_data_dir` or
   personal Chrome/Edge/Brave profiles).
2. Detects running Chromium processes whose `--user-data-dir` references a `best_driver_profile_`
   directory and skips them before deleting any file — preventing partial-deletion corruption.
3. On Windows, uses atomic directory staging (`os.rename(dir, dir + ".__bd_cleanup")`) before
   `shutil.rmtree` so any open handle anywhere inside a profile aborts cleanly with zero files lost.
4. With `kill_orphans=True`, only terminates Chromium process trees whose parent launcher PID
   is dead (true orphans); active browsers owned by live Python scripts are preserved.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import uuid
from typing import Any, Dict, Iterable, List, Optional, Sequence, Set, Tuple, TypedDict, Union

PathLike = Union[str, os.PathLike[str]]

PROFILE_PREFIX = "best_driver_profile_"
_STAGING_SUFFIX = ".__bd_cleanup"
_BROWSER_NAMES = frozenset(
    {
        "chrome.exe",
        "msedge.exe",
        "brave.exe",
        "chromium.exe",
        "chrome",
        "chromium",
        "chromium-browser",
        "google-chrome",
        "google-chrome-stable",
        "msedge",
        "microsoft-edge",
        "brave",
        "brave-browser",
    }
)
_USER_DATA_DIR_RE = re.compile(
    r'''"--user-data-dir=([^"]+)"|--user-data-dir=(?:"([^"]+)"|'([^']+)'|(\S+))''',
    re.IGNORECASE,
)


class CleanupReport(TypedDict):
    """Structured result returned by `best_driver.cleanup()`."""

    removed_dirs: List[str]
    freed_bytes: int
    killed_pids: List[int]
    skipped_in_use: List[str]
    skipped_recent: List[str]
    errors: List[str]
    dry_run: bool


def _norm_path(path: PathLike) -> str:
    return os.path.normcase(os.path.abspath(os.path.normpath(os.fspath(path))))


def _extract_profile_dir(cmdline: str) -> Optional[str]:
    if PROFILE_PREFIX not in cmdline:
        return None
    m = _USER_DATA_DIR_RE.search(cmdline)
    if not m:
        return None
    raw = (m.group(1) or m.group(2) or m.group(3) or m.group(4) or "").strip("\"'")
    if not raw:
        return None
    base = os.path.basename(os.path.normpath(raw))
    if not base.startswith(PROFILE_PREFIX):
        return None
    return _norm_path(raw)


def _dir_size_bytes(path: str) -> int:
    total = 0
    for root, _dirs, files in os.walk(path):
        for fn in files:
            try:
                total += os.path.getsize(os.path.join(root, fn))
            except OSError:
                pass
    return total


def _newest_mtime(path: str) -> float:
    try:
        newest = os.path.getmtime(path)
    except OSError:
        return 0.0
    try:
        with os.scandir(path) as it:
            for entry in it:
                try:
                    mt = entry.stat(follow_symlinks=False).st_mtime
                    if mt > newest:
                        newest = mt
                except OSError:
                    pass
    except OSError:
        pass
    return newest


def _is_pid_alive(pid: int) -> bool:
    if pid <= 0:
        return False
    if sys.platform == "win32":
        import ctypes

        PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
        STILL_ACTIVE = 259
        kernel32 = ctypes.windll.kernel32
        handle = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, int(pid))
        if not handle:
            return False
        try:
            code = ctypes.c_ulong(0)
            if kernel32.GetExitCodeProcess(handle, ctypes.byref(code)):
                return code.value == STILL_ACTIVE
            return True
        finally:
            kernel32.CloseHandle(handle)
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    except OSError:
        return False


def _scan_processes() -> Tuple[Dict[int, Tuple[int, str, str]], Set[int]]:
    """Return `(chromium_procs, all_live_pids)` where `chromium_procs[pid] = (ppid, name, cmdline)`."""
    chromium: Dict[int, Tuple[int, str, str]] = {}
    live_pids: Set[int] = set()

    if sys.platform == "win32":
        ps_script = (
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; "
            "$all = Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,CommandLine; "
            "$pids = @($all | ForEach-Object { $_.ProcessId }); "
            "$ch = @($all | Where-Object { $_.CommandLine -and $_.CommandLine.Contains('best_driver_profile_') }); "
            "@{ pids = $pids; procs = $ch } | ConvertTo-Json -Compress -Depth 3"
        )
        try:
            res = subprocess.run(
                ["powershell", "-NoProfile", "-NonInteractive", "-Command", ps_script],
                capture_output=True,
                encoding="utf-8",
                errors="replace",
                timeout=10,
                creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
            )
            if res.returncode == 0 and res.stdout.strip():
                payload = json.loads(res.stdout)
                raw_pids = payload.get("pids") or []
                if isinstance(raw_pids, int):
                    raw_pids = [raw_pids]
                live_pids = {int(p) for p in raw_pids if p is not None}

                raw_procs = payload.get("procs") or []
                if isinstance(raw_procs, dict):
                    raw_procs = [raw_procs]
                for item in raw_procs:
                    if not isinstance(item, dict):
                        continue
                    name = str(item.get("Name") or "").lower()
                    if name not in _BROWSER_NAMES:
                        continue
                    pid = int(item.get("ProcessId") or 0)
                    ppid = int(item.get("ParentProcessId") or 0)
                    cmd = str(item.get("CommandLine") or "")
                    if pid > 0:
                        chromium[pid] = (ppid, name, cmd)
        except Exception:
            pass
        return chromium, live_pids

    if sys.platform.startswith("linux") and os.path.isdir("/proc"):
        for entry in os.listdir("/proc"):
            if not entry.isdigit():
                continue
            pid = int(entry)
            live_pids.add(pid)
            try:
                with open(f"/proc/{pid}/cmdline", "rb") as fh:
                    raw = fh.read()
                if not raw or PROFILE_PREFIX.encode("utf-8") not in raw:
                    continue
                parts = [p.decode("utf-8", "ignore") for p in raw.split(b"\x00") if p]
                if not parts:
                    continue
                exe_name = os.path.basename(parts[0]).lower()
                if exe_name not in _BROWSER_NAMES:
                    continue
                ppid = 0
                try:
                    with open(f"/proc/{pid}/stat", "r", encoding="utf-8", errors="ignore") as sf:
                        stat_txt = sf.read()
                    rparen = stat_txt.rfind(")")
                    if rparen != -1:
                        fields = stat_txt[rparen + 2 :].split()
                        if len(fields) >= 2:
                            ppid = int(fields[1])
                except OSError:
                    pass
                chromium[pid] = (ppid, exe_name, " ".join(parts))
            except OSError:
                continue
        return chromium, live_pids

    # macOS / BSD fallback via `ps`
    try:
        res = subprocess.run(
            ["ps", "-ax", "-o", "pid=,ppid=,command="],
            capture_output=True,
            encoding="utf-8",
            errors="replace",
            timeout=10,
        )
        if res.returncode == 0:
            for line in res.stdout.splitlines():
                line = line.strip()
                if not line:
                    continue
                parts = line.split(None, 2)
                if len(parts) < 3 or not parts[0].isdigit() or not parts[1].isdigit():
                    continue
                pid, ppid, cmd = int(parts[0]), int(parts[1]), parts[2]
                live_pids.add(pid)
                if PROFILE_PREFIX not in cmd:
                    continue
                exe_name = os.path.basename(cmd.split(None, 1)[0]).lower()
                if exe_name in _BROWSER_NAMES or "chrome" in exe_name or "edge" in exe_name or "brave" in exe_name:
                    chromium[pid] = (ppid, exe_name, cmd)
    except Exception:
        pass
    return chromium, live_pids


def _posix_singleton_lock_alive(profile_dir: str) -> bool:
    lock_path = os.path.join(profile_dir, "SingletonLock")
    if not os.path.lexists(lock_path):
        return False
    try:
        target = os.readlink(lock_path)
    except OSError:
        return False
    # Format is "<hostname>-<pid>"
    if "-" in target:
        pid_str = target.rsplit("-", 1)[-1]
        if pid_str.isdigit():
            return _is_pid_alive(int(pid_str))
    return False


def _terminate_pid_tree(pid: int) -> bool:
    if pid <= 0 or pid == os.getpid():
        return False
    if sys.platform == "win32":
        try:
            res = subprocess.run(
                ["taskkill", "/PID", str(pid), "/T", "/F"],
                capture_output=True,
                encoding="utf-8",
                errors="replace",
                timeout=8,
                creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
            )
            return res.returncode == 0
        except Exception:
            return False
    try:
        os.kill(pid, signal.SIGTERM)
        for _ in range(15):
            if not _is_pid_alive(pid):
                return True
            time.sleep(0.05)
        os.kill(pid, signal.SIGKILL)
        return True
    except ProcessLookupError:
        return True
    except Exception:
        return False


def _safe_remove_dir(path: str) -> Tuple[bool, Optional[str]]:
    """Atomically stage then remove `path` so locked live profiles are never partially corrupted."""
    if sys.platform == "win32":
        staged = f"{path}{_STAGING_SUFFIX}_{os.getpid()}_{uuid.uuid4().hex[:6]}"
        try:
            os.rename(path, staged)
        except OSError:
            return False, None  # Locked / in-use by another process — skip untouched
        try:
            shutil.rmtree(staged)
            return True, None
        except OSError as exc:
            # Best-effort restore if deletion failed mid-way
            try:
                if os.path.exists(staged) and not os.path.exists(path):
                    os.rename(staged, path)
            except OSError:
                pass
            return False, f"{path}: {exc}"

    try:
        shutil.rmtree(path)
        return True, None
    except OSError as exc:
        return False, f"{path}: {exc}"


def cleanup(
    *,
    kill_orphans: bool = False,
    min_age_seconds: float = 0.0,
    exclude_dirs: Optional[Sequence[PathLike]] = None,
    dry_run: bool = False,
    temp_dir: Optional[PathLike] = None,
) -> CleanupReport:
    """Safely clean stale `best-driver` temporary Chromium profiles (and optional orphan processes).

    Args:
        kill_orphans: When `True`, terminates Chromium processes whose `--user-data-dir` is a
            `best_driver_profile_*` directory AND whose parent launcher process is dead.
            Active browsers owned by a running Python script are never killed.
        min_age_seconds: Skip directories modified more recently than this threshold (useful
            when multiple workers launch browsers concurrently).
        exclude_dirs: Additional profile directory paths to protect from cleanup.
        dry_run: Inspect and report what would be cleaned without killing processes or deleting files.
        temp_dir: Override the system temp directory to scan (defaults to `tempfile.gettempdir()`).
    """
    base_tmp = os.fspath(temp_dir) if temp_dir is not None else tempfile.gettempdir()
    protected: Set[str] = {_norm_path(p) for p in (exclude_dirs or ())}

    report: CleanupReport = {
        "removed_dirs": [],
        "freed_bytes": 0,
        "killed_pids": [],
        "skipped_in_use": [],
        "skipped_recent": [],
        "errors": [],
        "dry_run": dry_run,
    }

    chromium_procs, live_pids = _scan_processes()

    # Map normalized profile_dir -> set of Chromium PIDs using it
    profile_pids: Dict[str, Set[int]] = {}
    for pid, (_ppid, _name, cmd) in chromium_procs.items():
        pdir = _extract_profile_dir(cmd)
        if pdir:
            profile_pids.setdefault(pdir, set()).add(pid)

    # Identify root Chromium processes per profile and distinguish true orphans from active browsers
    in_use_dirs: Set[str] = set()
    orphan_roots_by_dir: Dict[str, List[int]] = {}

    for pdir, pids in profile_pids.items():
        roots = [pid for pid in pids if chromium_procs[pid][0] not in pids]
        if not roots:
            roots = list(pids)
        is_active = False
        for rpid in roots:
            ppid = chromium_procs[rpid][0]
            # Parent is alive and not Unix PID 1 (init/systemd/launchd) -> active bot owns this browser
            if ppid > 1 and (ppid in live_pids or _is_pid_alive(ppid)):
                is_active = True
                break
        if is_active or pdir in protected:
            in_use_dirs.add(pdir)
        else:
            orphan_roots_by_dir[pdir] = roots
            if not kill_orphans:
                in_use_dirs.add(pdir)

    if kill_orphans and orphan_roots_by_dir:
        killed_any = False
        for pdir, roots in orphan_roots_by_dir.items():
            for rpid in roots:
                if dry_run:
                    report["killed_pids"].append(rpid)
                elif _terminate_pid_tree(rpid):
                    report["killed_pids"].append(rpid)
                    killed_any = True
                else:
                    in_use_dirs.add(pdir)
        if killed_any and not dry_run:
            # Give OS a short window to release Chromium SQLite/lockfile handles
            time.sleep(0.35)

    now = time.time()
    try:
        entries = list(os.scandir(base_tmp))
    except OSError as exc:
        report["errors"].append(f"scandir({base_tmp}): {exc}")
        return report

    for entry in entries:
        name = entry.name
        if not name.startswith(PROFILE_PREFIX):
            continue
        if not entry.is_dir(follow_symlinks=False):
            continue
        full_path = entry.path
        norm = _norm_path(full_path)

        # Clean up any leftover staged directory from a prior interrupted cleanup
        is_staged = _STAGING_SUFFIX in name
        if norm in protected or norm in in_use_dirs:
            report["skipped_in_use"].append(full_path)
            continue
        if sys.platform != "win32" and not is_staged and _posix_singleton_lock_alive(full_path):
            report["skipped_in_use"].append(full_path)
            continue
        if min_age_seconds > 0 and not is_staged:
            age = now - _newest_mtime(full_path)
            if age < min_age_seconds:
                report["skipped_recent"].append(full_path)
                continue

        size_bytes = _dir_size_bytes(full_path)
        if dry_run:
            report["removed_dirs"].append(full_path)
            report["freed_bytes"] += size_bytes
            continue

        ok, err = _safe_remove_dir(full_path)
        if ok:
            report["removed_dirs"].append(full_path)
            report["freed_bytes"] += size_bytes
        elif err is not None:
            report["errors"].append(err)
        else:
            report["skipped_in_use"].append(full_path)

    return report


__all__ = ["CleanupReport", "PROFILE_PREFIX", "cleanup"]
