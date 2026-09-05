import { test } from "node:test";
import assert from "node:assert/strict";
import {
  HarnessError,
  JcodeClient,
  isKnownEvent,
  type AdvisorControlResult,
  type AdvisorRequest,
  type AdvisorRouteSelection,
  type ApiEvent,
  type ApiRequest,
  type Transport,
} from "../dist/index.js";

type RequestFrame = ApiRequest & { v: number; id: number };
type Send = (frame: ApiEvent & { reply_to?: number }) => void;

function mockTransport(onRequest: (request: RequestFrame, send: Send) => void) {
  const requests: RequestFrame[] = [];
  let onData: (chunk: string) => void = () => {};
  let onClose: (error?: Error) => void = () => {};
  const send: Send = (frame) => onData(`${JSON.stringify({ v: 1, ...frame })}\n`);
  const transport: Transport = {
    write(data) {
      const frame = JSON.parse(data) as RequestFrame;
      if (frame.req === "hello") {
        send({
          ev: "hello_ok",
          reply_to: frame.id,
          version: 1,
          server: "mock-advisor/1.0",
          capabilities: ["advisor"],
        });
        return;
      }
      requests.push(frame);
      onRequest(frame, send);
    },
    onData(listener) { onData = listener; },
    onClose(listener) { onClose = listener; },
    close() { onClose(); },
  };
  return { transport, send, requests };
}

// RuntimeKey's serde tag preserves acronym word boundaries; it is not the
// "openai-oauth" permission key. Keep the exact identity supplied by the server.
const subscription: AdvisorRouteSelection = {
  model: "gpt-5",
  runtime_key: { kind: "open-a-i-o-auth" },
  api_method: "openai-oauth",
  provider_label: "OpenAI",
};

test("advisor forwards every command, exact OAuth route, and effort", async (t) => {
  const commands: AdvisorRequest[] = [
    { action: "status" },
    { action: "inspect" },
    { action: "enable" },
    { action: "disable" },
    { action: "use_primary" },
    { action: "acknowledge", note_id: "adv-1" },
    { action: "dismiss", note_id: "adv-2" },
    { action: "model_options" },
    { action: "model_options", selection: null },
    { action: "model_options", selection: subscription },
    { action: "select_model", selection: subscription, reasoning_effort: "high" },
    { action: "select_model", selection: subscription, reasoning_effort: null },
    { action: "select_model", selection: subscription },
  ];
  const mock = mockTransport((request, send) => {
    assert.equal(request.req, "advisor");
    send({ ev: "advisor_result", reply_to: request.id, session_id: "s1", result: { message: "ok" } });
  });
  const client = await JcodeClient.connect({ transport: mock.transport });
  t.after(() => client.close());
  assert.equal(client.supports("advisor"), true);
  for (const request of commands) {
    assert.deepEqual(await client.advisor("s1", request), { message: "ok" });
  }
  assert.deepEqual(
    mock.requests.map(({ v, id, ...request }) => request),
    commands.map((request) => ({ req: "advisor", session_id: "s1", request })),
  );
});

test("advisor returns typed settings and model options without losing route identity", async (t) => {
  const result: AdvisorControlResult = {
    message: "Advisor model options",
    model_settings: {
      enabled: true,
      selection: subscription,
      reasoning_effort: "high",
      follows_primary: false,
    },
    model_options: {
      selection: subscription,
      reasoning_effort: "high",
      available_routes: [{
        model: "gpt-5",
        provider: "OpenAI",
        api_method: "openai-oauth",
        available: true,
        detail: "subscription",
      }],
      available_selections: [subscription],
      available_efforts: ["low", "medium", "high"],
    },
  };
  const mock = mockTransport((request, send) => {
    send({ ev: "advisor_result", reply_to: request.id, session_id: "s1", result });
  });
  const client = await JcodeClient.connect({ transport: mock.transport });
  t.after(() => client.close());
  const options = await client.advisor("s1", { action: "model_options" });
  assert.deepEqual(options, result);
  const selection = options.model_options?.available_selections?.[0];
  assert.ok(selection);
  await client.advisor("s1", { action: "model_options", selection });
  await client.advisor("s1", { action: "select_model", selection, reasoning_effort: "high" });
  assert.deepEqual(mock.requests[2], {
    v: 1,
    id: 4,
    req: "advisor",
    session_id: "s1",
    request: { action: "select_model", selection: subscription, reasoning_effort: "high" },
  });
  assert.equal(isKnownEvent({ ev: "advisor_result", session_id: "s1", result }), true);
});

