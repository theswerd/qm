use crate::policy;
use crate::protocol::*;
use base64::Engine;
use core_foundation::array::CFArray;
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::event::{CGEvent, CGEventFlags, CGEventType, CGMouseButton, EventField, ScrollEventUnit};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use core_graphics::window::{
    create_image, kCGWindowImageBoundsIgnoreFraming, kCGWindowImageNominalResolution, kCGWindowListOptionIncludingWindow,
};
use foreign_types::ForeignType;
use objc2_app_kit::{NSApplicationActivationPolicy, NSWorkspace};
use std::collections::HashMap;
use std::ffi::c_void;
use std::time::Duration;

type El = CFType;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
    fn AXUIElementCopyAttributeValue(e: CFTypeRef, a: CFStringRef, v: *mut CFTypeRef) -> i32;
    fn AXUIElementSetAttributeValue(e: CFTypeRef, a: CFStringRef, v: CFTypeRef) -> i32;
    fn AXUIElementIsAttributeSettable(e: CFTypeRef, a: CFStringRef, v: *mut bool) -> i32;
    fn AXUIElementCopyActionNames(e: CFTypeRef, v: *mut CFTypeRef) -> i32;
    fn AXUIElementPerformAction(e: CFTypeRef, a: CFStringRef) -> i32;
    fn AXUIElementSetMessagingTimeout(e: CFTypeRef, t: f32) -> i32;
    fn AXValueGetValue(v: CFTypeRef, kind: u32, out: *mut c_void) -> bool;
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(o: CFTypeRef) -> bool;
    fn _AXUIElementGetWindow(e: CFTypeRef, id: *mut u32) -> i32;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGEventSetLocation(e: *mut c_void, p: CGPoint);
}

const MAX_NODES: usize = 800;
const MAX_DEPTH: usize = 60;
const BORING: &[&str] = &["AXShowMenu", "AXScrollToVisible", "AXShowDefaultUI", "AXShowAlternateUI"];

struct App {
    name: String,
    bundle: Option<String>,
    pid: i32,
    active: bool,
}

struct Snapshot {
    elements: Vec<El>,
    origin: (f64, f64),
}

#[derive(Default)]
pub struct Desk {
    cache: HashMap<i32, Snapshot>,
}

fn fail(code: ErrorCode, m: impl Into<String>) -> Failure {
    Failure::new(code, m)
}

fn cfs(s: &str) -> CFString {
    CFString::new(s)
}

fn attr(e: &El, name: &str) -> Option<CFType> {
    let mut v: CFTypeRef = std::ptr::null();
    let err = unsafe { AXUIElementCopyAttributeValue(e.as_CFTypeRef(), cfs(name).as_concrete_TypeRef(), &mut v) };
    (err == 0 && !v.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(v) })
}

fn text_of(v: &CFType) -> Option<String> {
    if let Some(s) = v.downcast::<CFString>() {
        return Some(s.to_string());
    }
    let n = v.downcast::<CFNumber>()?;
    n.to_i64().map(|i| i.to_string()).or_else(|| n.to_f64().map(|f| f.to_string()))
}

