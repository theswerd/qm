import { app } from "electron";
import { spawn } from "node:child_process";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { flyTo } from "./cursor.mjs";

const binary =
  process.env.QM_DESK_BIN ??
  (app.isPackaged
    ? path.join(process.resourcesPath, "desk")
    : path.join(path.dirname(fileURLToPath(import.meta.url)), "desk/target/release/desk"));
const paced = process.env.QM_DESK_CURSOR !== "0";
let child;
let pending;
let queue = Promise.resolve();
let next = 1;

function start() {
  if (child) return child;
  const proc = spawn(binary, ["serve"], { stdio: ["pipe", "pipe", "inherit"] });
  const fail = (error) => {
    if (child === proc) child = undefined;
    pending?.({ type: "error", code: "action_failed", message: `desk stopped: ${error.message}`, action: null });
  };
  proc.on("error", fail);
  proc.stdin.on("error", fail);
  proc.on("exit", (code, signal) => fail(new Error(`exit ${code ?? signal}`)));
  createInterface({ input: proc.stdout }).on("line", (line) => pending?.(JSON.parse(line).reply));
  child = proc;
  return proc;
}

function request(cmd) {
  const run = () =>
    new Promise((resolve) => {
      const proc = start();
      const id = next++;
      const send = (body) => proc.stdin.write(`${JSON.stringify({ id, cmd: body })}\n`);
      pending = (reply) => {
        if (reply.type !== "pointer") {
          pending = undefined;
          resolve(reply);
        } else {
          const arrived = () => send({ type: "arrived" });
          flyTo(reply.point).then(arrived, arrived);
        }
      };
      send(cmd);
    });
  const result = queue.then(run);
  queue = result;
  return result;
}

export const desk = {
  apps: () => request({ type: "apps" }),
  observe: (app, screenshot) => request({ type: "observe", app, screenshot }),
  act: (app, actions, screenshot) => request({ type: "act", app, actions, screenshot, pace: paced }),
};
