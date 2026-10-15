import assert from "node:assert/strict";
import { test } from "node:test";
import { resolveSandbox } from "../scripts/dev/lib/sandbox.ts";

const resolve = (worktree: string, env: Record<string, string> = {}) =>
  resolveSandbox({
    worktree,
    requested: "freestyle",
    corePort: 8080,
    lock: "/tmp/qm-dev-freestyle-test",
    baseEnv: {
      FREESTYLE_API_KEY: "test-key",
      PUBLIC_API_URL: "https://core.example.com",
      ...env,
    },
    log() {},
  });

test("Freestyle dev configuration passes the key and preserves a stable worktree namespace", async () => {
  const first = await resolve("/worktrees/freestyle-a");
  const restarted = await resolve("/worktrees/freestyle-a");
  const other = await resolve("/worktrees/freestyle-b");
  assert.equal(first.backend, "freestyle");
  assert.equal(first.env.SANDBOX_BACKEND, "freestyle");
  assert.equal(first.env.FREESTYLE_API_KEY, "test-key");
  assert.match(first.env.FREESTYLE_NAME_PREFIX ?? "", /^qmdev-[a-f0-9]{12}$/);
  assert.equal(restarted.env.FREESTYLE_NAME_PREFIX, first.env.FREESTYLE_NAME_PREFIX);
  assert.notEqual(other.env.FREESTYLE_NAME_PREFIX, first.env.FREESTYLE_NAME_PREFIX);
});

test("Freestyle dev configuration accepts a custom snapshot and rejects blank credentials", async () => {
  const result = await resolve("/worktrees/freestyle-a", {
    FREESTYLE_SNAPSHOT_ID: "custom-snapshot",
    FREESTYLE_NAME_PREFIX: "my-dev-boxes",
    FREESTYLE_API_KEY: " test-key ",
  });
  assert.equal(result.env.FREESTYLE_SNAPSHOT_ID, "custom-snapshot");
  assert.equal(result.env.FREESTYLE_NAME_PREFIX, "my-dev-boxes");
  assert.equal(result.env.FREESTYLE_API_KEY, "test-key");
  await assert.rejects(resolve("/worktrees/freestyle-a", { FREESTYLE_API_KEY: "  " }), /FREESTYLE_API_KEY/);
});