fn str_attr(e: &El, name: &str) -> Option<String> {
    attr(e, name).and_then(|v| text_of(&v)).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn bool_attr(e: &El, name: &str) -> Option<bool> {
    attr(e, name).and_then(|v| v.downcast::<CFBoolean>()).map(bool::from)
}

fn settable(e: &El, name: &str) -> bool {
    let mut b = false;
    unsafe { AXUIElementIsAttributeSettable(e.as_CFTypeRef(), cfs(name).as_concrete_TypeRef(), &mut b) == 0 && b }
}

fn ax_value<T: Default>(e: &El, name: &str, kind: u32) -> Option<T> {
    let v = attr(e, name)?;
    let mut out = T::default();
    unsafe { AXValueGetValue(v.as_CFTypeRef(), kind, &mut out as *mut T as *mut c_void) }.then_some(out)
}

fn frame(e: &El) -> Option<Rect> {
    let p: CGPoint = ax_value(e, "AXPosition", 1)?;
    let s: CGSize = ax_value(e, "AXSize", 2)?;
    Some(Rect { x: p.x, y: p.y, w: s.width, h: s.height })
}

fn list(v: CFType) -> Vec<CFType> {
    v.downcast::<CFArray>().map(|a| a.iter().map(|p| unsafe { CFType::wrap_under_get_rule(*p) }).collect()).unwrap_or_default()
}

fn actions(e: &El) -> Vec<String> {
    let mut v: CFTypeRef = std::ptr::null();
    if unsafe { AXUIElementCopyActionNames(e.as_CFTypeRef(), &mut v) } != 0 || v.is_null() {
        return vec![];
    }
    list(unsafe { CFType::wrap_under_create_rule(v) }).iter().filter_map(text_of).collect()
}

fn set_attr(e: &El, name: &str, v: &CFType) -> i32 {
    unsafe { AXUIElementSetAttributeValue(e.as_CFTypeRef(), cfs(name).as_concrete_TypeRef(), v.as_CFTypeRef()) }
}

fn perform(e: &El, action: &str) -> Result<(), Failure> {
    match unsafe { AXUIElementPerformAction(e.as_CFTypeRef(), cfs(action).as_concrete_TypeRef()) } {
        0 => Ok(()),
        n => Err(fail(ErrorCode::ActionFailed, format!("{action} failed (AXError {n})"))),
    }
}

fn clip(s: &str, n: usize) -> String {
    let s = s.replace('\n', "⏎");
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s
    }
}

fn running() -> Vec<App> {
    NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .filter(|a| a.activationPolicy() == NSApplicationActivationPolicy::Regular)
        .map(|a| App {
            name: a.localizedName().map(|s| s.to_string()).unwrap_or_default(),
            bundle: a.bundleIdentifier().map(|s| s.to_string()),
            pid: a.processIdentifier(),
            active: a.isActive(),
        })
        .collect()
}

fn pt(x: f64, y: f64) -> CGPoint {
    CGPoint::new(x, y)
}

fn source() -> Result<CGEventSource, Failure> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState).map_err(|_| fail(ErrorCode::ActionFailed, "no event source"))
}

fn post(e: CGEvent, pid: i32) {
    e.post_to_pid(pid);
    std::thread::sleep(Duration::from_millis(8));
}

fn mouse(pid: i32, win: u32, ty: CGEventType, p: CGPoint, b: CGMouseButton, clicks: i64) -> Result<(), Failure> {
    let e = CGEvent::new_mouse_event(source()?, ty, p, b).map_err(|_| fail(ErrorCode::ActionFailed, "mouse event"))?;
    e.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, clicks);
    e.set_integer_value_field(91, win as i64);
    e.set_integer_value_field(92, win as i64);
    post(e, pid);
    Ok(())
}

fn click_at(pid: i32, win: u32, p: CGPoint, button: Button, count: u8) -> Result<(), Failure> {
    let (down, up, b) = match button {
        Button::Left => (CGEventType::LeftMouseDown, CGEventType::LeftMouseUp, CGMouseButton::Left),
        Button::Right => (CGEventType::RightMouseDown, CGEventType::RightMouseUp, CGMouseButton::Right),
        Button::Middle => (CGEventType::OtherMouseDown, CGEventType::OtherMouseUp, CGMouseButton::Center),
    };
    for n in 1..=count.max(1) as i64 {
        mouse(pid, win, down, p, b, n)?;
        mouse(pid, win, up, p, b, n)?;
    }
    Ok(())
}

fn key(pid: i32, code: u16, flags: CGEventFlags, text: Option<&str>) -> Result<(), Failure> {
    for down in [true, false] {
        let e = CGEvent::new_keyboard_event(source()?, code, down).map_err(|_| fail(ErrorCode::ActionFailed, "key event"))?;
        e.set_flags(flags);
        if let Some(t) = text {
            e.set_string(t);
        }
        post(e, pid);
    }
    Ok(())
}

