import { reportHandledError } from "./browser-errors.ts";
import { fetchUiState, putUiState } from "./core-bridge";

const PIN_LATEST_PROMPT_KEY = "pin-latest-prompt";
const ALWAYS_STEER_KEY = "always-steer";

type ChatPrefKey = typeof PIN_LATEST_PROMPT_KEY | typeof ALWAYS_STEER_KEY;

const DEFAULTS: Record<ChatPrefKey, boolean> = {
  [PIN_LATEST_PROMPT_KEY]: true,
  [ALWAYS_STEER_KEY]: false,
};

// The server copy is authoritative; this browser copy only keeps the first paint from
// flashing the default before the server answers.
const CACHE_PREFIX = "chat-pref:";

const values = new Map<ChatPrefKey, boolean>();
const touched = new Set<ChatPrefKey>();
const listeners = new Set<() => void>();

function cached(key: ChatPrefKey): boolean {
  try {
    const raw = localStorage.getItem(CACHE_PREFIX + key);
    if (raw === "1") return true;
    if (raw === "0") return false;
  } catch {
    void 0;
  }
  return DEFAULTS[key];
}

function read(key: ChatPrefKey): boolean {
  let value = values.get(key);
  if (value === undefined) {
    value = cached(key);
    values.set(key, value);
  }
  return value;
}

function apply(key: ChatPrefKey, value: boolean): void {
  const changed = read(key) !== value;
  values.set(key, value);
  try {
    if (value === DEFAULTS[key]) localStorage.removeItem(CACHE_PREFIX + key);
    else localStorage.setItem(CACHE_PREFIX + key, value ? "1" : "0");
  } catch {
    void 0;
  }
  if (changed) for (const listener of listeners) listener();
}

function write(key: ChatPrefKey, value: boolean): void {
  touched.add(key);
  apply(key, value);
  void putUiState(key, value, Date.now()).catch((error: unknown) => reportHandledError("web:save_chat_pref", error));
}

export function pinLatestPrompt(): boolean {
  return read(PIN_LATEST_PROMPT_KEY);
}

export function alwaysSteer(): boolean {
  return read(ALWAYS_STEER_KEY);
}

export function setPinLatestPrompt(value: boolean): void {
  write(PIN_LATEST_PROMPT_KEY, value);
}

export function setAlwaysSteer(value: boolean): void {
  write(ALWAYS_STEER_KEY, value);
}

export function onChatPrefsChange(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export async function loadChatPrefs(): Promise<void> {
  await Promise.all(
    (Object.keys(DEFAULTS) as ChatPrefKey[]).map(async (key) => {
      try {
        const rec = await fetchUiState(key);
        if (touched.has(key)) return;
        apply(key, typeof rec.value === "boolean" ? rec.value : DEFAULTS[key]);
      } catch (error) {
        reportHandledError("web:load_chat_pref", error);
      }
    }),
  );
}
