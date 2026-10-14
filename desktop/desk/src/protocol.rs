use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: u64,
    pub cmd: Command,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Permissions {
        #[serde(default)]
        prompt: bool,
    },
    Apps,
    Observe {
        app: String,
        #[serde(default)]
        screenshot: bool,
    },
    Act {
        app: String,
        actions: Vec<Action>,
        #[serde(default = "yes")]
        observe: bool,
        #[serde(default)]
        screenshot: bool,
    },
}

fn yes() -> bool {
    true
}

fn one() -> u8 {
    1
}

fn three() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Click {
        target: Target,
        #[serde(default)]
        button: Button,
        #[serde(default = "one")]
        count: u8,
    },
    TypeText {
        text: String,
    },
    PressKey {
        key: String,
        #[serde(default)]
        modifiers: Vec<Modifier>,
    },
    Scroll {
        #[serde(default)]
        target: Option<Target>,
        direction: Direction,
        #[serde(default = "three")]
        lines: u32,
    },
    Drag {
        from: Point,
        to: Point,
    },
    SetValue {
        index: u32,
        value: String,
    },
    Perform {
        index: u32,
        action: String,
    },
    Raise,
    Wait {
        ms: u32,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    Index(u32),
    Point(Point),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    #[default]
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modifier {
    Command,
    Shift,
    Option,
    Control,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub id: u64,
    pub reply: Reply,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    Permissions { accessibility: bool, screen_recording: bool, binary: String, help: String },
    Apps { apps: Vec<AppInfo> },
    Observation(Observation),
    Acted { completed: u32, failure: Option<Failure>, observation: Option<Observation> },
    Error(Failure),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppInfo {
    pub name: String,
    pub bundle_id: Option<String>,
    pub pid: i32,
    pub active: bool,
    pub access: Access,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    Allowed,
    NotListed,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub app: String,
    pub bundle_id: Option<String>,
    pub pid: i32,
    pub window: String,
    pub frame: Rect,
    pub elements: u32,
    pub truncated: bool,
    pub tree: String,
    pub screenshot: Option<Screenshot>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Screenshot {
    pub png_base64: String,
    pub width: u32,
    pub height: u32,
    pub pixels_per_point: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub code: ErrorCode,
    pub message: String,
    pub action: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    UnsupportedPlatform,
    PermissionMissing,
    AppNotFound,
    AppNotAllowed,
    AppBlocked,
    NoWindow,
    BadIndex,
    UnknownKey,
    ActionFailed,
}

impl Failure {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Failure { code, message: message.into(), action: None }
    }
}
