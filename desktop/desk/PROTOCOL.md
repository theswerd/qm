# desk protocol

`desk serve` reads one JSON request per line on stdin and writes one JSON response per line on stdout, in order. Requests run one at a time. Every type is a closed serde type with `deny_unknown_fields`, tagged by `"type"`. Source of truth: `src/protocol.rs`.

## Requests

```
Request  { id: u64, cmd: Command }

Command  = permissions { prompt?: bool = false }
         | apps
         | observe     { app: string, screenshot?: bool = false }
         | act         { app: string, actions: [Action], observe?: bool = true, screenshot?: bool = false }

Action   = click       { target: Target, button?: left|right|middle = left, count?: u8 = 1 }
         | type_text   { text: string }
         | press_key   { key: string, modifiers?: [command|shift|option|control] }
         | scroll      { target?: Target, direction: up|down|left|right, lines?: u32 = 3 }
         | drag        { from: Point, to: Point }
         | set_value   { index: u32, value: string }
         | perform     { index: u32, action: string }        // any AX action the element lists, e.g. "ShowMenu", "Increment"
         | raise                                             // bring the app's window to the front (steals focus)
         | wait        { ms: u32 }                            // capped at 10000

Target   = { "index": u32 } | { "point": Point }
Point    = { x: f64, y: f64 }
```

`app` matches a running app's name or bundle id, case-insensitive. `key` is a letter, digit, punctuation, `return`, `tab`, `space`, `delete`, `escape`, `forwarddelete`, `home`, `end`, `pageup`, `pagedown`, arrows (`left` …), or `f1`–`f12`.

Example:

```json
{"id":1,"cmd":{"type":"observe","app":"Calculator"}}
{"id":2,"cmd":{"type":"act","app":"Calculator","actions":[{"type":"click","target":{"index":12}},{"type":"type_text","text":"*3="}]}}
```

## Responses

```
Response { id: u64, reply: Reply }          // id 0 when the line could not be parsed

Reply    = permissions { accessibility: bool, screen_recording: bool, binary: string, help: string }
         | apps        { apps: [AppInfo] }
         | observation { ...Observation }
         | acted       { completed: u32, failure: Failure?, observation: Observation? }
         | error       { ...Failure }

AppInfo     { name, bundle_id?, pid: i32, active: bool, access: allowed|not_listed|blocked }
Observation { app, bundle_id?, pid, window: string, frame: Rect, elements: u32, truncated: bool,
              tree: string, screenshot: Screenshot? }
Rect        { x, y, w, h }                   // global screen points, top-left origin
Screenshot  { png_base64, width, height, pixels_per_point }
Failure     { code: ErrorCode, message: string, action: u32? }   // action = index of the failed action in a batch
ErrorCode   = bad_request | unsupported_platform | permission_missing | app_not_found | app_not_allowed
            | app_blocked | no_window | bad_index | unknown_key | action_failed
```

## Observation semantics

- The tree covers the app's focused window (else main, else first). One line per kept element:
  `[index] Role "title" (description) = "value" [focused disabled selected settable] <Actions> @x,y wxh`.
  Unlabelled containers are folded away; indentation shows nesting. Capped at 800 elements (`truncated: true`).
- Frames and `point` targets are **window-relative points** (origin = window top-left).
  The screenshot is captured at nominal resolution, so one PNG pixel is about one point; `pixels_per_point` says what it really was.
  Divide PNG pixels by `pixels_per_point` before sending a `point`.
- Indices refer to the most recent observation of that app in this `serve` session. `act` returns a fresh observation (after a 250 ms settle), and new indices replace the old ones.
- A batch stops at the first failed action. `acted.completed` counts what ran; the observation still comes back.

## How actions are delivered

- `click` on an index, left button, single: `AXPress` if the element offers it (no pointer, no focus change); otherwise a synthetic click at the element's center.
- Other clicks, `drag`, `scroll`, `type_text`, `press_key`: CGEvents posted to the app's pid (`CGEventPostToPid`). The real pointer does not move and the app is not brought forward. Some apps ignore events while in the background; then send `raise` first.
- `set_value` writes `AXValue`. `perform` runs any listed AX action.

## Policy

`~/.config/desk/config.json` (or `$DESK_CONFIG`): `{"allow": ["com.apple.calculator", "TextEdit"]}`. With no file, Calculator and TextEdit are allowed.
Entries match name or bundle id. A built-in blocklist always wins: terminals, Keychain Access, Passwords, System Settings, Script Editor, Automator, 1Password, Bitwarden, LastPass, Dashlane, Codex/ChatGPT.
Only the Mac's owner edits this file (`desk allow <app>`); there is no protocol command to widen it.

## Tools crystal would register

Thin typed wrappers, one request each:

| tool | input | maps to |
|---|---|---|
| `computer_list` | `{desk_id}` | `apps` |
| `computer_observe` | `{desk_id, app, screenshot?}` | `observe` |
| `computer_act` | `{desk_id, app, actions: [Action], screenshot?}` | `act` (observe on) |

Crystal's approval layer asks once per `(session, bundle_id)` before the first `observe`/`act` on an app ("Allow the agent to use Calculator?"). desk's allowlist is the floor under that: crystal can't approve its way past the owner's local list. The screenshot goes to blobs and the model sees it as an image item. `tree` goes in as text.

## Reaching a Mac from crystal

crystal's tools run in the cloud. Regan's Mac sits behind NAT, so the Mac has to dial out.

1. **Local stdio (works now).** A harness on the same Mac spawns `desk serve` and writes/reads lines.
2. **Desk dials out (proposed).** The desk opens a WebSocket to crystal (`wss://<crystal>/v1/desks/connect`) with a device token minted in the crystal UI, and carries the same JSON lines as text frames, unchanged. Crystal core keeps one live connection per desk and routes `computer_*` calls to it by `desk_id`. Disconnect means the tools fail fast with `desk offline`. Outbound-only: no inbound port, no tunnel. The token is revocable, and the allowlist stays on the Mac.
   Zero-code version to try today, with any WebSocket endpoint that speaks these lines:
   `websocat -t --exit-on-eof "wss://<crystal>/v1/desks/connect?token=$DESK_TOKEN" sh-c:'desk serve'`
   A native `desk connect <url>` subcommand (tokio-tungstenite, reconnect with backoff) is about 60 lines when crystal has the endpoint.
3. **Not recommended:** exposing `desk serve` on a port or tunnel. It is an inbound attack surface on a machine with Accessibility rights.
