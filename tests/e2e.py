#!/usr/bin/env python3
"""Exercise the installed CLI/IPC contract with a private, simulation-only engine.

Uses only the Python standard library. Build bitengine and bitctl first. Nothing
installs a service, modifies login items, or contacts the campus gateway. Fixture
passwords are supplied over stdin and never printed, even on assertion failure.
"""
from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import time
import uuid


PROJECT = Path(__file__).resolve().parents[1]
MAX_FRAME = 1024 * 1024


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def process_image(pid: int) -> Path:
    """Read executable identity without enumerating or controlling other apps."""
    if os.name == "nt":
        from ctypes import wintypes
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel.OpenProcess.restype = wintypes.HANDLE
        kernel.QueryFullProcessImageNameW.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD)]
        kernel.QueryFullProcessImageNameW.restype = wintypes.BOOL
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        handle = kernel.OpenProcess(0x1000, False, pid)
        require(bool(handle), "Cannot verify owned child executable")
        try:
            buffer = ctypes.create_unicode_buffer(32768)
            count = wintypes.DWORD(len(buffer))
            require(bool(kernel.QueryFullProcessImageNameW(handle, 0, buffer, ctypes.byref(count))), "Cannot read owned child executable")
            return Path(buffer.value).resolve()
        finally:
            kernel.CloseHandle(handle)
    if sys.platform == "darwin":
        library = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        library.proc_pidpath.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_uint32]
        buffer = ctypes.create_string_buffer(4096)
        require(library.proc_pidpath(pid, buffer, len(buffer)) > 0, "Cannot verify owned child executable")
        return Path(os.fsdecode(buffer.value)).resolve()
    return Path(f"/proc/{pid}/exe").resolve(strict=True)


def verify_child(process: subprocess.Popen, executable: Path) -> None:
    require(process.poll() is None, "Owned child exited unexpectedly")
    require(os.path.normcase(str(process_image(process.pid))) == os.path.normcase(str(executable.resolve())), "Owned child executable identity changed")


def terminate_owned(process: subprocess.Popen, executable: Path) -> None:
    if process.poll() is not None:
        return
    try:
        verify_child(process, executable)
    except (AssertionError, OSError):
        # Shutdown can complete between poll() and the image query. Waiting on
        # our existing Popen handle is safe; never replace it by a PID lookup.
        try:
            process.wait(timeout=0.5)
            return
        except subprocess.TimeoutExpired:
            raise AssertionError("Refusing to stop a child whose executable could not be verified") from None
    process.terminate()
    try:
        process.wait(timeout=4)
    except subprocess.TimeoutExpired:
        verify_child(process, executable)
        process.kill()
        process.wait(timeout=4)


def receive_exact(stream, size: int) -> bytes:
    result = bytearray()
    while len(result) < size:
        chunk = stream.read(size - len(result)) if os.name == "nt" else stream.recv(size - len(result))
        require(bool(chunk), "Local IPC closed before a complete frame")
        result.extend(chunk)
    return bytes(result)


def raw_settings_request(data_dir: Path, pid: int, request: dict) -> dict:
    """One deterministic stale-revision test; never accepts secret commands.

    This helper runs in a bounded child process so a blocked pipe read is killed
    using that child's exact handle. Public CLI remains the path for all normal
    operations. Named-pipe server PID must equal the engine owned by this run.
    """
    require(request.get("command", {}).get("command") == "settings_put", "Raw IPC is restricted to non-secret settings")
    suffix = "-" + hashlib.sha256(str(data_dir).encode("utf-8")).hexdigest()[:16]
    if os.name == "nt":
        import msvcrt
        from ctypes import wintypes
        names = [name for name in os.listdir("\\\\.\\pipe\\") if name.startswith("BitNet-v1-user-") and name.endswith(suffix)]
        require(len(names) == 1, "Expected one isolated named pipe")
        stream = open("\\\\.\\pipe\\" + names[0], "r+b", buffering=0)
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.GetNamedPipeServerProcessId.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.ULONG)]
        kernel.GetNamedPipeServerProcessId.restype = wintypes.BOOL
        owner = wintypes.ULONG()
        handle = msvcrt.get_osfhandle(stream.fileno())
        require(bool(kernel.GetNamedPipeServerProcessId(handle, ctypes.byref(owner))) and owner.value == pid, "Isolated pipe belongs to another process")
    else:
        runtime_dir = Path(os.environ["XDG_RUNTIME_DIR"])
        endpoint = runtime_dir / f"bitnet-v1{suffix}.sock"
        require(endpoint.stat().st_uid == os.getuid(), "Isolated socket owner differs")
        stream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        stream.settimeout(6)
        stream.connect(str(endpoint))
        if sys.platform.startswith("linux"):
            peer_pid, peer_uid, _ = struct.unpack("3i", stream.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize("3i")))
            require(peer_pid == pid and peer_uid == os.getuid(), "Isolated socket belongs to another process")
    try:
        encoded = json.dumps(request, ensure_ascii=False).encode("utf-8")
        packet = struct.pack(">I", len(encoded)) + encoded
        if os.name == "nt":
            stream.write(packet)
        else:
            stream.sendall(packet)
        length = struct.unpack(">I", receive_exact(stream, 4))[0]
        require(0 < length <= MAX_FRAME, "Local IPC frame is out of range")
        reply = json.loads(receive_exact(stream, length))
        require(reply.get("request_id") == request["request_id"] and reply.get("api_version") == 1, "IPC correlation mismatch")
        return reply
    finally:
        stream.close()


