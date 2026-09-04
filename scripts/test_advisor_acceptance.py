#!/usr/bin/env python3
"""Exercise the selfdev binary through an isolated Unix socket and real HTTP transport.

Default: deterministic local Responses API fixture; no external provider or user daemon.
--live: opt-in OpenAI API calls for all three modes, using OPENAI_API_KEY already
provided by the caller. Never claims the fixture is a live-model acceptance run.
"""

import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import tempfile
import threading
import time


MODES = ("interactive", "selfdev-guardian", "final-review")
SYNTHETIC_SECRET = "fixture-inline-private-value"
PROMPT = (
    "Dry-run review: a hard acceptance requirement is a successful test run. "
    "No tests have been run and you must not execute tools. "
    "Reply exactly: Implementation complete; tests were not run. "
    f"Synthetic redaction fixture OPENAI_API_KEY={SYNTHETIC_SECRET}"
)
PRIMARY = "Implementation complete; tests were not run."


def strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for child in value.values():
            yield from strings(child)
    elif isinstance(value, list):
        for child in value:
            yield from strings(child)


class Fixture(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self):
        super().__init__(("127.0.0.1", 0), Handler)
        self.reviews = []
        self.primary_tools = []
        self.errors = []


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_GET(self):
        data = json.dumps({"object": "list", "data": [{"id": "gpt-5", "object": "model"}]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        try:
            length = int(self.headers.get("Content-Length", "0"))
            assert 0 < length < 2 * 1024 * 1024, "unbounded provider request"
            body = json.loads(self.rfile.read(length))
            advisor = any("You are Jcode's independent advisor." in text for text in strings(body))
            if advisor:
                assert not body.get("tools"), "advisor received tools"
                encoded = json.dumps(body)
                assert SYNTHETIC_SECRET not in encoded, "unredacted advisor evidence"
                inputs = []
                for text in strings(body):
                    try:
                        value = json.loads(text)
                    except (ValueError, TypeError):
                        continue
                    if isinstance(value, dict) and "objective" in value:
                        inputs.append(value)
                assert inputs, "missing structured turn evidence"
                evidence = inputs[-1]
                assert len(json.dumps(evidence, ensure_ascii=False).encode()) <= 32768
                assert all(key in evidence for key in (
                    "diff_summary", "diagnostics", "verification_status", "outstanding_todos", "acceptance_criteria"
                )), "missing evidence fields"
                assert PRIMARY in evidence["latest_primary_turn"]
                self.server.reviews.append(body)
                result = json.dumps({
                    "severity": "blocker",
                    "summary": f"Unverified acceptance {len(self.server.reviews)} OPENAI_API_KEY={SYNTHETIC_SECRET}",
                    "evidence": [PRIMARY],
                    "recommended_action": "Run the required verification before claiming acceptance.",
                    "blocking": True,
                })
            else:
                self.server.primary_tools.append(len(body.get("tools", [])))
                result = PRIMARY
            response = {
                "id": "resp_fixture", "status": "completed", "model": "gpt-5",
                "output": [{"id": "msg_fixture", "type": "message", "role": "assistant",
                            "status": "completed", "content": [{"type": "output_text", "text": result}]}],
                "usage": {"input_tokens": 12, "output_tokens": 12, "total_tokens": 24},
            }
            events = [
                {"type": "response.created", "response": {"id": "resp_fixture", "status": "in_progress"}},
                {"type": "response.output_text.delta", "delta": result},
                {"type": "response.completed", "response": response},
            ]
            data = "".join(f"event: {event['type']}\ndata: {json.dumps(event)}\n\n" for event in events).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        except Exception as error:
            # All requests here contain synthetic fixture data. Retain assertion
            # messages only, never request bodies or authorization headers.
            self.server.errors.append(str(error))
            self.send_error(500, "fixture assertion failed")


class Client:
    def __init__(self, path, session_id, workspace):
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.settimeout(30)
        self.socket.connect(str(path))
        self.buffer = b""
        self.next_id = 1
        request = {"working_dir": str(workspace), "selfdev": False, "allow_session_takeover": True}
        if session_id:
            request["target_session_id"] = session_id
        self.send("subscribe", **request)
        event = self.until(lambda item: item.get("type") == "session")
        self.session_id = event["session_id"]
        assert not session_id or session_id == self.session_id, "restart attached a different session"

    def send(self, kind, **fields):
        request_id = self.next_id
        self.next_id += 1
        self.socket.sendall((json.dumps({"type": kind, "id": request_id, **fields}) + "\n").encode())
        return request_id

    def until(self, predicate, timeout=90):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            while b"\n" not in self.buffer:
                self.socket.settimeout(max(0.1, deadline - time.monotonic()))
                data = self.socket.recv(65536)
                if not data:
                    raise EOFError("isolated daemon disconnected")
                self.buffer += data
                assert len(self.buffer) <= 2 * 1024 * 1024, "unbounded socket response"
            line, self.buffer = self.buffer.split(b"\n", 1)
            event = json.loads(line)
            if event.get("type") == "error":
                # Provider failures may contain private data in live mode.
                raise AssertionError("daemon reported an error; inspect its private local log")
            if predicate(event):
                return event
        raise TimeoutError("isolated socket request timed out")

    def control(self, action, note_id=None):
        request = {"action": action}
        if note_id:
            request["note_id"] = note_id
        rid = self.send("advisor", request=request)
        return self.until(lambda item: item.get("type") == "advisor_result" and item.get("id") == rid)["result"]["message"]

    def turn(self):
        rid = self.send("message", content=PROMPT)
        self.until(lambda item: item.get("type") == "done" and item.get("id") == rid, timeout=150)

    def note(self, excluding=()):
        deadline = time.monotonic() + 95
        while time.monotonic() < deadline:
            message = self.control("inspect")
            assert SYNTHETIC_SECRET not in message, "inspect leaked synthetic secret"
            notes = [note for note in re.findall(r"adv-[a-f0-9-]+", message) if note not in excluding]
            if notes:
                return notes[0], message
            assert "Failed" not in self.control("status"), "advisor review failed"
            time.sleep(0.1)
        raise TimeoutError("advisor note did not arrive")

    def close(self):
        self.socket.close()


class Daemon:
    def __init__(self, binary, root, mode, fixture, model):
        self.binary = binary
        self.root = root
        self.home = root / "home"
        self.workspace = root / "work"
        self.path = root / "daemon.sock"
        self.home.mkdir()
        self.workspace.mkdir()
        subprocess.run(["git", "init", "-q", str(self.workspace)], check=True)
        self.log = (root / "daemon.log").open("wb")
        self.process = None
        # JCODE_HOME and the socket isolate all application state. Keep only
        # runtime essentials; do not inherit unrelated provider credentials.
        self.env = {key: os.environ[key] for key in (
            "PATH", "LANG", "LC_ALL", "LD_LIBRARY_PATH", "SSL_CERT_FILE", "SSL_CERT_DIR"
        ) if key in os.environ}
        self.env.update({
            "JCODE_HOME": str(self.home), "JCODE_RUNTIME_DIR": str(root / "runtime"),
            "JCODE_NO_TELEMETRY": "1", "DO_NOT_TRACK": "1", "JCODE_CI": "1",
            "JCODE_OPENAI_TRANSPORT": "https",
            "OPENAI_API_KEY": "fixture-key" if fixture else os.environ["OPENAI_API_KEY"],
        })
        if fixture:
            self.env["JCODE_OPENAI_API_BASE"] = f"http://127.0.0.1:{fixture.server_port}/v1"
        (self.home / "config.toml").write_text(
            f'[provider]\ndefault_provider = "openai-api"\ndefault_model = {json.dumps(model)}\n'
            '[sponsors]\nenabled = false\n'
            f'[advisor]\nenabled = true\nmode = "{mode}"\n'
            'max_reviews_per_session = 6\nhandled_note_immunity_turns = 2\nredact = true\n'
        )
        self.model = model

    def start(self, session_id=None):
        self.process = subprocess.Popen([
            str(self.binary), "--no-update", "--no-selfdev", "--provider", "openai-api",
            "--model", self.model, "--socket", str(self.path), "-C", str(self.workspace),
            "--tools", "read", "serve",
        ], env=self.env, stdout=self.log, stderr=subprocess.STDOUT, start_new_session=True)
        return self.connect(session_id)

    def connect(self, session_id):
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            assert self.process.poll() is None, "isolated daemon exited during startup/reload"
            try:
                return Client(self.path, session_id, self.workspace)
            except (FileNotFoundError, ConnectionRefusedError, ConnectionResetError, EOFError):
                time.sleep(0.1)
        raise TimeoutError("isolated daemon socket unavailable")

    def stop(self):
        if self.process and self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGKILL)
            self.process.wait(timeout=10)

    def checkpoint(self):
        paths = list((self.home / "state" / "advisor").glob("*.json"))
        assert paths, "missing durable advisor checkpoint"
        for path in paths:
            data = path.read_bytes()
            assert len(data) <= 256 * 1024
            assert SYNTHETIC_SECRET.encode() not in data
            assert b"private_context" not in data and b'"objective"' not in data
            assert path.stat().st_mode & 0o077 == 0, "checkpoint is not owner-only"


def exercise(binary, mode, fixture, model):
    with tempfile.TemporaryDirectory(prefix="jca-") as tmp:
        daemon = Daemon(binary, Path(tmp), mode, fixture, model)
        client = None
        try:
            client = daemon.start()
            session = client.session_id
            client.turn()
            note, message = client.note()
            assert "Evidence:" in message, "review has no evidence"
            assert "Acknowledged" in client.control("acknowledge", note)
            daemon.checkpoint()
            client.close()
            daemon.stop()  # Abrupt restart proves controls were durable before success.
            client = daemon.start(session)
            assert note in client.control("inspect") and "Acknowledged" in client.control("inspect")
            if fixture:
                review_count = len(fixture.reviews)
                for _ in range(2):
                    client.turn()
                    assert len(fixture.reviews) == review_count, "handled concern storm after restart"
                client.turn()
                second, _ = client.note(excluding=(note,))
                assert len(fixture.reviews) == review_count + 1
                assert "Dismissed" in client.control("dismiss", second)
                rid = client.send("reload", force=True)
                client.until(lambda item: item.get("type") == "reloading" or (
                    item.get("type") == "done" and item.get("id") == rid
                ))
                # Require the old connection to close: a no-op reload is not a pass.
                try:
                    client.until(lambda _item: False, timeout=45)
                except (EOFError, ConnectionResetError):
                    pass
                client.close()
                client = daemon.connect(session)
                assert second in client.control("inspect") and "Dismissed" in client.control("inspect")
            assert "disabled" in client.control("disable")
            client.close()
            daemon.stop()
            client = daemon.start(session)
            assert "Advisor: off" in client.control("status"), "disable lost on process restart"
            rid = client.send("rewind", message_index=1)
            client.until(lambda item: item.get("type") == "history" and item.get("id") == rid)
            assert "no retained notes" in client.control("inspect")
            assert "Advisor: off" in client.control("status"), "rewind revoked disable"
            rid = client.send("rewind_undo")
            client.until(lambda item: item.get("type") == "history" and item.get("id") == rid)
            assert "Advisor: off" in client.control("status")
            daemon.checkpoint()
            if fixture:
                assert not fixture.errors, fixture.errors
                assert any(fixture.primary_tools), "primary lost normal tools"
            return {"mode": mode, "status": "passed", "restart": True,
                    "reload_and_immunity": bool(fixture), "rewind": True}
        except Exception:
            if fixture:
                print("Fixture errors:", fixture.errors)
                # Fixture-only logs contain no real credentials or user data.
                print((Path(tmp) / "daemon.log").read_text(errors="replace")[-7000:])
            raise
        finally:
            if client:
                client.close()
            daemon.stop()
            daemon.log.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/selfdev/jcode"))
    parser.add_argument("--live", action="store_true", help="Use the caller's OpenAI API key (billable).")
    parser.add_argument("--model", default="gpt-5")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    if args.live and not os.environ.get("OPENAI_API_KEY"):
        parser.error("--live requires an existing OPENAI_API_KEY; no provider calls were made")
    fixture = None if args.live else Fixture()
    if fixture:
        threading.Thread(target=fixture.serve_forever, daemon=True).start()
    try:
        results = [exercise(binary, mode, fixture, args.model) for mode in MODES]
        print(json.dumps({"provider": "live-openai" if args.live else "local-http-fixture",
                          "binary_sha256": hashlib.file_digest(binary.open("rb"), "sha256").hexdigest(),
                          "results": results}, indent=2))
    finally:
        if fixture:
            fixture.shutdown()
            fixture.server_close()


if __name__ == "__main__":
    main()
