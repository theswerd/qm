# desk

A small Rust program that lets an agent see and drive Mac apps: list running apps, read a window's accessibility tree as numbered elements (plus an optional screenshot), and run batches of clicks, typing, keys, scrolls and drags. It speaks JSON lines on stdio so crystal (or any harness) can drive it; see `PROTOCOL.md`. It copies the *shape* of Codex's Computer Use: AX tree first, indexed targets, events posted to the target pid, a per-app allowlist. No Codex code or prompt text is used.

**Status: written and type-checked on Linux against the macOS targets; never run on a Mac.** Expect a round of fixes the first time you run it.

## 1. Install Rust

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
xcode-select --install    # if you don't have the command line tools yet
```

## 2. Build

```sh
tar xzf crystal-desk.tar.gz && cd desk
cargo build --release
./target/release/desk            # prints usage
```

## 3. Grant permissions

macOS grants Accessibility and Screen Recording to the app that *launches* desk, so grant your terminal app (Terminal, iTerm, Ghostty...).

```sh
./target/release/desk permissions --prompt
```

Then open System Settings > Privacy & Security, and in both **Accessibility** and **Screen Recording** switch on your terminal app (add it with + if it isn't listed). Quit the terminal completely (Cmd-Q) and reopen it. Check:

```sh
./target/release/desk permissions     # want "accessibility": true, "screen_recording": true
```

Screen Recording is only needed for `--png` and screenshots; the AX tree and actions need only Accessibility.

## 4. Try it

```sh
open -a Calculator
./target/release/desk apps                          # Calculator should show Allowed
./target/release/desk observe Calculator --png /tmp/calc.png
```

You get lines like `[12] Button (7) <Press> @20,180 40x40`. To click "7" by index, use its number:

```sh
./target/release/desk act Calculator '[{"type":"click","target":{"index":12}}]'
```

The display should read 7, and the command prints a fresh tree. More:

```sh
./target/release/desk act Calculator '[{"type":"press_key","key":"escape"},{"type":"type_text","text":"12*34="}]'
./target/release/desk act Calculator '[{"type":"press_key","key":"c","modifiers":["command"]}]'   # copy the result
```

Only Calculator and TextEdit are allowed out of the box. To add an app (blocklisted ones like Terminal, Keychain Access, password managers and System Settings can't be added):

```sh
./target/release/desk allow com.apple.Notes
```

## 5. Safe demo script

`./demo.sh` opens Calculator, prints permissions and the tree, saves a screenshot to `/tmp/desk-calculator.png`, clicks 7 and + by index, types `5=`, and expects 12. It then sends two JSON lines through `desk serve`, and the second is refused because Terminal is blocked. It touches nothing but Calculator.

## 6. Driving it from a harness

```sh
./target/release/desk serve
{"id":1,"cmd":{"type":"apps"}}
```

One request per line in, one response per line out. See `PROTOCOL.md` for types, the three crystal tools, and how a cloud crystal would reach this Mac (the Mac dials out over WebSocket).

## What is unverified until it runs on a Mac

- **Events posted to a background app.** `CGEventPostToPid` clicks and keys might be ignored, or land in the wrong window, when the app isn't frontmost. I set the window-under-pointer fields (91/92), but that is a guess at what Codex does. If nothing happens, add `{"type":"raise"}` first; that does take focus.
- **Calculator's tree shape.** The demo greps for a button labelled 7 / +. macOS 14 and 15 Calculators may label buttons differently ("Add" vs "+").
- **Screenshots.** They use `CGWindowListCreateImage`, which Apple deprecated in macOS 14 in favour of ScreenCaptureKit. It still links, but on macOS 15+ it may return nothing or show a privacy prompt every month. The fix is a ScreenCaptureKit path. The `pixels_per_point` scale also needs checking on a Retina screen.
- **`_AXUIElementGetWindow`** (private, widely used) maps the AX window to a window id for capture and event targeting.
- **`AXManualAccessibility`** is set on each app so Electron and Chrome apps expose their trees. Harmless elsewhere, untested.
- **Speed.** Each element costs about 7 AX round trips, so big windows (Safari, Xcode) may take seconds. The 800-element cap bounds this.
- **x86_64** also type-checks; universal binary: `cargo build --release --target aarch64-apple-darwin --target x86_64-apple-darwin` then `lipo`.