class Harness:
    def __init__(self, binaries: Path, seconds: int):
        self.deadline = time.monotonic() + seconds
        self.binaries = binaries.resolve()
        extension = ".exe" if os.name == "nt" else ""
        self.engine_exe = self.binaries / ("bitengine" + extension)
        self.cli_exe = self.binaries / ("bitctl" + extension)
        require(self.engine_exe.is_file() and self.cli_exe.is_file(), "Build bitengine and bitctl before running this test")
        base = PROJECT / ".local" / "e2e"
        base.mkdir(parents=True, exist_ok=True)
        self.data_dir = base / (time.strftime("%Y%m%d-%H%M%S") + "-" + uuid.uuid4().hex[:10])
        self.data_dir.mkdir()
        self.data_dir = self.data_dir.resolve()
        require(self.data_dir.is_relative_to(base.resolve()), "E2E data escaped its project directory")
        self.env = os.environ.copy()
        self.env["BITNET_DATA_DIR"] = str(self.data_dir)
        self.env["BITNET_SIMULATE_ONLY"] = "1"
        if os.name == "nt":
            runtime_dir = self.data_dir / "run"
        else:
            # sockaddr_un is short (Linux: 108 bytes including NUL). Keep the
            # isolated runtime directory under this project, not /tmp or the
            # user's actual session runtime, even from a long WSL checkout.
            endpoint_length = len("/bitnet-v1-" + "0" * 16 + ".sock")
            name_length = min(9, 103 - len(os.fsencode(base.resolve())) - 1 - endpoint_length)
            require(name_length >= 4, "Checkout path is too long for an isolated Unix socket; use a shorter project path")
            runtime_dir = base / ("r" + uuid.uuid4().hex[:name_length - 1])
        self.env["XDG_RUNTIME_DIR"] = str(runtime_dir.resolve())
        self.env["PYTHONIOENCODING"] = "utf-8"
        Path(self.env["XDG_RUNTIME_DIR"]).mkdir(mode=0o700)
        self.process: subprocess.Popen | None = None
        self.engine_output = []
        self.outputs: list[bytes] = []
        self.secrets = ["simulation-only", "wrong", "e2e-secret-" + uuid.uuid4().hex, "e2e-secret-" + uuid.uuid4().hex]
        self.passed: list[str] = []

    def timeout(self, maximum: float = 12) -> float:
        remaining = self.deadline - time.monotonic()
        require(remaining > 0, "E2E total time limit reached")
        return min(maximum, remaining)

    def run(self, executable: Path, arguments: list[str], input_data: bytes | None = None, maximum: float = 12) -> subprocess.CompletedProcess:
        timeout = self.timeout(maximum)
        options = {"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}
        child = subprocess.Popen([str(executable), *arguments], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=self.env, cwd=PROJECT, **options)
        try:
            output, errors = child.communicate(input=input_data, timeout=timeout)
        except subprocess.TimeoutExpired:
            terminate_owned(child, executable)
            output, errors = child.communicate(timeout=2)
            self.outputs.extend([output, errors])
            raise AssertionError("A bounded child operation timed out") from None
        except BaseException:
            terminate_owned(child, executable)
            raise
        self.outputs.extend([output, errors])
        return subprocess.CompletedProcess(arguments, child.returncode, output, errors)

    def cli(self, *arguments: str, secret: str | None = None, code: int = 0, require_engine: bool = True):
        if require_engine:
            require(self.process is not None, "No owned engine")
            verify_child(self.process, self.engine_exe)
        result = self.run(self.cli_exe, ["--data-dir", str(self.data_dir), "--json", "--non-interactive", *arguments], None if secret is None else (secret + "\n").encode("utf-8"))
        require(result.returncode == code, "CLI returned exit " + str(result.returncode) + " instead of " + str(code) + " for " + " ".join(arguments[:2]))
        try:
            return json.loads(result.stdout)
        except (ValueError, UnicodeError):
            raise AssertionError("CLI did not return valid JSON") from None

    def status(self) -> dict:
        reply = self.cli("status")
        require(reply.get("ok") and reply["data"]["type"] == "snapshot", "Status reply is not a snapshot")
        snapshot = reply["data"]["data"]
        require(snapshot["scope"] == "user" and "simulation" in snapshot["capabilities"], "Refusing a non-simulation or system engine")
        return snapshot

    def start(self) -> None:
        number = len(self.engine_output) + 1
        output = open(self.data_dir / f"engine-{number}.stdout", "wb")
        errors = open(self.data_dir / f"engine-{number}.stderr", "wb")
        self.engine_output.extend([output, errors])
        options = {"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}
        self.process = subprocess.Popen([str(self.engine_exe), "--user", "--simulate", "--data-dir", str(self.data_dir)], stdin=subprocess.DEVNULL, stdout=output, stderr=errors, env=self.env, cwd=PROJECT, **options)
        until = time.monotonic() + self.timeout(10)
        while time.monotonic() < until:
            verify_child(self.process, self.engine_exe)
            result = self.run(self.cli_exe, ["--data-dir", str(self.data_dir), "--json", "status"], maximum=3)
            if result.returncode == 0:
                self.status()
                return
            time.sleep(0.1)
        raise AssertionError("Simulation engine did not become ready")

    def stop(self) -> None:
        require(self.process is not None, "No engine to stop")
        self.cli("stop")
        try:
            require(self.process.wait(timeout=self.timeout(5)) == 0, "Simulation engine did not shut down cleanly")
        finally:
            terminate_owned(self.process, self.engine_exe)
        self.process = None
        for output in self.engine_output:
            output.flush()

    def pass_case(self, name: str) -> None:
        self.passed.append(name)
        print("PASS " + name, flush=True)

    def statistics(self) -> dict:
        result = self.cli("statistics")
        require(result.get("ok") and result["data"]["type"] == "statistics", "Statistics reply is invalid")
        return result["data"]["data"]

    def exercise(self) -> None:
        # This query never starts a daemon; it only reads service registration.
        service = self.cli("service", "status", require_engine=False)
        require(not service["installed"], "Host has an installed service; run e2e in a disposable environment")
        self.start()
        snapshot = self.status()
        require(not snapshot["settings"]["automatic"] and not snapshot["settings"]["paused"], "Fresh simulation settings differ")
        require([a["id"] for a in self.cli("account", "list")] == ["demo"], "Simulation demo account is missing")
        self.pass_case("isolated startup and CLI status")

        self.cli("account", "add", "--id", "campus", "--username", "e2e-campus", "--password-stdin", secret=self.secrets[2])
        self.cli("account", "select", "demo")
        require(self.status()["settings"]["selected_account"] == "demo", "Account selection was not shared")
        accounts = self.cli("account", "list")
        require(any(a["id"] == "campus" and a["has_password"] for a in accounts), "Saved account metadata is missing")
        require(all("password" not in a for a in accounts), "Account metadata exposes a password property")
        self.pass_case("account add, concealed metadata and selection")

        self.cli("config", "set", "language", "en")
        self.cli("config", "set", "status_interval_secs", "5")
        gateway = self.status()["settings"]["gateway"]
        self.cli("profile", "save", "--id", "campus", "--name", "Campus")
        captured = self.status()["settings"]
        require(any(p["id"] == "campus" and p["gateway"] == gateway for p in captured["profiles"]), "Profile did not capture applied gateway settings")
        alternate_gateway = dict(gateway)
        alternate_gateway["base_url"] = "http://127.0.0.1"
        self.cli("config", "set", "gateway", json.dumps(alternate_gateway))
        self.cli("profile", "select", "campus")
        selected = self.status()["settings"]
        require(selected["active_profile"] == "campus" and selected["gateway"] == gateway, "Selecting a profile did not restore its captured gateway")
        self.cli("profile", "remove", "campus")
        require(not any(p["id"] == "campus" for p in self.status()["settings"]["profiles"]), "Profile removal did not persist")
        self.pass_case("profile capture, explicit selection and removal")

        imported = self.status()["settings"]
        injected_fingerprint = "a" * 64
        require(injected_fingerprint not in imported["trusted_networks"], "Trust fixture unexpectedly matches a trusted network")
        imported["trusted_networks"].append(injected_fingerprint)
        trust_file = self.data_dir / "trust-import.json"
        trust_file.write_text(json.dumps(imported), encoding="utf-8")
        self.cli("config", "import", str(trust_file))
        require(injected_fingerprint not in self.status()["settings"]["trusted_networks"], "Settings import added a trusted network")
        self.pass_case("settings import cannot add network trust")

        stale = self.status()["settings"]
        self.cli("pause")
        require(self.process is not None, "No owned engine for IPC")
        verify_child(self.process, self.engine_exe)
        request = {"api_version": 1, "request_id": "e2e-stale-" + uuid.uuid4().hex,
                   "command": {"command": "settings_put", "args": {"settings": stale, "expected_revision": stale["revision"]}}}
        helper = self.run(Path(sys.executable), [str(Path(__file__).resolve()), "--raw-settings", str(self.data_dir), str(self.process.pid)], json.dumps(request).encode("utf-8"), maximum=8)
        require(helper.returncode == 0, "Revision IPC helper failed")
        rejected = json.loads(helper.stdout)
        require(rejected["result"]["Err"]["code"] == "busy", "Stale revision was not rejected")
        after = self.status()["settings"]
        require(after["paused"] and after["revision"] > stale["revision"] and after["language"] == "en", "Stale settings overwrote newer intent")
        self.pass_case("settings persistence and deterministic revision conflict")

        self.cli("resume")
        require(self.status()["session"]["auth"] == "authenticated", "Simulation login did not complete")
        denied = self.cli("disconnect", code=7)
        require(denied["error"]["code"] == "confirmation_required" and not self.status()["settings"]["paused"], "Unconfirmed logout mutated state")
        self.cli("disconnect", "--yes")
        self.cli("check")
        snapshot = self.status()
        require(snapshot["settings"]["paused"] and snapshot["session"]["auth"] == "unauthenticated", "Manual logout did not persist pause intent")
        stored = json.loads((self.data_dir / "settings.json").read_text(encoding="utf-8"))
        require(stored["paused"], "Manual pause was not written to disk")
        imported = dict(stored)
        imported["paused"] = False
        imported["automatic"] = True
        import_file = self.data_dir / "pause-import.json"
        import_file.write_text(json.dumps(imported), encoding="utf-8")
        self.cli("config", "import", str(import_file))
        self.cli("check")
        snapshot = self.status()
        require(snapshot["settings"]["paused"] and snapshot["session"]["auth"] == "unauthenticated", "Config import cleared manual logout pause")
        self.stop()
        self.start()
        snapshot = self.status()
        require(snapshot["settings"]["paused"] and snapshot["session"]["auth"] == "unauthenticated" and snapshot["settings"]["language"] == "en", "Restart lost settings or manual logout pause")
        require([a["id"] for a in snapshot["accounts"]] == ["demo"], "Simulation unexpectedly persisted fixture credentials")
        self.pass_case("confirmation, logout pause and restart persistence")

        self.cli("account", "add", "--id", "campus", "--username", "e2e-campus", "--password-stdin", secret=self.secrets[1])
        self.cli("account", "select", "campus")
        before_failures = self.statistics()["login_failures"]
        error = self.cli("resume", code=6)
        require(error["error"]["code"] == "credentials", "Credential rejection has the wrong error")
        for _ in range(3):
            self.cli("check")
        snapshot = self.status()
        require(snapshot["state"] == "auth_error" and "campus" in snapshot["settings"]["blocked_accounts"] and snapshot["next_retry_at"] is None, "Rejected credentials were not held for replacement")
        require(self.statistics()["login_failures"] == before_failures + 1, "Rejected credentials retried repeatedly")
        imported = dict(snapshot["settings"])
        imported["blocked_accounts"] = []
        import_file = self.data_dir / "credential-import.json"
        import_file.write_text(json.dumps(imported), encoding="utf-8")
        self.cli("config", "import", str(import_file))
        self.cli("check")
        require("campus" in self.status()["settings"]["blocked_accounts"] and self.statistics()["login_failures"] == before_failures + 1, "Config import cleared a credential rejection hold")
        self.pass_case("settings imports cannot clear manual pause or credential holds")
        self.stop()
        self.start()
        require("campus" in self.status()["settings"]["blocked_accounts"], "Credential rejection pause did not survive restart")
        self.cli("account", "add", "--id", "campus", "--username", "e2e-campus", "--password-stdin", secret=self.secrets[3])
        self.cli("check")
        snapshot = self.status()
        require(snapshot["session"]["auth"] == "authenticated" and snapshot["session"]["username"] == "e2e-campus" and "campus" not in snapshot["settings"]["blocked_accounts"], "Replacing the password did not restore authentication")
        self.pass_case("credential rejection blocks retries and replacement recovers")

        self.cli("history", "--limit", "200")
        self.cli("statistics")
        self.cli("diagnose")
        issue_path = self.data_dir / "issue-report.md"
        issue = self.run(self.cli_exe, ["--data-dir", str(self.data_dir), "--json", "--non-interactive", "diagnose", "--issue", "--output", str(issue_path)])
        require(issue.returncode == 0, "Redacted Issue report export failed")
        issue_text = issue_path.read_text(encoding="utf-8")
        require(issue_text.startswith("#") and "```" in issue_text, "Issue report is not a Markdown diagnostic template")
        self.cli("config", "get")
        # Export has no stdout JSON payload by design.
        exported = self.run(self.cli_exe, ["--data-dir", str(self.data_dir), "--json", "--non-interactive", "config", "export", str(self.data_dir / "export.json")])
        require(exported.returncode == 0, "Non-secret settings export failed")
        self.cli("account", "remove", "campus", "--yes")
        require(self.status()["session"]["auth"] == "authenticated", "Deleting a saved account disconnected the active session")
        self.stop()
        unavailable = self.cli("account", "list", code=9, require_engine=False)
        require(unavailable["error"]["code"] == "unavailable", "Stopped simulation did not fail closed")
        absent = self.cli("status", code=9, require_engine=False)
        require(absent["error"]["code"] == "unavailable", "CLI spawned a real engine after simulation stopped")
        self.pass_case("redacted history, diagnostics and stopped-engine fail closed")

        for output in self.engine_output:
            output.close()
        leaked = []
        for secret in self.secrets:
            encoded = secret.encode("utf-8")
            if any(encoded in output for output in self.outputs):
                leaked.append("command output")
            for path in self.data_dir.rglob("*"):
                if path.is_file() and encoded in path.read_bytes():
                    leaked.append("isolated persisted data")
        require(not leaked, "A synthetic password leaked into output or persisted data")
        self.pass_case("synthetic passwords absent from CLI output, logs and database")

    def close(self) -> None:
        if self.process is not None:
            terminate_owned(self.process, self.engine_exe)
            self.process = None
        for output in self.engine_output:
            if not output.closed:
                output.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=PROJECT / "target" / "debug")
    parser.add_argument("--timeout", type=int, default=180, help="Total test deadline in seconds")
    parser.add_argument("--raw-settings", nargs=2, metavar=("DATA_DIR", "ENGINE_PID"), help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.raw_settings:
        try:
            data_dir = Path(args.raw_settings[0]).resolve()
            require(data_dir.is_relative_to((PROJECT / ".local" / "e2e").resolve()), "Raw IPC requires a private e2e directory")
            require(os.environ.get("BITNET_SIMULATE_ONLY") == "1" and os.environ.get("BITNET_DATA_DIR") == str(data_dir), "Raw IPC requires simulation-only isolation")
            request = json.load(sys.stdin)
            print(json.dumps(raw_settings_request(data_dir, int(args.raw_settings[1]), request)))
            return 0
        except Exception:
            print("Bounded settings IPC test failed", file=sys.stderr)
            return 1
    require(args.timeout > 0, "The total timeout must be positive")
    test = Harness(args.bin_dir, args.timeout)
    try:
        test.exercise()
        report = {"ok": True, "cases": test.passed, "simulation": True, "real_authentication": False,
                  "host_service_changes": False, "platform": sys.platform, "data_dir": str(test.data_dir)}
        (test.data_dir / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(f"PASS {len(test.passed)} end-to-end cases; private evidence: {test.data_dir}")
        return 0
    except Exception as error:
        # Never print request bodies, passwords, fixture values or raw subprocess output.
        print("FAIL " + (str(error) if isinstance(error, AssertionError) else type(error).__name__), file=sys.stderr)
        print(f"Private evidence retained: {test.data_dir}", file=sys.stderr)
        return 1
    finally:
        try:
            test.close()
        except Exception:
            print("Owned-child cleanup could not verify completion; no unrelated process was touched", file=sys.stderr)


if __name__ == "__main__":
    raise SystemExit(main())
