import "./support/auto-fake-sprites.ts";
import { mock, test } from "node:test";
import assert from "node:assert/strict";
import * as mockHarness from "../src/harness/mock-harness.ts";
import { testConfig } from "./support/test-config.ts";

let started!: () => void;
let release!: () => void;
mock.module("../src/harness/mock-harness.ts", {
  namedExports: {
    ...mockHarness,
    createMockHarness: () => {
      const harness = mockHarness.createMockHarness();
      harness.turns.runTurn = async () => {
        started();
        await new Promise<void>((resolve) => {
          release = resolve;
        });
        return { reply: "Done" };
      };
      return harness;
    },
  },
});
const { buildApp } = await import("../src/wiring.ts");

test("a run reports the runtime its turn runs on while working and after it finishes", async () => {
  const built = buildApp(testConfig({ workers: 1 }));
  built.runtime.start();
  try {
    for (const model of [undefined, "claude-haiku-5-5"]) {
      const ready = new Promise<void>((resolve) => {
        started = resolve;
      });
      const queued = await built.app.turn({
        surface: "web",
        liveActor: true,
        actor: { externalId: "U1" },
        conversation: { kind: "dm", threadRef: `runtime-${model ?? "default"}` },
        text: "hello",
        async: true,
        ...(model ? { model } : {}),
      });
      await ready;
      const live = (await built.app.getRun(queued.runId!))?.runtime;
      assert.ok(live?.modelId);
      if (model) assert.equal(live.modelId, model);
      assert.equal(live.routedBy, undefined);
      release();
      await built.runs.waitFor(queued.runId!, 5000);
      assert.deepEqual((await built.runs.get(queued.runId!))?.result?.runtime, live);
    }
  } finally {
    release?.();
    await built.runtime.stop();
  }
});