fn keycode(k: &str) -> Option<u16> {
    const KEYS: &[(&str, u16)] = &[
        ("a", 0),
        ("s", 1),
        ("d", 2),
        ("f", 3),
        ("h", 4),
        ("g", 5),
        ("z", 6),
        ("x", 7),
        ("c", 8),
        ("v", 9),
        ("b", 11),
        ("q", 12),
        ("w", 13),
        ("e", 14),
        ("r", 15),
        ("y", 16),
        ("t", 17),
        ("1", 18),
        ("2", 19),
        ("3", 20),
        ("4", 21),
        ("6", 22),
        ("5", 23),
        ("=", 24),
        ("9", 25),
        ("7", 26),
        ("-", 27),
        ("8", 28),
        ("0", 29),
        ("]", 30),
        ("o", 31),
        ("u", 32),
        ("[", 33),
        ("i", 34),
        ("p", 35),
        ("return", 36),
        ("enter", 36),
        ("l", 37),
        ("j", 38),
        ("'", 39),
        ("k", 40),
        (";", 41),
        ("\\", 42),
        (",", 43),
        ("/", 44),
        ("n", 45),
        ("m", 46),
        (".", 47),
        ("tab", 48),
        ("space", 49),
        ("`", 50),
        ("delete", 51),
        ("backspace", 51),
        ("escape", 53),
        ("esc", 53),
        ("f5", 96),
        ("f6", 97),
        ("f7", 98),
        ("f3", 99),
        ("f8", 100),
        ("f9", 101),
        ("f11", 103),
        ("f10", 109),
        ("f12", 111),
        ("home", 115),
        ("pageup", 116),
        ("forwarddelete", 117),
        ("f4", 118),
        ("end", 119),
        ("f2", 120),
        ("pagedown", 121),
        ("f1", 122),
        ("left", 123),
        ("right", 124),
        ("down", 125),
        ("up", 126),
    ];
    let k = k.to_ascii_lowercase();
    KEYS.iter().find(|(n, _)| *n == k).map(|(_, c)| *c)
}

fn flags(mods: &[Modifier]) -> CGEventFlags {
    mods.iter().fold(CGEventFlags::empty(), |f, m| {
        f | match m {
            Modifier::Command => CGEventFlags::CGEventFlagCommand,
            Modifier::Shift => CGEventFlags::CGEventFlagShift,
            Modifier::Option => CGEventFlags::CGEventFlagAlternate,
            Modifier::Control => CGEventFlags::CGEventFlagControl,
        }
    })
}

fn window_of(app: &El) -> Option<El> {
    attr(app, "AXFocusedWindow")
        .or_else(|| attr(app, "AXMainWindow"))
        .or_else(|| attr(app, "AXWindows").and_then(|w| list(w).into_iter().next()))
}

fn window_id(w: &El) -> u32 {
    let mut id = 0;
    unsafe { _AXUIElementGetWindow(w.as_CFTypeRef(), &mut id) };
    id
}

fn line(e: &El, origin: (f64, f64)) -> (String, bool) {
    let role = str_attr(e, "AXRole").unwrap_or_else(|| "AXUnknown".into());
    let title = str_attr(e, "AXTitle");
    let desc = str_attr(e, "AXDescription").or_else(|| str_attr(e, "AXHelp"));
    let value = str_attr(e, "AXValue");
    let can_set = settable(e, "AXValue");
    let acts: Vec<String> = actions(e).into_iter().filter(|a| !BORING.contains(&a.as_str())).collect();
    let keep = title.is_some() || desc.is_some() || value.is_some() || can_set || !acts.is_empty();
    let mut s = role.trim_start_matches("AX").to_string();
    if let Some(t) = &title {
        s += &format!(" \"{}\"", clip(t, 80));
    }
    if let Some(d) = desc.filter(|d| Some(d) != title.as_ref()) {
        s += &format!(" ({})", clip(&d, 80));
    }
    if let Some(v) = value.filter(|v| Some(v) != title.as_ref()) {
        s += &format!(" = \"{}\"", clip(&v, 120));
    }
    let mut tags = vec![];
    for (attr_name, want, tag) in [("AXFocused", true, "focused"), ("AXEnabled", false, "disabled"), ("AXSelected", true, "selected")] {
        if bool_attr(e, attr_name) == Some(want) {
            tags.push(tag);
        }
    }
    if can_set {
        tags.push("settable");
    }
    if !tags.is_empty() {
        s += &format!(" [{}]", tags.join(" "));
    }
    if !acts.is_empty() {
        let a: Vec<&str> = acts.iter().map(|a| a.trim_start_matches("AX")).collect();
        s += &format!(" <{}>", a.join(","));
    }
    if let Some(f) = frame(e) {
        s += &format!(" @{:.0},{:.0} {:.0}x{:.0}", f.x - origin.0, f.y - origin.1, f.w, f.h);
    }
    (s, keep)
}

