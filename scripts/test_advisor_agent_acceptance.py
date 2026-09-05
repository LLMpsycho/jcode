#!/usr/bin/env python3
"""Behavioral advisor scenarios using the isolated daemon and HTTP transport.

The provider is deterministic; real tool execution, advisor scheduling, message
injection, cancellation, and Unix socket dispatch are exercised by jcode itself.
"""

import json
from pathlib import Path
import tempfile
import threading
import time

from test_advisor_acceptance import (
    Daemon, Fixture, SYNTHETIC_SECRET, assert_fixture_provider_settings,
    configure_fixture_advisor, function_call, strings,
)

SOURCE_MARKER = "off_by_one_boundary_in_source"
ADVICE_MARKER = "INDEPENDENT_ADVISOR_VERIFIED_OFF_BY_ONE"
CORRECTED = "Corrected the boundary: accept index < length."
SCENARIOS = ("investigate-and-correct", "healthy-silence", "cancel-in-flight")


def tool_outputs(body):
    return [item.get("output", "") for item in body.get("input", [])
            if isinstance(item, dict) and item.get("type") == "function_call_output"]


class AgentFixture(Fixture):
    def __init__(self, scenario):
        super().__init__()
        self.scenario = scenario
        self.primary_started = threading.Event()
        self.advisor_started = threading.Event()
        self.advice_emitted = threading.Event()
        self.release_cancelled = threading.Event()
        self.independent_read = False
        self.corrected = False
        self.final_answers = 0
        self.investigation_calls = 0
        self.repair_requested = False
        self.primary_received_advice = False
        self.observations = []

    def reply(self, body, advisor):
        encoded = json.dumps(body)
        if advisor:
            self.reviews.append(body)
            self.advisor_started.set()
            assert SYNTHETIC_SECRET not in encoded, "advisor received an unredacted secret"
            tools = {tool.get("name") for tool in body.get("tools", [])}
            assert all(tool.get("type") == "function" for tool in body.get("tools", [])), "advisor received provider-hosted tools"
            assert {"read", "advise"} <= tools, "advisor cannot investigate and emit advice"
            assert not {"write", "edit", "bash"}.intersection(tools), "advisor can mutate the workspace"
            if self.scenario == "healthy-silence":
                return '{"silence":true}'
            if self.scenario == "cancel-in-flight":
                assert self.release_cancelled.wait(20), "cancellation scenario never released provider"
                return self.advice("cancelled-concern")
            if self.advice_emitted.is_set():
                return '{"silence":true}'
            assert self.primary_started.wait(15), "advisor is not running alongside primary work"
            outputs = "\n".join(strings(tool_outputs(body)))
            if SOURCE_MARKER not in outputs:
                # Neither the user prompt nor the primary's README read provides
                # this source. The advisor must inspect it through its own tool.
                assert SOURCE_MARKER not in encoded, "source was already supplied before investigation"
                self.investigation_calls += 1
                return function_call("read", {"file_path": "src/bounds.py"}, "advisor_source")
            self.independent_read = True
            self.observations.append("advisor-read-source")
            return self.advice("boundary-off-by-one")

        self.primary_tools.append(len(body.get("tools", [])))
        self.primary_settings.append({
            "model": body.get("model"),
            "reasoning_effort": body.get("reasoning", {}).get("effort"),
        })
        self.primary_started.set()
        if self.scenario == "cancel-in-flight":
            assert self.release_cancelled.wait(20), "cancellation scenario never released primary"
            return "This cancelled result must not resume the session."
        if self.scenario == "healthy-silence":
            self.final_answers += 1
            return "The task is complete; no issues were found."
        self.primary_received_advice |= ADVICE_MARKER in encoded
        if self.primary_received_advice:
            assert self.independent_read, "primary received advice without source investigation"
            outputs = "\n".join(strings(tool_outputs(body)))
            if SOURCE_MARKER not in outputs and not self.repair_requested:
                return function_call("read", {"file_path": "src/bounds.py"}, "primary_source")
            if not self.repair_requested:
                self.repair_requested = True
                return function_call("edit", {
                    "file_path": "src/bounds.py", "old_string": "return index <= length",
                    "new_string": "return index < length",
                }, "primary_repair")
            assert "return index < length" in outputs, "primary repair failed or its result was lost"
            self.corrected = True
            self.final_answers += 1
            self.observations.append("primary-corrected-after-advice")
            return CORRECTED
        assert len(self.primary_settings) <= 8, "advisor never steered the running primary"
        if len(self.primary_settings) > 1:
            assert self.advice_emitted.wait(15), "advisor did not finish independent investigation"
        self.observations.append("primary-tool-step")
        return function_call("read", {
            "file_path": "README.md", "start_line": len(self.primary_settings),
            "end_line": len(self.primary_settings),
        }, f"primary_read_{len(self.primary_settings)}")

    def advice(self, concern_id):
        self.advice_emitted.set()
        self.observations.append("advisor-emitted-advice")
        return function_call("advise", {
            "concern_id": concern_id,
            "severity": "blocker",
            "summary": ADVICE_MARKER,
            "evidence": ["src/bounds.py: return index <= length"],
            "recommended_action": "Read src/bounds.py and fix the boundary to index < length before finalizing.",
        }, f"advisor_note_{len(self.reviews)}")


