//! Платформенные проверки: DPI, вывод окна на передний план, кто владеет точкой экрана,
//! находится ли фокус в поле пароля (Windows: стиль ES_PASSWORD у стандартного Edit).

pub fn init() {
    #[cfg(windows)]
    {
        // Координаты окна, снимка и мыши — в физических пикселях на всех мониторах.
        let _ = enigo::set_dpi_awareness();
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{HWND, POINT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetClassNameW, GetGUIThreadInfo, GetWindowLongW, GetWindowThreadProcessId,
        IsIconic, SetForegroundWindow, ShowWindow, WindowFromPoint, ES_PASSWORD, GA_ROOT,
        GUITHREADINFO, GWL_STYLE, SW_RESTORE,
    };

    fn hwnd(id: u32) -> HWND {
        id as usize as HWND
    }

    pub fn bring_to_front(id: u32) -> bool {
        unsafe {
            let h = hwnd(id);
            if IsIconic(h) != 0 {
                ShowWindow(h, SW_RESTORE);
            }
            SetForegroundWindow(h) != 0
        }
    }

    pub fn point_owner_pid(x: i32, y: i32) -> Option<u32> {
        unsafe {
            let h = WindowFromPoint(POINT { x, y });
            if h.is_null() {
                return None;
            }
            let root = GetAncestor(h, GA_ROOT);
            let mut pid = 0u32;
            GetWindowThreadProcessId(if root.is_null() { h } else { root }, &mut pid);
            Some(pid)
        }
    }

    pub fn focus_in_password_field(id: u32) -> bool {
        unsafe {
            let tid = GetWindowThreadProcessId(hwnd(id), std::ptr::null_mut());
            let mut gi: GUITHREADINFO = std::mem::zeroed();
            gi.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
            if GetGUIThreadInfo(tid, &mut gi) == 0 || gi.hwndFocus.is_null() {
                return false;
            }
            let mut buf = [0u16; 64];
            let n = GetClassNameW(gi.hwndFocus, buf.as_mut_ptr(), buf.len() as i32);
            let class = String::from_utf16_lossy(&buf[..n.max(0) as usize]).to_lowercase();
            let style = GetWindowLongW(gi.hwndFocus, GWL_STYLE);
            class.contains("edit") && (style & ES_PASSWORD) != 0
        }
    }
}

/// Ввод текста на Windows: каждый UTF-16 символ — отдельные SendInput «нажать» и «отпустить»
/// (KEYEVENTF_UNICODE) с паузами. Пачка событий от enigo в Блокноте Windows 11 теряла
/// и дублировала символы. Возвращает false, если прервали стоп-клавишей.
#[cfg(windows)]
pub fn type_unicode(text: &str, stop: &std::sync::atomic::AtomicBool) -> Result<bool, String> {
    use std::time::Duration;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    };
    fn send(unit: u16, up: bool) -> Result<(), String> {
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: 0,
                    wScan: unit,
                    dwFlags: KEYEVENTF_UNICODE | if up { KEYEVENTF_KEYUP } else { 0 },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let n = unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) };
        if n == 1 { Ok(()) } else { Err("SendInput: ввод заблокирован (окно с правами администратора?)".into()) }
    }
    let mut buf = [0u16; 2];
    for c in text.chars() {
        if crate::killswitch::stopped(stop) {
            return Ok(false);
        }
        for &unit in c.encode_utf16(&mut buf).iter() {
            send(unit, false)?;
            std::thread::sleep(Duration::from_millis(4));
            send(unit, true)?;
        }
        std::thread::sleep(Duration::from_millis(14));
    }
    Ok(true)
}

#[cfg(not(windows))]
mod imp {
    pub fn bring_to_front(_id: u32) -> bool {
        false
    }
    pub fn focus_in_password_field(_id: u32) -> bool {
        false
    }
}

pub use imp::{bring_to_front, focus_in_password_field};
#[cfg(windows)]
pub use imp::point_owner_pid;
