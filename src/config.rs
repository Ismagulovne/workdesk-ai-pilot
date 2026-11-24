//! Настройки: <папка настроек>/WorkDeskPilot/config.json (создаётся при первом запуске).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Приложения, для которых окно выдаётся без диалога согласия (по имени приложения).
    /// По умолчанию пусто: согласие спрашивается всегда.
    pub trusted_apps: Vec<String>,
    /// Приложения, к которым ИИ не допускается никогда (подстрока имени приложения или заголовка).
    pub deny_apps: Vec<String>,
    /// Если заголовок окна содержит одну из подстрок — ввод текста и клавиш запрещён
    /// (окна входа, оплаты, паролей).
    pub secret_title_patterns: Vec<String>,
    /// Ограничения снимка, которые отдаём модели.
    pub max_long_edge: u32,
    pub max_pixels: u32,
    /// Сохранять снимки в журнал (по умолчанию нет — в них могут быть персональные данные).
    pub save_screenshots: bool,
    /// Писать введённый текст в журнал (по умолчанию нет — только длину).
    pub log_typed_text: bool,
}

impl Default for Config {
    fn default() -> Self {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        Config {
            trusted_apps: vec![],
            deny_apps: s(&[
                "1password", "bitwarden", "keepass", "lastpass", "dashlane", "enpass",
                "keychain access", "связка ключей", "credential manager", "диспетчер учетных данных",
                "kaspi", "halyk", "homebank", "freedom bank", "jusan", "bcc.kz",
                "wdpilot", "workdesk ai pilot", "claude",
            ]),
            secret_title_patterns: s(&[
                "password", "пароль", "құпия сөз", "passcode", "pin-код", "пин-код",
                "sign in", "log in", "login", "вход в", "войти", "авторизац",
                "2fa", "two-factor", "одноразов", "otp", "cvv", "cvc",
                "payment", "checkout", "оплат", "банковск", "card number", "номер карты",
            ]),
            max_long_edge: 1568,
            max_pixels: 1_150_000,
            save_screenshots: false,
            log_typed_text: false,
        }
    }
}

pub fn app_dir() -> PathBuf {
    dirs::config_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("WorkDeskPilot")
}

pub fn log_dir() -> PathBuf {
    dirs::data_local_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("WorkDeskPilot")
        .join("logs")
}

impl Config {
    pub fn load() -> Config {
        let path = app_dir().join("config.json");
        let mut cfg = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                eprintln!("wdpilot: config.json не прочитан ({e}), беру настройки по умолчанию");
                Config::default()
            }),
            Err(_) => {
                let cfg = Config::default();
                let _ = std::fs::create_dir_all(app_dir());
                if let Ok(text) = serde_json::to_string_pretty(&cfg) {
                    let _ = std::fs::write(&path, text);
                }
                cfg
            }
        };
        // Обязательные запреты нельзя убрать настройками: ИИ не управляет ни самим Pilot,
        // ни чатом Claude (иначе мог бы сам нажать «Разрешить»).
        for m in MANDATORY_DENY {
            if !cfg.deny_apps.iter().any(|d| d.eq_ignore_ascii_case(m)) {
                cfg.deny_apps.push(m.to_string());
            }
        }
        cfg
    }
}

pub const MANDATORY_DENY: [&str; 3] = ["wdpilot", "workdesk ai pilot", "claude"];

#[cfg(test)]
mod tests {
    #[test]
    fn mandatory_in_default() {
        let c = super::Config::default();
        for m in super::MANDATORY_DENY {
            assert!(c.deny_apps.iter().any(|d| d == m));
        }
    }
}
