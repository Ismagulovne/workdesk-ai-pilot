//! Разбор клавиш и сочетаний в формате Computer Use («ctrl+s», «Return», «super»).

use enigo::Key;

pub fn parse_combo(text: &str) -> Result<Vec<Key>, String> {
    let parts: Vec<&str> = text.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        // «+» как самостоятельная клавиша
        if text.trim() == "+" {
            return Ok(vec![Key::Unicode('+')]);
        }
        return Err("пустое сочетание клавиш".into());
    }
    parts.iter().map(|p| parse_key(p)).collect()
}

pub fn parse_key(name: &str) -> Result<Key, String> {
    let n = name.trim().to_lowercase();
    let k = match n.as_str() {
        "ctrl" | "control" | "control_l" | "control_r" => Key::Control,
        "shift" | "shift_l" | "shift_r" => Key::Shift,
        "alt" | "alt_l" | "alt_r" | "option" => Key::Alt,
        "super" | "super_l" | "win" | "windows" | "cmd" | "command" | "meta" => Key::Meta,
        "enter" | "return" | "kp_enter" => Key::Return,
        "tab" => Key::Tab,
        "esc" | "escape" => Key::Escape,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "page_up" | "prior" => Key::PageUp,
        "pagedown" | "page_down" | "next" => Key::PageDown,
        "up" | "arrowup" => Key::UpArrow,
        "down" | "arrowdown" => Key::DownArrow,
        "left" | "arrowleft" => Key::LeftArrow,
        "right" | "arrowright" => Key::RightArrow,
        "capslock" | "caps_lock" => Key::CapsLock,
        "f1" => Key::F1, "f2" => Key::F2, "f3" => Key::F3, "f4" => Key::F4,
        "f5" => Key::F5, "f6" => Key::F6, "f7" => Key::F7, "f8" => Key::F8,
        "f9" => Key::F9, "f10" => Key::F10, "f11" => Key::F11, "f12" => Key::F12,
        "minus" => Key::Unicode('-'),
        "plus" => Key::Unicode('+'),
        "equal" | "equals" => Key::Unicode('='),
        "comma" => Key::Unicode(','),
        "period" | "dot" => Key::Unicode('.'),
        "slash" => Key::Unicode('/'),
        "backslash" => Key::Unicode('\\'),
        "semicolon" => Key::Unicode(';'),
        "apostrophe" | "quote" => Key::Unicode('\''),
        "grave" | "backtick" => Key::Unicode('`'),
        _ => {
            let mut chars = n.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => char_key(c),
                _ => return Err(format!("неизвестная клавиша: {name}")),
            }
        }
    };
    Ok(k)
}

/// Буква или цифра как клавиша. На Windows латиница и цифры — прямыми кодами VK,
/// чтобы ctrl+s, ctrl+c и т. п. работали при любой раскладке (иначе при русской
/// раскладке enigo шлёт символ текстом, и сочетание не срабатывает).
fn char_key(c: char) -> Key {
    #[cfg(windows)]
    {
        if c.is_ascii_lowercase() {
            return Key::Other(c.to_ascii_uppercase() as u32);
        }
        if c.is_ascii_digit() {
            return Key::Other(c as u32);
        }
    }
    Key::Unicode(c)
}

pub fn is_modifier(k: &Key) -> bool {
    matches!(k, Key::Control | Key::Shift | Key::Alt | Key::Meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos() {
        assert_eq!(parse_combo("ctrl+s").unwrap(), vec![Key::Control, char_key('s')]);
        #[cfg(windows)]
        assert_eq!(parse_combo("ctrl+S").unwrap(), vec![Key::Control, Key::Other(0x53)]);
        assert_eq!(parse_combo("Return").unwrap(), vec![Key::Return]);
        assert_eq!(parse_combo("ctrl+shift+T").unwrap().len(), 3);
        assert!(parse_combo("ctrl+nonsense").is_err());
        assert_eq!(parse_combo("+").unwrap(), vec![Key::Unicode('+')]);
    }
}
