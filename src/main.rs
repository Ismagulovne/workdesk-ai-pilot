//! WorkDesk AI Pilot (wdpilot) — MCP-сервер (stdio, JSON-RPC 2.0).
//! Даёт ИИ-агенту (Claude Code, Claude Desktop или своему агенту на Claude API)
//! видеть ОДНО выбранное пользователем окно и управлять в нём мышью и клавиатурой.
//!
//! Автор и правообладатель: Нурболат Исмагулов.

mod config;
mod journal;
mod keys;
mod killswitch;
mod pilot;
mod platform;
mod ocr;
mod policy;
mod tools;

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

const PROTOCOL_FALLBACK: &str = "2025-06-18";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("wdpilot {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return;
    }
    if args.iter().any(|a| a == "--self-test") {
        // Проверка в CI: протокол MCP и описание инструментов без доступа к экрану.
        std::process::exit(tools::self_test());
    }

    platform::init();
    let cfg = config::Config::load();
    if args.iter().any(|a| a == "--list-windows") {
        // Отладка без MCP: какие окна видит wdpilot.
        match pilot::windows_json(&cfg) {
            Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default()),
            Err(e) => {
                eprintln!("wdpilot: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let stop = killswitch::start();
    let mut pilot = pilot::Pilot::new(cfg, stop);

    let stdin = io::stdin();
    let mut out = io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                send(&mut out, json!({"jsonrpc": "2.0", "id": null,
                    "error": {"code": -32700, "message": format!("parse error: {e}")}}));
                continue;
            }
        };
        if let Some(resp) = handle(Some(&mut pilot), &msg) {
            send(&mut out, resp);
        }
    }
}

fn send(out: &mut impl Write, v: Value) {
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

/// Обработка одного сообщения JSON-RPC. `pilot = None` — режим самопроверки без экрана.
pub fn handle(pilot: Option<&mut pilot::Pilot>, msg: &Value) -> Option<Value> {
    let id = msg.get("id")?.clone(); // без id — уведомление, ответ не нужен
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
    let result: Result<Value, (i64, String)> = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_FALLBACK),
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "wdpilot", "version": env!("CARGO_PKG_VERSION") },
            "instructions": tools::INSTRUCTIONS,
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::list() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match pilot {
                Some(p) => Ok(tools::call(p, name, &args)),
                None => Ok(tools::error_result("самопроверка: экран недоступен")),
            }
        }
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
    })
}

const HELP: &str = "wdpilot — WorkDesk AI Pilot, MCP-сервер управления одним окном.

Запуск как MCP-сервер (stdio):   wdpilot
Список окон (отладка):           wdpilot --list-windows
Проверка без экрана (CI):        wdpilot --self-test
Версия:                          wdpilot --version

Остановить ИИ в любой момент: Ctrl+Alt+Q или увести мышь в левый верхний угол экрана.
Настройки: config.json в папке настроек WorkDeskPilot; журнал действий — в папке logs.";
