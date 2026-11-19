#!/usr/bin/env python3
"""Pair over SSH against an isolated engine with the actual `zuse` command, then connect through
the tunnel. Uses an in-process SSH server, so it never touches ~/.ssh or a running Zuse."""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

root = Path(__file__).resolve().parents[3]
jobs = os.environ.get("CARGO_BUILD_JOBS", "4")
subprocess.run(["cargo", "build", "-j", jobs, "-p", "zuse-cli"], cwd=root, check=True)
subprocess.run(["cargo", "build", "-j", jobs, "--release", "-p", "zuse-mobile-bridge"], cwd=root, check=True)
metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=root))
target = Path(metadata["target_directory"])
binary = target / "debug" / "zuse-cli"
java = Path(os.environ["JAVA_HOME"])


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


with tempfile.TemporaryDirectory(prefix="zuse-ssh-acceptance-") as data:
    native = Path(data) / "jni"
    native.mkdir()
    subprocess.run(["cc", "-shared", "-fPIC", "-O2", "-I" + str(java / "include"), "-I" + str(java / "include/linux"), "-I" + str(root / "crates/mobile-bridge/include"), str(root / "apps/android/native/pairing-jni.c"), str(target / "release/libzuse_mobile_bridge.a"), "-ldl", "-lpthread", "-lm", "-o", str(native / "libzuse_pairing.so")], check=True)
    listen, ipc = free_port(), free_port()
    env = os.environ.copy()
    env["ZUSE_NATIVE_JNI_DIR"] = str(native)
    with open(Path(data) / "server.log", "w") as log:
        server = subprocess.Popen([str(binary), "serve", "--listen", f"127.0.0.1:{listen}", "--ipc-port", str(ipc), "--data-dir", str(Path(data) / "profile")], cwd=root, env=env, stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 60
            while True:
                if server.poll() is not None:
                    raise RuntimeError("Isolated engine exited before readiness: " + (Path(data) / "server.log").read_text()[-2000:])
                try:
                    with socket.create_connection(("127.0.0.1", ipc), timeout=0.5):
                        break
                except OSError:
                    if time.monotonic() >= deadline:
                        raise TimeoutError("Isolated engine did not start in 60 seconds")
                    time.sleep(0.2)
            env["ZUSE_SSH_E2E_BIN"] = str(binary)
            env["ZUSE_SSH_E2E_IPC_PORT"] = str(ipc)
            subprocess.run([str(root / "apps/android/gradlew"), "-p", str(root / "apps/android"), ":core:test", "--tests", "sh.zuse.core.RealSshPairingTests", "--rerun-tasks", *os.environ.get("ZUSE_GRADLE_ARGS", "").split()], cwd=root, env=env, check=True)
        finally:
            server.terminate()
            server.wait(timeout=20)
print("SSH pairing acceptance passed")
