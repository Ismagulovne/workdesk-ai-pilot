//! Журнал действий: JSON Lines, один файл в день. Введённый текст по умолчанию не пишется.

use serde_json::{json, Value};
use std::io::Write;

pub fn write(action: &str, window: Option<Value>, args: Value, outcome: Result<&str, &str>) {
    let dir = crate::config::log_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let now = chrono::Local::now();
    let path = dir.join(format!("{}.jsonl", now.format("%Y-%m-%d")));
    let (ok, detail) = match outcome {
        Ok(d) => (true, d),
        Err(d) => (false, d),
    };
    let line = json!({
        "ts": now.to_rfc3339(),
        "action": action,
        "window": window,
        "args": args,
        "ok": ok,
        "detail": detail,
    });
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

pub fn save_png(png: &[u8]) -> Option<String> {
    let dir = crate::config::log_dir().join("shots");
    std::fs::create_dir_all(&dir).ok()?;
    let name = format!("{}.png", chrono::Local::now().format("%Y%m%d-%H%M%S%.3f"));
    std::fs::write(dir.join(&name), png).ok()?;
    Some(name)
}