struct Walk {
    origin: (f64, f64),
    elements: Vec<El>,
    tree: String,
    truncated: bool,
}

impl Walk {
    fn visit(&mut self, e: &El, depth: usize, indent: usize) {
        if self.elements.len() >= MAX_NODES || depth > MAX_DEPTH {
            self.truncated = true;
            return;
        }
        let (s, keep) = line(e, self.origin);
        let next = if keep {
            self.tree += &format!("{}[{}] {}\n", "  ".repeat(indent), self.elements.len(), s);
            self.elements.push(e.clone());
            indent + 1
        } else {
            indent
        };
        for c in attr(e, "AXChildren").map(list).unwrap_or_default() {
            self.visit(&c, depth + 1, next);
        }
    }
}

fn png(img: &core_graphics::image::CGImage) -> Option<(String, u32, u32)> {
    let (w, h, row) = (img.width(), img.height(), img.bytes_per_row());
    if img.bits_per_pixel() != 32 {
        return None;
    }
    let data = img.data();
    let bytes = data.bytes();
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let i = y * row + x * 4;
            rgba.extend_from_slice(&[bytes[i + 2], bytes[i + 1], bytes[i], bytes[i + 3]]);
        }
    }
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().ok()?.write_image_data(&rgba).ok()?;
    }
    Some((base64::engine::general_purpose::STANDARD.encode(buf), w as u32, h as u32))
}

fn permissions(prompt: bool) -> Reply {
    let accessibility = if prompt {
        let d = CFDictionary::from_CFType_pairs(&[(cfs("AXTrustedCheckOptionPrompt").as_CFType(), CFBoolean::true_value().as_CFType())]);
        unsafe { AXIsProcessTrustedWithOptions(d.as_CFTypeRef()) }
    } else {
        unsafe { AXIsProcessTrusted() }
    };
    let screen_recording = unsafe {
        if prompt {
            CGRequestScreenCaptureAccess()
        } else {
            CGPreflightScreenCaptureAccess()
        }
    };
    let binary = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    let help = "System Settings > Privacy & Security > Accessibility and > Screen Recording: add the app that launches desk \
(Terminal, iTerm, Ghostty, or your harness) with +, switch it on, then quit and reopen that app. \
`desk permissions --prompt` raises the system prompts."
        .to_string();
    Reply::Permissions { accessibility, screen_recording, binary, help }
}

impl Desk {
    pub fn handle(&mut self, cmd: Command) -> Reply {
        let r = match cmd {
            Command::Permissions { prompt } => Ok(permissions(prompt)),
            Command::Apps => apps(),
            Command::Observe { app, screenshot } => resolve(&app).and_then(|a| self.observe(&a, screenshot)).map(Reply::Observation),
            Command::Act { app, actions, observe, screenshot } => self.act(&app, &actions, observe, screenshot),
        };
        r.unwrap_or_else(Reply::Error)
    }

