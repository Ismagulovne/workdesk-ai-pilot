//! Платформенные проверки: DPI, вывод окна на передний план, кто владеет точкой экрана,
//! находится ли фокус в поле пароля (Windows: стиль ES_PASSWORD у стандартного Edit).

/// Вопрос человеку «Да / Нет». По умолчанию выбрано «Нет» (случайный Enter — отказ),
/// окно поверх всех и выводится на передний план.
pub fn confirm(title: &str, text: &str) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO,
        };
        let w = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let (t, c) = (w(text), w(title));
        let r = unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                t.as_ptr(),
                c.as_ptr(),
                MB_YESNO | MB_DEFBUTTON2 | MB_ICONWARNING | MB_TOPMOST | MB_SETFOREGROUND,
            )
        };
        r == IDYES
    }
    #[cfg(not(windows))]
    {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(title)
            .set_description(text)
            .set_buttons(rfd::MessageButtons::YesNo)
            .show()
            == rfd::MessageDialogResult::Yes
    }
}

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

/// Вставка текста через буфер обмена (Windows): сохранить текст буфера пользователя,
/// положить наш текст с пометками «не в историю, не в облако, не для менеджеров буфера»,
/// нажать Ctrl+V, вернуть прежний текст. Если в буфере сейчас не текст (картинка, файлы) —
/// не трогаем его и возвращаем Ok(false): вызывающий введёт текст посимвольно.
#[cfg(windows)]
pub fn paste_text(text: &str) -> Result<bool, String> {
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{GlobalFree, HANDLE};
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, CountClipboardFormats, EmptyClipboard, GetClipboardData,
        IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    };
    const CF_UNICODETEXT: u32 = 13;

    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe { CloseClipboard() };
        }
    }
    fn open() -> Result<Guard, String> {
        for _ in 0..25 {
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Ok(Guard);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err("буфер обмена занят другой программой".into())
    }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    unsafe fn put(format: u32, bytes: &[u8]) -> bool {
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
        if h.is_null() {
            return false;
        }
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            GlobalFree(h);
            return false;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        GlobalUnlock(h);
        if SetClipboardData(format, h as HANDLE).is_null() {
            GlobalFree(h);
            return false;
        }
        true
    }
    unsafe fn put_text_private(s: &str) -> bool {
        let w = wide(s);
        let bytes = std::slice::from_raw_parts(w.as_ptr() as *const u8, w.len() * 2);
        if !put(CF_UNICODETEXT, bytes) {
            return false;
        }
        // Пометки для истории буфера Windows (Win+V), облачного буфера и менеджеров буфера.
        let zero = 0u32.to_ne_bytes();
        for name in ["ExcludeClipboardContentFromMonitorProcessing", "CanIncludeInClipboardHistory", "CanUploadToCloudClipboard"] {
            let f = RegisterClipboardFormatW(wide(name).as_ptr());
            if f != 0 {
                put(f, &zero);
            }
        }
        true
    }
    unsafe fn read_text() -> Option<String> {
        let h = GetClipboardData(CF_UNICODETEXT);
        if h.is_null() {
            return None;
        }
        let p = GlobalLock(h) as *const u16;
        if p.is_null() {
            return None;
        }
        let max = GlobalSize(h) / 2;
        let mut len = 0;
        while len < max && *p.add(len) != 0 {
            len += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
        GlobalUnlock(h);
        Some(s)
    }
    fn key(vk: u16, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 },
            },
        }
    }

    let text = text.replace("\r\n", "\n").replace('\n', "\r\n");
    let saved = {
        let _g = open()?;
        let count = unsafe { CountClipboardFormats() };
        let has_text = unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) } != 0;
        if count > 0 && !has_text {
            return Ok(false); // в буфере картинка или файлы — не портим
        }
        let saved = if has_text { unsafe { read_text() } } else { None };
        unsafe {
            EmptyClipboard();
            if !put_text_private(&text) {
                return Err("не удалось положить текст в буфер обмена".into());
            }
        }
        saved
    };
    // Ctrl+V
    let seq = [key(0x11, false), key(0x56, false), key(0x56, true), key(0x11, true)];
    for k in &seq {
        unsafe { SendInput(1, k, std::mem::size_of::<INPUT>() as i32) };
        std::thread::sleep(Duration::from_millis(10));
    }
    // Даём приложению прочитать буфер, затем возвращаем пользователю его текст.
    std::thread::sleep(Duration::from_millis(400));
    let _g = open()?;
    unsafe {
        EmptyClipboard();
        if let Some(old) = saved {
            put_text_private(&old);
        }
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
