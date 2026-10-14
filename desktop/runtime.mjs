import { BrowserWindow, ipcMain, session } from "electron";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { request } from "./desk.mjs";

const directory = path.dirname(fileURLToPath(import.meta.url));
const COMMANDS = new Set(["apps", "observe", "act"]);
let window;
let ready;
let fresh = true;
let queue = Promise.resolve();

export const tools = [
  {
    name: "ui__js",
    description:
      "Run JavaScript on the user's Mac to see and operate their apps. Code runs in a persistent session with top-level await; bindings survive between calls until ui__js_reset. A `computer` object drives apps; the first call in a session returns its API reference. Output from console.log and observation methods comes back as text and images. Pass a short `title` saying what the code does.",
    inputSchema: {
      type: "object",
      properties: {
        code: { type: "string", description: "JavaScript to run. Top-level await is allowed." },
        title: { type: "string", description: "A few words describing what this code does." },
        timeout_ms: { type: "integer", minimum: 1000, maximum: 55000, description: "Defaults to 30000." },
      },
      required: ["code"],
    },
    timeoutMs: 60000,
  },
  {
    name: "ui__js_reset",
    description:
      "Discard the ui__js session: all bindings and app handles. Use when it is wedged or state is confusing.",
    inputSchema: { type: "object", properties: {} },
    timeoutMs: 10000,
  },
];

ipcMain.handle("qm:runtime-desk", (event, cmd) => {
  if (!window || window.isDestroyed() || event.sender !== window.webContents) throw new Error("Untrusted desk request");
  if (!COMMANDS.has(cmd?.type)) throw new Error("Unsupported desk command");
  return request(cmd);
});

const page = path.join(directory, "runtime.html");

function open() {
  if (window && !window.isDestroyed()) return ready;
  const isolated = session.fromPartition("qm-computer");
  isolated.webRequest.onBeforeRequest((details, callback) => callback({ cancel: !details.url.startsWith("file:") }));
  isolated.setPermissionRequestHandler((_contents, _permission, callback) => callback(false));
  window = new BrowserWindow({
    show: false,
    webPreferences: {
      session: isolated,
      preload: path.join(directory, "runtime-preload.cjs"),
      nodeIntegration: false,
      contextIsolation: true,
      sandbox: true,
      backgroundThrottling: false,
    },
  });
  window.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
  window.webContents.on("will-navigate", (event) => event.preventDefault());
  window.webContents.debugger.attach("1.3");
  fresh = true;
  ready = window.loadFile(page);
  return ready;
}

function reset() {
  if (!window || window.isDestroyed()) return;
  window.webContents.debugger.sendCommand("Runtime.terminateExecution").catch(() => {});
  fresh = true;
  ready = window.loadFile(page);
}

async function shown(cdp, result) {
  if (result.objectId) {
    const described = await cdp("Runtime.callFunctionOn", {
      objectId: result.objectId,
      functionDeclaration: "function () { return __describe(this); }",
      returnByValue: true,
    });
    return described.result.value;
  }
  return result.type === "undefined" ? undefined : (result.description ?? String(result.value));
}

async function evaluate(code, timeoutMs) {
  await open();
  const cdp = (method, params) => window.webContents.debugger.sendCommand(method, params);
  let timer;
  const expired = new Promise((resolve) => (timer = setTimeout(() => resolve("timeout"), timeoutMs)));
  const outcome = await Promise.race([
    cdp("Runtime.evaluate", { expression: code, replMode: true, awaitPromise: true }),
    expired,
  ]).catch((error) => ({ exceptionDetails: { text: error.message } }));
  clearTimeout(timer);
  if (outcome === "timeout") {
    reset();
    return { content: `[timed out after ${timeoutMs} ms; the session was reset]`, isError: true };
  }
  const value = outcome.exceptionDetails ? undefined : await shown(cdp, outcome.result);
  const { text, images } = (await cdp("Runtime.evaluate", { expression: "__drain()", returnByValue: true })).result
    .value;
  const error = outcome.exceptionDetails
    ? (outcome.exceptionDetails.exception?.description ?? outcome.exceptionDetails.text)
    : undefined;
  const docs = fresh ? (await cdp("Runtime.evaluate", { expression: "__docs", returnByValue: true })).result.value : "";
  fresh = false;
  const content = [docs, text, value === undefined ? "" : `=> ${value}`, error ? `[error] ${error}` : ""]
    .filter(Boolean)
    .join("\n\n");
  return {
    content: content || "[no output]",
    ...(images.length ? { images } : {}),
    ...(error ? { isError: true } : {}),
  };
}

export function call(name, args) {
  const job = queue.then(() => {
    if (name === "ui__js_reset") {
      reset();
      return { content: "[session reset]" };
    }
    if (name !== "ui__js" || typeof args?.code !== "string") return { content: "[error] unknown call", isError: true };
    const timeout = Number.isInteger(args.timeout_ms) ? Math.min(Math.max(args.timeout_ms, 1000), 55000) : 30000;
    return evaluate(args.code, timeout);
  });
  queue = job.catch(() => {});
  return job;
}
