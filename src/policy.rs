//! Правила безопасности: запретные приложения, окна ввода секретов, номера карт и токены в тексте.

use crate::config::Config;

fn contains_any(hay: &str, needles: &[String]) -> Option<String> {
    let h = hay.to_lowercase();
    needles
        .iter()
        .find(|n| !n.is_empty() && h.contains(&n.to_lowercase()))
        .cloned()
}

/// Можно ли вообще выдавать окно ИИ.
pub fn window_allowed(cfg: &Config, app: &str, title: &str) -> Result<(), String> {
    if let Some(p) = contains_any(app, &cfg.deny_apps).or_else(|| contains_any(title, &cfg.deny_apps)) {
        return Err(format!("окно запрещено правилами (совпадение: «{p}»): менеджеры паролей и банковские приложения ИИ не выдаются"));
    }
    Ok(())
}

/// Можно ли вводить текст/клавиши в окно с таким заголовком.
pub fn typing_allowed(cfg: &Config, title: &str) -> Result<(), String> {
    if let Some(p) = contains_any(title, &cfg.secret_title_patterns) {
        return Err(format!(
            "ввод запрещён: заголовок окна похож на вход/оплату («{p}»). \
             Пароли, коды и платёжные данные вводит человек — попросите пользователя."
        ));
    }
    Ok(())
}

/// Нельзя вводить номера банковских карт и похожие на секреты строки.
pub fn text_allowed(text: &str) -> Result<(), String> {
    if has_card_number(text) {
        return Err("ввод запрещён: в тексте номер банковской карты. Платёжные данные вводит человек.".into());
    }
    let prefixes = ["sk-ant-", "sk-", "ghp_", "github_pat_", "gho_", "xoxb-", "xoxp-", "akia", "aiza", "-----begin"];
    for w in text.split_whitespace() {
        let lw = w.to_lowercase();
        if w.len() >= 20 && prefixes.iter().any(|p| lw.starts_with(p)) {
            return Err("ввод запрещён: текст похож на токен или ключ доступа. Секреты вводит человек.".into());
        }
    }
    Ok(())
}

fn has_card_number(text: &str) -> bool {
    // Ищем последовательности из 13–19 цифр (пробелы и дефисы внутри допускаются) с верной контрольной суммой Луна.
    let mut digits: Vec<u32> = Vec::new();
    let flush = |d: &mut Vec<u32>| -> bool {
        let ok = (13..=19).contains(&d.len()) && luhn(d);
        d.clear();
        ok
    };
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if let Some(v) = c.to_digit(10) {
            digits.push(v);
        } else if (*c == ' ' || *c == '-')
            && !digits.is_empty()
            && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit())
        {
            continue;
        } else if flush(&mut digits) {
            return true;
        }
    }
    flush(&mut digits)
}

fn luhn(d: &[u32]) -> bool {
    let sum: u32 = d
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &v)| if i % 2 == 1 { let x = v * 2; if x > 9 { x - 9 } else { x } } else { v })
        .sum();
    sum % 10 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards() {
        assert!(text_allowed("4111 1111 1111 1111").is_err());
        assert!(text_allowed("карта 4111-1111-1111-1111 срок").is_err());
        assert!(text_allowed("Счёт 2026-11-23, сумма 150000").is_ok());
        assert!(text_allowed("ИИН 900101300123").is_ok());
    }

    #[test]
    fn tokens() {
        assert!(text_allowed("ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_err());
        assert!(text_allowed("обычный текст письма").is_ok());
    }

    #[test]
    fn titles() {
        let c = Config::default();
        assert!(typing_allowed(&c, "Вход в Kaspi.kz").is_err());
        assert!(typing_allowed(&c, "Блокнот — отчёт.txt").is_ok());
        assert!(window_allowed(&c, "KeePassXC", "db.kdbx").is_err());
        assert!(window_allowed(&c, "1cv8", "1С:Предприятие").is_ok());
        assert!(window_allowed(&c, "Claude", "Claude").is_err());
    }
}