def assert_correction(fixture, client, request_id):
    assert fixture.independent_read and fixture.investigation_calls == 1, "advisor did not independently inspect source"
    assert fixture.corrected and fixture.final_answers == 1, "primary did not correct the first turn"
    assert fixture.observations.index("advisor-read-source") < fixture.observations.index("primary-corrected-after-advice")
    done = [event for event in client.events if event.get("type") == "done" and event.get("id") == request_id]
    assert len(done) == 1, "corrective work required another client turn"
    assert any(CORRECTED in text for event in client.events for text in strings(event)), "corrected answer was not delivered"


def exercise_agent(binary, scenario, model):
    fixture = AgentFixture(scenario)
    threading.Thread(target=fixture.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix="jca-agent-") as tmp:
        daemon = Daemon(binary, Path(tmp), "interactive", fixture, model, tools="read,edit")
        (daemon.workspace / "src").mkdir()
        (daemon.workspace / "src" / "bounds.py").write_text(
            f"# {SOURCE_MARKER}\ndef valid(index, length):\n    return index <= length\n"
        )
        (daemon.workspace / "README.md").write_text(
            "Inspect the index boundary before explaining it.\n" * 8
        )
        client = None
        try:
            client = daemon.start()
            configure_fixture_advisor(client)
            request_id = client.send("message", content=(
                "Fix the index boundary implementation. Inspect README.md first. "
                f"Synthetic redaction fixture OPENAI_API_KEY={SYNTHETIC_SECRET}"
            ))
            if scenario == "cancel-in-flight":
                assert fixture.primary_started.wait(15), "primary never started"
                assert fixture.advisor_started.wait(15), "advisor did not run concurrently"
                cancel_id = client.send("cancel")
                client.until(lambda event: event.get("type") == "done" and
                             event.get("id") in (request_id, cancel_id), timeout=15)
                fixture.release_cancelled.set()
                # The HTTP fixture completes the old requests after cancellation.
                # Status round trips let the daemon process any stale callbacks.
                deadline = time.monotonic() + 2
                while time.monotonic() < deadline:
                    assert "no retained notes" in client.control("inspect"), "cancelled advisor callback published a note"
                    time.sleep(0.05)
                assert len(fixture.primary_settings) == 1, "cancelled advice resumed the primary"
            else:
                client.until(lambda event: event.get("type") == "done" and
                             event.get("id") == request_id, timeout=90)
                if scenario == "investigate-and-correct":
                    assert_correction(fixture, client, request_id)
                    source = (daemon.workspace / "src" / "bounds.py").read_text()
                    assert "return index < length" in source and "return index <= length" not in source, "advisor feedback did not lead to an actual code repair"
                    assert ADVICE_MARKER in client.control("inspect"), "advisor note was not inspectable"
                else:
                    assert fixture.reviews, "healthy work was never observed"
                    assert "no retained notes" in client.control("inspect"), "healthy advisor emitted unsolicited advice"
                    assert len(fixture.primary_settings) == 1, "healthy silence restarted primary work"
            assert_fixture_provider_settings(fixture, model, 0, 0)
            assert not fixture.errors, fixture.errors
            return {"mode": "interactive", "scenario": scenario, "status": "passed", "client_turns": 1,
                    "advisor_requests": len(fixture.reviews),
                    "independent_source_read": fixture.independent_read,
                    "primary_corrected_before_done": fixture.corrected}
        except Exception:
            print("Fixture errors:", fixture.errors)
            print((Path(tmp) / "daemon.log").read_text(errors="replace")[-7000:])
            raise
        finally:
            fixture.release_cancelled.set()
            if client:
                client.close()
            daemon.stop()
            daemon.log.close()
            fixture.shutdown()
            fixture.server_close()


def exercise_agent_scenarios(binary, model):
    return [exercise_agent(binary, scenario, model) for scenario in SCENARIOS]