    fn observe(&mut self, app: &App, shot: bool) -> Result<Observation, Failure> {
        let (_, w) = window(app)?;
        let f = frame(&w).unwrap_or(Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
        let mut walk = Walk { origin: (f.x, f.y), elements: vec![], tree: String::new(), truncated: false };
        for c in attr(&w, "AXChildren").map(list).unwrap_or_default() {
            walk.visit(&c, 0, 0);
        }
        let screenshot = if shot { Some(capture(&w, f)?) } else { None };
        let obs = Observation {
            app: app.name.clone(),
            bundle_id: app.bundle.clone(),
            pid: app.pid,
            window: str_attr(&w, "AXTitle").unwrap_or_default(),
            frame: f,
            elements: walk.elements.len() as u32,
            truncated: walk.truncated,
            tree: walk.tree,
            screenshot,
        };
        self.cache.insert(app.pid, Snapshot { elements: walk.elements, origin: walk.origin });
        Ok(obs)
    }

    fn act(&mut self, query: &str, actions: &[Action], observe: bool, shot: bool) -> Result<Reply, Failure> {
        let app = resolve(query)?;
        if !self.cache.contains_key(&app.pid) {
            self.observe(&app, false)?;
        }
        let (ax, w) = window(&app)?;
        let win = window_id(&w);
        let mut completed = 0;
        let mut failure = None;
        for (i, a) in actions.iter().enumerate() {
            if let Err(mut e) = self.step(&app, &ax, &w, win, a) {
                e.action = Some(i as u32);
                failure = Some(e);
                break;
            }
            completed += 1;
        }
        std::thread::sleep(Duration::from_millis(250));
        let observation = if observe { Some(self.observe(&app, shot)?) } else { None };
        Ok(Reply::Acted { completed, failure, observation })
    }

    fn element(&self, pid: i32, index: u32) -> Result<El, Failure> {
        let s = self.cache.get(&pid).ok_or_else(|| fail(ErrorCode::BadIndex, "observe first"))?;
        s.elements.get(index as usize).cloned().ok_or_else(|| fail(ErrorCode::BadIndex, format!("no element {index}; observe again")))
    }

    fn point(&self, pid: i32, t: Target) -> Result<CGPoint, Failure> {
        match t {
            Target::Point(p) => {
                let o = self.cache.get(&pid).map(|s| s.origin).unwrap_or((0.0, 0.0));
                Ok(pt(o.0 + p.x, o.1 + p.y))
            }
            Target::Index(i) => {
                let f = frame(&self.element(pid, i)?).ok_or_else(|| fail(ErrorCode::BadIndex, format!("element {i} has no frame")))?;
                Ok(pt(f.x + f.w / 2.0, f.y + f.h / 2.0))
            }
        }
    }

    fn step(&self, app: &App, ax: &El, w: &El, win: u32, a: &Action) -> Result<(), Failure> {
        let pid = app.pid;
        match a {
            Action::Click { target: Target::Index(i), button: Button::Left, count: 1 } => {
                let e = self.element(pid, *i)?;
                if actions(&e).iter().any(|a| a == "AXPress") {
                    perform(&e, "AXPress")
                } else {
                    click_at(pid, win, self.point(pid, Target::Index(*i))?, Button::Left, 1)
                }
            }
            Action::Click { target, button, count } => click_at(pid, win, self.point(pid, *target)?, *button, *count),
            Action::TypeText { text } => {
                let chars: Vec<char> = text.chars().collect();
                for chunk in chars.chunks(16) {
                    key(pid, 0, CGEventFlags::empty(), Some(&chunk.iter().collect::<String>()))?;
                }
                Ok(())
            }
            Action::PressKey { key: k, modifiers } => {
                let code = keycode(k).ok_or_else(|| fail(ErrorCode::UnknownKey, format!("unknown key {k}")))?;
                key(pid, code, flags(modifiers), None)
            }
            Action::Scroll { target, direction, lines } => {
                let p = match target {
                    Some(t) => self.point(pid, *t)?,
                    None => frame(w).map(|f| pt(f.x + f.w / 2.0, f.y + f.h / 2.0)).unwrap_or(pt(0.0, 0.0)),
                };
                let n = *lines as i32;
                let (dy, dx) = match direction {
                    Direction::Up => (n, 0),
                    Direction::Down => (-n, 0),
                    Direction::Left => (0, n),
                    Direction::Right => (0, -n),
                };
                let e = CGEvent::new_scroll_event(source()?, ScrollEventUnit::LINE, 2, dy, dx, 0)
                    .map_err(|_| fail(ErrorCode::ActionFailed, "scroll event"))?;
                unsafe { CGEventSetLocation(e.as_ptr() as *mut c_void, p) };
                post(e, pid);
                Ok(())
            }
            Action::Drag { from, to } => {
                let (a, b) = (self.point(pid, Target::Point(*from))?, self.point(pid, Target::Point(*to))?);
                mouse(pid, win, CGEventType::LeftMouseDown, a, CGMouseButton::Left, 1)?;
                for s in 1..=12 {
                    let t = s as f64 / 12.0;
                    mouse(
                        pid,
                        win,
                        CGEventType::LeftMouseDragged,
                        pt(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t),
                        CGMouseButton::Left,
                        1,
                    )?;
                    std::thread::sleep(Duration::from_millis(12));
                }
                mouse(pid, win, CGEventType::LeftMouseUp, b, CGMouseButton::Left, 1)
            }
            Action::SetValue { index, value } => match set_attr(&self.element(pid, *index)?, "AXValue", &cfs(value).as_CFType()) {
                0 => Ok(()),
                n => Err(fail(ErrorCode::ActionFailed, format!("setting AXValue failed (AXError {n})"))),
            },
            Action::Perform { index, action } => {
                let name = if action.starts_with("AX") { action.clone() } else { format!("AX{action}") };
                perform(&self.element(pid, *index)?, &name)
            }
            Action::Raise => {
                set_attr(ax, "AXFrontmost", &CFBoolean::true_value().as_CFType());
                perform(w, "AXRaise")
            }
            Action::Wait { ms } => {
                std::thread::sleep(Duration::from_millis((*ms).min(10_000) as u64));
                Ok(())
            }
        }
    }
}

fn apps() -> Result<Reply, Failure> {
    let cfg = policy::load().map_err(|e| fail(ErrorCode::BadRequest, e))?;
    let apps = running()
        .into_iter()
        .map(|a| AppInfo {
            access: policy::access(&cfg, &a.name, a.bundle.as_deref()),
            name: a.name,
            bundle_id: a.bundle,
            pid: a.pid,
            active: a.active,
        })
        .collect();
    Ok(Reply::Apps { apps })
}

fn resolve(query: &str) -> Result<App, Failure> {
    if !unsafe { AXIsProcessTrusted() } {
        return Err(fail(ErrorCode::PermissionMissing, "Accessibility not granted; run `desk permissions --prompt`"));
    }
    let cfg = policy::load().map_err(|e| fail(ErrorCode::BadRequest, e))?;
    let app = running()
        .into_iter()
        .find(|a| a.name.eq_ignore_ascii_case(query) || a.bundle.as_deref().is_some_and(|b| b.eq_ignore_ascii_case(query)))
        .ok_or_else(|| fail(ErrorCode::AppNotFound, format!("no running app named {query}; open it first")))?;
    match policy::access(&cfg, &app.name, app.bundle.as_deref()) {
        Access::Allowed => Ok(app),
        Access::Blocked => Err(fail(ErrorCode::AppBlocked, format!("{} is on the built-in blocklist", app.name))),
        Access::NotListed => Err(fail(
            ErrorCode::AppNotAllowed,
            format!("{} is not allowlisted; the Mac's owner can run `desk allow {}`", app.name, app.bundle.as_deref().unwrap_or(&app.name)),
        )),
    }
}

fn window(app: &App) -> Result<(El, El), Failure> {
    let ax = unsafe { CFType::wrap_under_create_rule(AXUIElementCreateApplication(app.pid)) };
    unsafe { AXUIElementSetMessagingTimeout(ax.as_CFTypeRef(), 1.5) };
    set_attr(&ax, "AXManualAccessibility", &CFBoolean::true_value().as_CFType());
    let w = window_of(&ax).ok_or_else(|| fail(ErrorCode::NoWindow, format!("{} has no open window", app.name)))?;
    Ok((ax, w))
}

fn capture(w: &El, f: Rect) -> Result<Screenshot, Failure> {
    if !unsafe { CGPreflightScreenCaptureAccess() } {
        return Err(fail(ErrorCode::PermissionMissing, "Screen Recording not granted; run `desk permissions --prompt`"));
    }
    let rect = CGRect::new(&pt(f.x, f.y), &CGSize::new(f.w, f.h));
    let img = create_image(
        rect,
        kCGWindowListOptionIncludingWindow,
        window_id(w),
        kCGWindowImageBoundsIgnoreFraming | kCGWindowImageNominalResolution,
    )
    .ok_or_else(|| fail(ErrorCode::ActionFailed, "window capture returned nothing"))?;
    let (png_base64, width, height) = png(&img).ok_or_else(|| fail(ErrorCode::ActionFailed, "unexpected pixel format"))?;
    Ok(Screenshot { png_base64, width, height, pixels_per_point: if f.w > 0.0 { width as f64 / f.w } else { 1.0 } })
}