test("advisor accepts older replies without selectable route metadata", async (t) => {
  const result: AdvisorControlResult = {
    message: "No routes",
    model_options: {
      selection: null,
      reasoning_effort: null,
      available_routes: [],
      available_efforts: [],
    },
  };
  const mock = mockTransport((request, send) => {
    send({ ev: "advisor_result", reply_to: request.id, session_id: "s1", result });
  });
  const client = await JcodeClient.connect({ transport: mock.transport });
  t.after(() => client.close());
  assert.deepEqual(await client.advisor("s1", { action: "model_options" }), result);
});

test("advisor replies stay correlated while a turn and other requests are active", async (t) => {
  const mock = mockTransport((request, send) => {
    if (request.req === "send_message") {
      send({ ev: "message_accepted", session_id: request.session_id });
    }
    if (request.req === "ping") send({ ev: "pong", reply_to: request.id });
  });
  const client = await JcodeClient.connect({ transport: mock.transport });
  t.after(() => client.close());
  const events: ApiEvent[] = [];
  const turn = client.run("s1", "Continue", { onEvent: (event) => events.push(event) });
  const status = client.advisor("s1", { action: "status" });
  const inspect = client.advisor("s1", { action: "inspect" });
  const advisorRequests = mock.requests.filter((request) => request.req === "advisor");
  assert.equal(advisorRequests.length, 2);
  mock.send({ ev: "text_delta", session_id: "s1", text: "Still " });
  mock.send({
    ev: "advisor_result",
    reply_to: advisorRequests[1].id,
    session_id: "s1",
    result: { message: "inspection" },
  });
  await client.ping();
  mock.send({ ev: "text_delta", session_id: "other", text: "wrong session" });
  mock.send({ ev: "text_delta", session_id: "s1", text: "working" });
  mock.send({
    ev: "advisor_result",
    reply_to: advisorRequests[0].id,
    session_id: "s1",
    result: { message: "status" },
  });
  mock.send({ ev: "turn_done", session_id: "s1" });
  assert.deepEqual(await status, { message: "status" });
  assert.deepEqual(await inspect, { message: "inspection" });
  assert.equal((await turn).text, "Still working");
  assert.deepEqual(events.map((event) => event.ev), [
    "message_accepted", "text_delta", "text_delta", "turn_done",
  ]);
});

test("advisor refusal preserves the resulting durable settings", async (t) => {
  const result: AdvisorControlResult = {
    message: "Advisor remains disabled",
    error: "Selected route is unavailable",
    model_settings: {
      enabled: false,
      selection: subscription,
      reasoning_effort: "high",
      follows_primary: false,
    },
  };
  const mock = mockTransport((request, send) => {
    send({ ev: "advisor_result", reply_to: request.id, session_id: "s1", result });
  });
  const client = await JcodeClient.connect({ transport: mock.transport });
  t.after(() => client.close());
  assert.deepEqual(await client.advisor("s1", { action: "enable" }), result);
});

test("advisor rejects harness failures and unexpected reply kinds", async (t) => {
  const replies: ApiEvent[] = [
    { ev: "error", code: "unknown_session", message: "No such session" },
    { ev: "ok" },
  ];
  const mock = mockTransport((request, send) => {
    const reply = replies.shift();
    assert.ok(reply);
    send({ ...reply, reply_to: request.id });
  });
  const client = await JcodeClient.connect({ transport: mock.transport });
  t.after(() => client.close());
  for (const code of ["unknown_session", "unexpected_reply"]) {
    await assert.rejects(client.advisor("s1", { action: "status" }), (error: unknown) => {
      assert.ok(error instanceof HarnessError);
      assert.equal(error.code, code);
      return true;
    });
  }
});

test("advisor rejects a transport failure while awaiting its reply", async (t) => {
  const mock = mockTransport(() => {});
  const client = await JcodeClient.connect({ transport: mock.transport });
  t.after(() => client.close());
  const request = client.advisor("s1", { action: "status" });
  const failure = assert.rejects(request, /harness connection closed/);
  await client.close();
  await failure;
});
