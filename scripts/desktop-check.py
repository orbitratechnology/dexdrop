#!/usr/bin/env python3
"""End-to-end check of a packaged DexDrop desktop app (Windows, macOS, Linux).

Starts the real packaged app with its own state and download folders, has it
trust the `dexdrop` CLI and accept its transfers automatically, then:

1. checks that the app is listening and takes a screenshot of it;
2. checks that the CLI on the same machine discovers the app;
3. sends a file and a folder from the CLI (found by name, or by address if
   discovery is not available) and checks they arrive byte-identical in the
   app's download folder;
4. checks that the app recorded the transfer in its history.

    python3 scripts/desktop-check.py --app <app executable> --cli <dexdrop CLI> --out <dir>

Screenshots, logs and a summary go to the output folder (and to the GitHub
Actions job summary). Standard library only.
"""

import argparse
import base64
import json
import os
import platform
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

APP_NAME = "CI Desktop"


class Check:
    def __init__(self, out: Path):
        self.out = out
        self.lines = [f"## Desktop app check ({platform.system()})", ""]

    def note(self, line: str) -> None:
        print(line, flush=True)
        self.lines.append(line)

    def finish(self, ok: bool) -> None:
        text = "\n".join(self.lines) + "\n"
        (self.out / "summary.md").write_text(text, encoding="utf-8")
        summary = os.environ.get("GITHUB_STEP_SUMMARY")
        if summary:
            with open(summary, "a", encoding="utf-8") as f:
                f.write(text)
        sys.exit(0 if ok else 1)


def run(cmd, timeout=120, **kw):
    return subprocess.run(
        [str(c) for c in cmd],
        capture_output=True,
        text=True,
        timeout=timeout,
        **kw,
    )


def peer_id_of(cli: Path, state: Path) -> str:
    """The peer ID of the identity in `state`, creating it if needed."""
    res = run([cli, "--state", state, "id"])
    for line in res.stdout.splitlines():
        if line.strip().startswith("peer ID "):
            return line.strip().split()[-1]
    raise RuntimeError(f"no peer ID in `dexdrop id` output: {res.stdout} {res.stderr}")


def id_bytes(peer_id: str) -> list:
    padded = peer_id.upper() + "=" * (-len(peer_id) % 8)
    return list(base64.b32decode(padded))


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def listening(port: int) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=1):
            return True
    except OSError:
        return False


def screenshot(path: Path) -> bool:
    system = platform.system()
    try:
        if system == "Linux":
            cmd = ["import", "-window", "root", str(path)]
        elif system == "Darwin":
            cmd = ["screencapture", "-x", str(path)]
        else:
            script = (
                "Add-Type -AssemblyName System.Windows.Forms,System.Drawing;"
                "$b=[System.Windows.Forms.Screen]::PrimaryScreen.Bounds;"
                "$i=New-Object System.Drawing.Bitmap $b.Width,$b.Height;"
                "$g=[System.Drawing.Graphics]::FromImage($i);"
                "$g.CopyFromScreen($b.Location,[System.Drawing.Point]::Empty,$b.Size);"
                f"$i.Save('{path}');"
            )
            cmd = ["powershell", "-NoProfile", "-Command", script]
        return run(cmd, timeout=30).returncode == 0 and path.exists()
    except Exception:
        return False


ALLOW_SCRIPT = """
tell application "System Events"
  repeat with proc in (every process)
    try
      repeat with w in (every window of proc)
        if exists (button "Allow" of w) then
          click button "Allow" of w
          return name of proc
        end if
      end repeat
    end try
  end repeat
end tell
return ""
"""


