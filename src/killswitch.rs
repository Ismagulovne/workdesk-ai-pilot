//! Мгновенная остановка: Ctrl+Alt+Q или мышь в левом верхнем углу основного экрана.
//! Флаг остаётся поднятым, пока пользователь заново не разрешит окно в диалоге select_window.

use device_query::{DeviceQuery, DeviceState, Keycode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const CORNER: i32 = 3;

pub fn start() -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::spawn(move || {
        #[cfg(windows)]
        let ds = DeviceState::new();
        #[cfg(not(windows))]
        let ds = match DeviceState::checked_new() {
            Some(d) => d,
            None => {
                eprintln!("wdpilot: нет доступа к клавиатуре/мыши для стоп-клавиши \
                           (macOS: дайте разрешение «Универсальный доступ»). Стоп через угол экрана недоступен.");
                return;
            }
        };
        loop {
            let keys = ds.get_keys();
            let ctrl = keys.iter().any(|k| matches!(k, Keycode::LControl | Keycode::RControl));
            let alt = keys.iter().any(|k| matches!(k, Keycode::LAlt | Keycode::RAlt));
            let q = keys.contains(&Keycode::Q);
            let (mx, my) = ds.get_mouse().coords;
            let corner = mx <= CORNER && my <= CORNER && mx >= -CORNER && my >= -CORNER;
            if (ctrl && alt && q) || corner {
                if !flag.swap(true, Ordering::SeqCst) {
                    eprintln!("wdpilot: ОСТАНОВЛЕНО пользователем ({})",
                        if corner { "мышь в углу экрана" } else { "Ctrl+Alt+Q" });
                    crate::journal::write("kill_switch", None, serde_json::json!({}),
                        Ok(if corner { "corner" } else { "hotkey" }));
                }
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    });
    stop
}

pub fn stopped(flag: &AtomicBool) -> bool {
    flag.load(Ordering::SeqCst)
}

pub fn reset(flag: &AtomicBool) {
    flag.store(false, Ordering::SeqCst);
}

/// Сон, прерываемый стоп-флагом. Возвращает false, если прервали.
pub fn sleep(flag: &AtomicBool, secs: f64) -> bool {
    let end = std::time::Instant::now() + Duration::from_secs_f64(secs.max(0.0));
    while std::time::Instant::now() < end {
        if stopped(flag) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    !stopped(flag)
}
