mod policy;
mod protocol;

#[cfg(target_os = "macos")]
mod mac;

#[cfg(not(target_os = "macos"))]
mod mac {
    use crate::protocol::{Command, ErrorCode, Failure, Reply};
    #[derive(Default)]
    pub struct Desk(());
    impl Desk {
        pub fn handle(&mut self, _: Command) -> Reply {
            Reply::Error(Failure::new(ErrorCode::UnsupportedPlatform, "desk only drives macOS"))
        }
    }
}

use base64::Engine;
use protocol::*;
use std::io::{BufRead, Write};

const USAGE: &str = "usage:
  desk permissions [--prompt]
  desk apps
  desk observe <app> [--png out.png]
  desk act <app> '<actions json>' [--png out.png]
  desk allow <app name or bundle id>
  desk serve            JSON lines on stdin/stdout (see PROTOCOL.md)";

fn main() {
    if let Err(e) = run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let mut desk = mac::Desk::default();
    let arg = |i: usize| args.get(i).cloned().ok_or_else(|| USAGE.to_string());
    let png = args.iter().position(|a| a == "--png").and_then(|i| args.get(i + 1).cloned());
    let cmd = match args.first().map(String::as_str) {
        Some("serve") => return serve(&mut desk),
        Some("allow") => return allow(&arg(1)?),
        Some("permissions") => Command::Permissions { prompt: args.iter().any(|a| a == "--prompt") },
        Some("apps") => Command::Apps,
        Some("observe") => Command::Observe { app: arg(1)?, screenshot: png.is_some() },
        Some("act") => Command::Act {
            app: arg(1)?,
            actions: serde_json::from_str(&arg(2)?).map_err(|e| format!("bad actions json: {e}"))?,
            observe: true,
            screenshot: png.is_some(),
        },
        _ => return Err(USAGE.into()),
    };
    print(desk.handle(cmd), png.as_deref())
}

fn show(o: &Observation, png: Option<&str>) -> Result<(), String> {
    let cut = if o.truncated { " (truncated)" } else { "" };
    let f = o.frame;
    println!("{} \"{}\" pid {} @{:.0},{:.0} {:.0}x{:.0}, {} elements{cut}", o.app, o.window, o.pid, f.x, f.y, f.w, f.h, o.elements);
    print!("{}", o.tree);
    if let (Some(s), Some(path)) = (&o.screenshot, png) {
        let bytes = base64::engine::general_purpose::STANDARD.decode(&s.png_base64).map_err(|e| e.to_string())?;
        std::fs::write(path, bytes).map_err(|e| e.to_string())?;
        println!("screenshot {}x{} ({} px per point) -> {path}", s.width, s.height, s.pixels_per_point);
    }
    Ok(())
}

fn print(reply: Reply, png: Option<&str>) -> Result<(), String> {
    match &reply {
        Reply::Observation(o) => show(o, png),
        Reply::Acted { completed, failure, observation } => {
            println!("completed {completed} action(s)");
            if let Some(f) = failure {
                println!("action {} failed: {:?}: {}", f.action.unwrap_or(0), f.code, f.message);
            }
            observation.as_ref().map_or(Ok(()), |o| show(o, png))
        }
        Reply::Apps { apps } => {
            for a in apps {
                let active = if a.active { " (active)" } else { "" };
                println!("{:<28} {:<40} pid {:<6} {:?}{active}", a.name, a.bundle_id.as_deref().unwrap_or("-"), a.pid, a.access);
            }
            Ok(())
        }
        Reply::Permissions { .. } => serde_json::to_string_pretty(&reply).map(|s| println!("{s}")).map_err(|e| e.to_string()),
        Reply::Error(f) => Err(format!("{:?}: {}", f.code, f.message)),
    }
}

fn allow(app: &str) -> Result<(), String> {
    let mut c = policy::load()?;
    if policy::access(&c, app, Some(app)) == Access::Blocked {
        return Err(format!("{app} is on the built-in blocklist"));
    }
    if !c.allow.iter().any(|a| a.eq_ignore_ascii_case(app)) {
        c.allow.push(app.to_string());
    }
    policy::save(&c)?;
    println!("allowed {app} in {}", policy::path().display());
    Ok(())
}

fn serve(desk: &mut mac::Desk) -> Result<(), String> {
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<Request>(&line) {
            Ok(r) => Response { id: r.id, reply: desk.handle(r.cmd) },
            Err(e) => Response { id: 0, reply: Reply::Error(Failure::new(ErrorCode::BadRequest, e.to_string())) },
        };
        let s = serde_json::to_string(&resp).map_err(|e| e.to_string())?;
        writeln!(out, "{s}").and_then(|_| out.flush()).map_err(|e| e.to_string())?;
    }
    Ok(())
}