def allow_local_network(timeout: float = 25) -> str:
    """Answers macOS's "Allow … to find devices on local networks?" prompt,
    as a person would the first time. Returns the process that showed it, or
    "" when it could not be answered (no prompt, or UI scripting refused)."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            res = run(["osascript", "-e", ALLOW_SCRIPT], timeout=30)
            shown_by = res.stdout.strip()
            if shown_by:
                return shown_by
        except Exception:
            pass
        time.sleep(2)
    return ""


def write_payload(src: Path) -> dict:
    """Files to send, by relative path."""
    files = {
        "photo.bin": os.urandom(5_000_000),
        "Docs/notes.txt": "Hello from the CI check.\n".encode() * 100,
        "Docs/Deep/data.bin": os.urandom(300_000),
        "Docs/empty.txt": b"",
    }
    for rel, data in files.items():
        p = src / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data)
    return files


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--app", required=True, type=Path, help="the app's executable")
    ap.add_argument("--cli", required=True, type=Path, help="the dexdrop CLI")
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument(
        "--require-discovery",
        action="store_true",
        help="fail when the CLI does not discover the app",
    )
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    check = Check(args.out)
    app, cli = args.app.resolve(), args.cli.resolve()
    if app.suffix == ".app":  # a macOS bundle: its executable
        import plistlib

        info = plistlib.loads((app / "Contents" / "Info.plist").read_bytes())
        app = app / "Contents" / "MacOS" / info["CFBundleExecutable"]
    for path in (app, cli):
        if not path.exists():
            check.note(f"FAILED: {path} does not exist")
            check.finish(False)

    work = Path(tempfile.mkdtemp(prefix="dexdrop-check-"))
    app_base, app_dl, cli_state = work / "app", work / "app-downloads", work / "cli"
    app_state = app_base / "state"
    port = free_port()

    # The app's identity is created ahead of time, so the CLI can address
    # it; the app trusts the CLI and accepts its transfers automatically.
    app_id = peer_id_of(cli, app_state)
    cli_id = peer_id_of(cli, cli_state)
    settings_path = app_state / "settings.json"
    settings = json.loads(settings_path.read_text()) if settings_path.exists() else {}
    settings.update(device_name=APP_NAME, visible=True, auto_accept_trusted=True,
                    local_network_explained=True)  # no first-use screen to tap
    settings_path.write_text(json.dumps(settings))
    (app_state / "trusted-devices.json").write_text(
        json.dumps(
            {
                "peers": [
                    {
                        "peer_id": id_bytes(cli_id),
                        "name": "CI CLI",
                        "platform": platform.system().lower(),
                        "auto_accept": True,
                        "added": 0,
                    }
                ]
            }
        )
    )
    check.note(f"- App: `{app.name}`, peer `{app_id[:12]}…`, port {port}")

    env = dict(
        os.environ,
        DEXDROP_STATE_DIR=str(app_base),
        DEXDROP_DOWNLOAD_DIR=str(app_dl),
        DEXDROP_PORT=str(port),
    )
    log = open(args.out / "app.log", "w", encoding="utf-8")
    proc = subprocess.Popen([str(app)], env=env, stdout=log, stderr=subprocess.STDOUT)
    ok = False
    try:
        t0 = time.time()
        while time.time() - t0 < 90 and proc.poll() is None and not listening(port):
            time.sleep(0.5)
        if proc.poll() is not None:
            check.note(f"FAILED: the app exited with code {proc.returncode} (app.log)")
            return
        if not listening(port):
            check.note("FAILED: the app is not listening after 90 s (app.log)")
            return
        check.note(f"- The app started and is listening ({time.time() - t0:.1f} s)")
        time.sleep(4)  # let the window render
        if screenshot(args.out / "01-started.png"):
            check.note("- Screenshot: `01-started.png`")
        require_discovery = args.require_discovery
        if platform.system() == "Darwin":
            # macOS asks the user before an app may use the local network.
            shown_by = allow_local_network()
            if shown_by:
                check.note(f'- Answered "Allow" to the local-network prompt ({shown_by})')
                time.sleep(3)
                if screenshot(args.out / "01b-allowed.png"):
                    check.note("- Screenshot: `01b-allowed.png`")
            else:
                check.note("- WARNING: could not answer the local-network prompt; discovery not required")
                require_discovery = False

        # Discovery: does the CLI see the app on the network?
        peers = run([cli, "--state", cli_state, "peers", "--wait", "6"])
        (args.out / "cli-peers.log").write_text(peers.stdout + peers.stderr)
        if APP_NAME in peers.stdout:
            target = APP_NAME
            check.note(f'- Discovery: the CLI sees "{APP_NAME}"')
        elif require_discovery:
            check.note(f'FAILED: discovery: the CLI does not see "{APP_NAME}" (cli-peers.log)')
            return
        else:
            target = f"127.0.0.1:{port}#{app_id}"
            check.note(f'- WARNING: discovery: the CLI does not see "{APP_NAME}"; sending by address')

        # A file and a folder (nested, with an empty file), accepted by the
        # app because it trusts the CLI.
        src = work / "src"
        files = write_payload(src)
        try:
            send = run(
                [cli, "--state", cli_state, "send", target, src / "photo.bin", src / "Docs"],
                timeout=180,
            )
        except subprocess.TimeoutExpired as stuck:
            # Keep what shows where it stopped: the CLI's output so far, and
            # what reached the app's download folder (temporary files too).
            out = (stuck.stdout or b"") + (stuck.stderr or b"")
            if isinstance(out, bytes):
                out = out.decode(errors="replace")
            arrived = sorted(
                f"{p.relative_to(app_dl)} {p.stat().st_size}"
                for p in app_dl.rglob("*")
                if p.is_file()
            ) if app_dl.exists() else []
            (args.out / "cli-send.log").write_text(
                out + "\n--- in the app's download folder:\n" + "\n".join(arrived)
            )
            check.note("FAILED: the transfer did not finish in 180 s (cli-send.log)")
            return
        (args.out / "cli-send.log").write_text(send.stdout + send.stderr)
        if send.returncode != 0:
            check.note(f"FAILED: the CLI reported a failed transfer (cli-send.log)")
            return
        for rel, data in files.items():
            got = app_dl / rel
            if not got.exists() or got.read_bytes() != data:
                check.note(f"FAILED: `{rel}` is missing or differs in the app's download folder")
                return
        total = sum(len(d) for d in files.values())
        check.note(
            f"- {len(files)} files ({total / 1e6:.1f} MB, one folder with a subfolder and an "
            "empty file) sent from the CLI, accepted automatically, received byte-identical"
        )

        # The app recorded the transfer.
        deadline = time.time() + 15
        outcome = None
        while time.time() < deadline:
            try:
                history = json.loads((app_state / "history.json").read_text())
                outcome = history["entries"][0]["outcome"]
                break
            except Exception:
                time.sleep(0.5)
        if outcome != "Completed":
            check.note(f"FAILED: the app's history shows {outcome!r}, not a completed transfer")
            return
        check.note("- The app recorded the transfer as completed in its history")
        time.sleep(2)
        if screenshot(args.out / "02-received.png"):
            check.note("- Screenshot: `02-received.png`")
        ok = True
    finally:
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(10)
            except subprocess.TimeoutExpired:
                proc.kill()
        log.close()
        shutil.rmtree(work, ignore_errors=True)
        if ok:
            check.note("")
            check.note("Passed.")
        check.finish(ok)


if __name__ == "__main__":
    main()
