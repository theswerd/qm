import { errMessage } from "../../chassis/src/errors.ts";

interface DesktopComputer {
  tools(): Promise<Array<{ name: string }>>;
  call(callId: string, name: string, args: unknown): Promise<unknown>;
}

interface Activity {
  type: string;
  payload: unknown;
}

const computer = (globalThis as { qmDesktop?: { computer?: DesktopComputer } }).qmDesktop?.computer;
let declared: Array<{ name: string }> = [];
const started = new Set<string>();
if (computer)
  void computer.tools().then(
    (tools) => {
      declared = tools;
    },
    () => {},
  );

export function desktopTools(): { clientTools?: Array<{ name: string }> } {
  return declared.length ? { clientTools: declared } : {};
}

export function runDesktopCalls(
  activity: readonly Activity[],
  send: (callId: string, result: unknown) => Promise<unknown>,
): void {
  if (!computer) return;
  const answered = new Set(
    activity.filter((a) => a.type === "tool_result").map((a) => (a.payload as { callId?: unknown } | null)?.callId),
  );
  for (const a of activity) {
    const call = a.payload as { tool?: unknown; client?: unknown; callId?: unknown; args?: unknown } | null;
    if (a.type !== "tool_call" || call?.client !== true || typeof call.callId !== "string") continue;
    const { callId, tool } = call;
    if (answered.has(callId) || started.has(callId) || !declared.some((d) => d.name === tool)) continue;
    started.add(callId);
    void computer
      .call(callId, tool as string, call.args)
      .catch((error: unknown) => ({ content: `[error] ${errMessage(error)}`, isError: true }))
      .then((result) => send(callId, result))
      .catch(() => {});
  }
}
