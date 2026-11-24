//! Ядро: выбранное окно, снимки, перевод координат снимка в экранные, ввод мыши и клавиатуры
//! с проверками безопасности перед каждым действием.

use crate::config::Config;
use crate::{keys, killswitch, platform, policy};
use base64::Engine;
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use image::{imageops::FilterType, DynamicImage, ImageFormat, RgbaImage};
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use xcap::Window;

pub const STOPPED: &str = "Остановлено пользователем (Ctrl+Alt+Q или мышь в углу экрана). \
Управление окном отозвано. Прекратите действия и спросите человека, продолжать ли; \
продолжить можно только после нового разрешения (select_window).";

pub enum Out {
    Text(String),
    Image { png_b64: String, note: String },
}
pub type R = Result<Out, String>;

#[derive(Clone)]
pub struct Target {
    pub id: u32,
    pub pid: u32,
    pub app: String,
    pub title: String,
}

#[derive(Clone, Copy, PartialEq)]
struct Geo {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

impl Geo {
    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w as i32 && y < self.y + self.h as i32
    }
}

struct Shot {
    geo: Geo,
    sent_w: u32,
    sent_h: u32,
}

pub struct Pilot {
    cfg: Config,
    stop: Arc<AtomicBool>,
    enigo: Option<Enigo>,
    target: Option<Target>,
    shot: Option<Shot>,
}

fn es<T: std::fmt::Display>(x: T) -> String {
    x.to_string()
}
fn ie<T: std::fmt::Debug>(x: T) -> String {
    format!("ошибка ввода: {x:?}")
}

fn geo(w: &Window) -> Result<Geo, String> {
    Ok(Geo {
        x: w.x().map_err(es)?,
        y: w.y().map_err(es)?,
        w: w.width().map_err(es)?,
        h: w.height().map_err(es)?,
    })
}

/// Имя приложения для людей: если ОС отдаёт полный путь к exe — только имя файла без расширения.
pub fn app_name(w: &Window) -> String {
    let raw = w.app_name().unwrap_or_default();
    if raw.contains('\\') || raw.contains('/') {
        let file = raw.rsplit(['\\', '/']).next().unwrap_or(&raw);
        return file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(file).to_string();
    }
    raw
}

fn find(id: u32) -> Result<Window, String> {
    Window::all()
        .map_err(|e| format!("не удалось получить список окон: {e}"))?
        .into_iter()
        .find(|w| w.id().ok() == Some(id))
        .ok_or_else(|| format!("окно {id} не найдено (закрыто?)"))
}

/// Размер снимка, который отдаём модели: длинная сторона и число пикселей в пределах настроек.
pub fn fit(w: u32, h: u32, long_edge: u32, max_px: u32) -> (u32, u32) {
    let (wf, hf) = (w.max(1) as f64, h.max(1) as f64);
    let s = 1f64
        .min(long_edge as f64 / wf.max(hf))
        .min((max_px as f64 / (wf * hf)).sqrt());
    (((wf * s).round() as u32).max(1), ((hf * s).round() as u32).max(1))
}

fn png(img: RgbaImage) -> Result<Vec<u8>, String> {
    let mut buf = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(img)
        .write_to(&mut buf, ImageFormat::Png)
        .map_err(|e| format!("PNG: {e}"))?;
    Ok(buf.into_inner())
}

pub fn windows_json(cfg: &Config) -> Result<Value, String> {
    let mut out = vec![];
    for w in Window::all().map_err(es)? {
        if w.is_minimized().unwrap_or(false) {
            continue;
        }
        let (Ok(id), Ok(g)) = (w.id(), geo(&w)) else { continue };
        if g.w < 40 || g.h < 40 {
            continue;
        }
        let title = w.title().unwrap_or_default();
        let app = app_name(&w);
        if title.trim().is_empty() && app.trim().is_empty() {
            continue;
        }
        let allowed = policy::window_allowed(cfg, &app, &title);
        out.push(json!({
            "window_id": id, "app": app, "title": title, "pid": w.pid().unwrap_or(0),
            "x": g.x, "y": g.y, "width": g.w, "height": g.h,
            "focused": w.is_focused().unwrap_or(false),
            "allowed": allowed.is_ok(),
        }));
    }
    Ok(Value::Array(out))
}

impl Pilot {
    pub fn new(cfg: Config, stop: Arc<AtomicBool>) -> Self {
        Pilot { cfg, stop, enigo: None, target: None, shot: None }
    }

    pub fn log_typed_text(&self) -> bool {
        self.cfg.log_typed_text
    }

    pub fn log(&self, action: &str, args: Value, res: &R) {
        let window = self.target.as_ref().map(|t| json!({"id": t.id, "app": t.app, "title": t.title}));
        let outcome = match res {
            Ok(Out::Text(t)) => Ok(if t.len() > 200 { "ok" } else { t.as_str() }),
            Ok(Out::Image { note, .. }) => Ok(note.as_str()),
            Err(m) => Err(m.as_str()),
        };
        crate::journal::write(action, window, args, outcome);
    }

    // ---------- выбор окна ----------

    pub fn list_windows(&self) -> R {
        let v = windows_json(&self.cfg)?;
        Ok(Out::Text(serde_json::to_string_pretty(&v).map_err(es)?))
    }

    pub fn select_window(&mut self, id: u32) -> R {
        let w = find(id)?;
        let app = app_name(&w);
        let title = w.title().unwrap_or_default();
        let pid = w.pid().map_err(es)?;
        policy::window_allowed(&self.cfg, &app, &title)?;
        let trusted = self.cfg.trusted_apps.iter().any(|a| a.eq_ignore_ascii_case(&app));
        if !trusted {
            let answer = rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("WorkDesk AI Pilot — разрешить ИИ управлять окном?")
                .set_description(format!(
                    "ИИ-агент просит управлять окном:\n\n{app} — «{title}»\n\n\
                     Он будет видеть только это окно и нажимать в нём мышью и клавиатурой.\n\
                     Пароли, коды и платёжные данные он вводить не будет.\n\n\
                     Остановить в любой момент: Ctrl+Alt+Q или увести мышь в левый верхний угол экрана.\n\n\
                     Разрешить?"
                ))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show();
            if answer != rfd::MessageDialogResult::Yes {
                self.target = None;
                self.shot = None;
                return Err("пользователь не разрешил управлять этим окном".into());
            }
        }
        killswitch::reset(&self.stop);
        self.target = Some(Target { id, pid, app: app.clone(), title: title.clone() });
        self.shot = None;
        Ok(Out::Text(format!("Окно выбрано: {app} — «{title}». Дальше: screenshot.")))
    }

    pub fn release(&mut self) -> R {
        self.target = None;
        self.shot = None;
        Ok(Out::Text("Окно освобождено, управление завершено.".into()))
    }

    pub fn status(&self) -> R {
        Ok(Out::Text(
            json!({
                "window": self.target.as_ref().map(|t| json!({"id": t.id, "app": t.app, "title": t.title})),
                "stopped": killswitch::stopped(&self.stop),
                "last_screenshot": self.shot.as_ref().map(|s| json!([s.sent_w, s.sent_h])),
                "config": crate::config::app_dir().join("config.json"),
                "journal": crate::config::log_dir(),
                "stop_hotkey": "Ctrl+Alt+Q или мышь в левый верхний угол",
            })
            .to_string(),
        ))
    }

    /// Проверки перед каждым действием: стоп, окно на месте, тот же процесс, не запрещено.
    fn live(&mut self) -> Result<Window, String> {
        if killswitch::stopped(&self.stop) {
            self.target = None;
            self.shot = None;
            return Err(STOPPED.into());
        }
        let t = self
            .target
            .clone()
            .ok_or("окно не выбрано: вызовите list_windows, затем select_window")?;
        let w = match find(t.id) {
            Ok(w) => w,
            Err(e) => {
                self.target = None;
                return Err(e);
            }
        };
        if w.pid().unwrap_or(0) != t.pid {
            self.target = None;
            return Err("под этим id теперь окно другого процесса — выберите окно заново".into());
        }
        let title = w.title().unwrap_or_default();
        policy::window_allowed(&self.cfg, &t.app, &title)?;
        if let Some(tt) = self.target.as_mut() {
            tt.title = title;
        }
        Ok(w)
    }

    fn tid(&self) -> u32 {
        self.target.as_ref().map(|t| t.id).unwrap_or(0)
    }

    fn enigo(&mut self) -> Result<&mut Enigo, String> {
        if self.enigo.is_none() {
            let en = Enigo::new(&Settings::default())
                .map_err(|e| format!("нет доступа к вводу (macOS: «Универсальный доступ»): {e:?}"))?;
            self.enigo = Some(en);
        }
        Ok(self.enigo.as_mut().expect("enigo"))
    }

    // ---------- снимки ----------

    pub fn screenshot(&mut self) -> R {
        let w = self.live()?;
        if w.is_minimized().unwrap_or(false) {
            platform::bring_to_front(self.tid());
            std::thread::sleep(Duration::from_millis(300));
        }
        let g = geo(&w)?;
        let img = w.capture_image().map_err(|e| format!("не удалось снять окно: {e}"))?;
        let (sw, sh) = fit(img.width(), img.height(), self.cfg.max_long_edge, self.cfg.max_pixels);
        let small = if (sw, sh) == (img.width(), img.height()) {
            img
        } else {
            image::imageops::resize(&img, sw, sh, FilterType::Triangle)
        };
        let data = png(small)?;
        if self.cfg.save_screenshots {
            crate::journal::save_png(&data);
        }
        self.shot = Some(Shot { geo: g, sent_w: sw, sent_h: sh });
        let title = self.target.as_ref().map(|t| t.title.clone()).unwrap_or_default();
        Ok(Out::Image {
            png_b64: base64::engine::general_purpose::STANDARD.encode(&data),
            note: format!("Снимок {sw}x{sh} окна «{title}». Координаты действий — в пикселях этого снимка."),
        })
    }

    pub fn zoom(&mut self, r: [f64; 4]) -> R {
        let w = self.live()?;
        let (sent_w, sent_h) = match &self.shot {
            Some(s) => (s.sent_w as f64, s.sent_h as f64),
            None => return Err("сначала сделайте screenshot".into()),
        };
        let img = w.capture_image().map_err(|e| format!("не удалось снять окно: {e}"))?;
        let (fx, fy) = (img.width() as f64 / sent_w, img.height() as f64 / sent_h);
        let x0 = (r[0].min(r[2]).max(0.0) * fx) as u32;
        let y0 = (r[1].min(r[3]).max(0.0) * fy) as u32;
        let x1 = ((r[0].max(r[2]) * fx) as u32).min(img.width());
        let y1 = ((r[1].max(r[3]) * fy) as u32).min(img.height());
        if x1 <= x0 + 1 || y1 <= y0 + 1 {
            return Err("пустая или неверная область zoom".into());
        }
        let crop = image::imageops::crop_imm(&img, x0, y0, x1 - x0, y1 - y0).to_image();
        let (zw, zh) = fit(crop.width(), crop.height(), self.cfg.max_long_edge, self.cfg.max_pixels);
        let crop = if (zw, zh) == (crop.width(), crop.height()) {
            crop
        } else {
            image::imageops::resize(&crop, zw, zh, FilterType::Triangle)
        };
        let data = png(crop)?;
        Ok(Out::Image {
            png_b64: base64::engine::general_purpose::STANDARD.encode(&data),
            note: format!(
                "Увеличенный фрагмент [{}, {}, {}, {}]. Координаты для кликов — по-прежнему по полному снимку.",
                r[0], r[1], r[2], r[3]
            ),
        })
    }

    // ---------- координаты ----------

    fn to_screen(&mut self, w: &Window, x: f64, y: f64) -> Result<(i32, i32), String> {
        let s = self.shot.as_ref().ok_or("сначала сделайте screenshot")?;
        let g = geo(w)?;
        if g.w != s.geo.w || g.h != s.geo.h {
            return Err("размер окна изменился — сделайте новый screenshot".into());
        }
        if x < 0.0 || y < 0.0 || x >= s.sent_w as f64 || y >= s.sent_h as f64 {
            return Err(format!("точка ({x}, {y}) вне снимка {}x{}", s.sent_w, s.sent_h));
        }
        let sx = g.x + (x * g.w as f64 / s.sent_w as f64).round() as i32;
        let sy = g.y + (y * g.h as f64 / s.sent_h as f64).round() as i32;
        if sx.abs() <= killswitch::CORNER + 1 && sy.abs() <= killswitch::CORNER + 1 {
            return Err("точка совпадает с углом-стопом экрана".into());
        }
        self.not_covered(w, sx, sy)?;
        Ok((sx, sy))
    }

    #[cfg(windows)]
    fn not_covered(&self, _w: &Window, sx: i32, sy: i32) -> Result<(), String> {
        let pid = self.target.as_ref().map(|t| t.pid).unwrap_or(0);
        match platform::point_owner_pid(sx, sy) {
            Some(p) if p != pid => Err(format!(
                "в этой точке другое окно (процесс {p}) — ИИ нажимает только в выбранном окне. \
                 Выведите окно наверх (focus_window) и сделайте новый screenshot."
            )),
            _ => Ok(()),
        }
    }

    #[cfg(not(windows))]
    fn not_covered(&self, w: &Window, sx: i32, sy: i32) -> Result<(), String> {
        let Some(t) = self.target.as_ref() else { return Ok(()) };
        let my_z = w.z().unwrap_or(i32::MIN);
        for o in Window::all().map_err(es)? {
            if o.id().ok() == Some(t.id) || o.pid().unwrap_or(0) == t.pid || o.is_minimized().unwrap_or(false) {
                continue;
            }
            let title = o.title().unwrap_or_default();
            if title.trim().is_empty() {
                continue; // служебные прозрачные окна оболочки
            }
            let Ok(og) = geo(&o) else { continue };
            if o.z().unwrap_or(i32::MIN) > my_z && og.contains(sx, sy) {
                return Err(format!(
                    "точку перекрывает окно «{title}» ({}) — выведите выбранное окно наверх и сделайте новый screenshot",
                    o.app_name().unwrap_or_default()
                ));
            }
        }
        Ok(())
    }

    fn cursor_inside(&mut self, w: &Window) -> Result<(i32, i32), String> {
        let g = geo(w)?;
        let (cx, cy) = self.enigo()?.location().map_err(ie)?;
        if !g.contains(cx, cy) {
            return Err("курсор вне выбранного окна — укажите coordinate".into());
        }
        self.not_covered(w, cx, cy)?;
        Ok((cx, cy))
    }

    fn front_for_mouse(&self, w: &Window) {
        if !w.is_focused().unwrap_or(false) {
            platform::bring_to_front(self.tid());
            std::thread::sleep(Duration::from_millis(120));
        }
    }

    fn ensure_keyboard(&mut self, w: &Window) -> Result<(), String> {
        if !w.is_focused().unwrap_or(false) {
            platform::bring_to_front(self.tid());
            std::thread::sleep(Duration::from_millis(150));
            if !w.is_focused().unwrap_or(false) {
                return Err("окно не в фокусе: кликните внутри него (left_click) и повторите".into());
            }
        }
        let title = w.title().unwrap_or_default();
        policy::typing_allowed(&self.cfg, &title)?;
        if platform::focus_in_password_field(self.tid()) {
            return Err("фокус в поле пароля — пароль вводит человек, попросите пользователя".into());
        }
        Ok(())
    }

    fn with_mods(
        &mut self,
        mods: &str,
        f: impl FnOnce(&mut Enigo, &AtomicBool) -> Result<(), String>,
    ) -> Result<(), String> {
        let mods: Vec<Key> = if mods.trim().is_empty() { vec![] } else { keys::parse_combo(mods)? };
        if let Some(k) = mods.iter().find(|k| !keys::is_modifier(k)) {
            return Err(format!("в text для мыши допускаются только модификаторы, получено {k:?}"));
        }
        let stop = self.stop.clone();
        let en = self.enigo()?;
        for k in &mods {
            en.key(*k, Direction::Press).map_err(ie)?;
        }
        let r = f(en, &stop);
        for k in mods.iter().rev() {
            let _ = en.key(*k, Direction::Release);
        }
        r
    }

    // ---------- мышь ----------

    pub fn click(&mut self, button: &str, count: u32, at: Option<(f64, f64)>, mods: &str) -> R {
        let w = self.live()?;
        self.front_for_mouse(&w);
        let pos = match at {
            Some((x, y)) => Some(self.to_screen(&w, x, y)?),
            None => {
                self.cursor_inside(&w)?;
                None
            }
        };
        let btn = match button {
            "right" => Button::Right,
            "middle" => Button::Middle,
            _ => Button::Left,
        };
        self.with_mods(mods, |en, _| {
            if let Some((x, y)) = pos {
                en.move_mouse(x, y, Coordinate::Abs).map_err(ie)?;
            }
            for i in 0..count {
                en.button(btn, Direction::Click).map_err(ie)?;
                if i + 1 < count {
                    std::thread::sleep(Duration::from_millis(40));
                }
            }
            Ok(())
        })?;
        Ok(Out::Text("OK".into()))
    }

    pub fn drag(&mut self, from: (f64, f64), to: (f64, f64), mods: &str) -> R {
        let w = self.live()?;
        self.front_for_mouse(&w);
        let (ax, ay) = self.to_screen(&w, from.0, from.1)?;
        let (bx, by) = self.to_screen(&w, to.0, to.1)?;
        self.with_mods(mods, |en, stop| {
            en.move_mouse(ax, ay, Coordinate::Abs).map_err(ie)?;
            en.button(Button::Left, Direction::Press).map_err(ie)?;
            let steps = 15;
            for i in 1..=steps {
                if killswitch::stopped(stop) {
                    let _ = en.button(Button::Left, Direction::Release);
                    return Err(STOPPED.into());
                }
                let x = ax + (bx - ax) * i / steps;
                let y = ay + (by - ay) * i / steps;
                en.move_mouse(x, y, Coordinate::Abs).map_err(ie)?;
                std::thread::sleep(Duration::from_millis(12));
            }
            en.button(Button::Left, Direction::Release).map_err(ie)
        })?;
        Ok(Out::Text("OK".into()))
    }

    pub fn mouse_move(&mut self, at: (f64, f64)) -> R {
        let w = self.live()?;
        let (x, y) = self.to_screen(&w, at.0, at.1)?;
        self.enigo()?.move_mouse(x, y, Coordinate::Abs).map_err(ie)?;
        Ok(Out::Text("OK".into()))
    }

    pub fn mouse_button(&mut self, down: bool) -> R {
        let w = self.live()?;
        if down {
            self.cursor_inside(&w)?;
        }
        let dir = if down { Direction::Press } else { Direction::Release };
        self.enigo()?.button(Button::Left, dir).map_err(ie)?;
        Ok(Out::Text("OK".into()))
    }

    pub fn cursor_position(&mut self) -> R {
        let w = self.live()?;
        let g = geo(&w)?;
        let (cx, cy) = self.enigo()?.location().map_err(ie)?;
        let Some(s) = self.shot.as_ref() else { return Err("сначала сделайте screenshot".into()) };
        if !g.contains(cx, cy) {
            return Ok(Out::Text("Курсор вне выбранного окна.".into()));
        }
        let x = ((cx - g.x) as f64 * s.sent_w as f64 / g.w as f64).round();
        let y = ((cy - g.y) as f64 * s.sent_h as f64 / g.h as f64).round();
        Ok(Out::Text(format!("X={x}, Y={y}")))
    }

    pub fn scroll(&mut self, dir: &str, amount: f64, at: Option<(f64, f64)>, mods: &str) -> R {
        let w = self.live()?;
        let pos = match at {
            Some((x, y)) => Some(self.to_screen(&w, x, y)?),
            None => {
                self.cursor_inside(&w)?;
                None
            }
        };
        let a = (amount.round() as i32).clamp(1, 50);
        let (len, axis) = match dir {
            "up" => (-a, Axis::Vertical),
            "down" => (a, Axis::Vertical),
            "left" => (-a, Axis::Horizontal),
            "right" => (a, Axis::Horizontal),
            _ => return Err("scroll_direction: up, down, left или right".into()),
        };
        self.with_mods(mods, |en, _| {
            if let Some((x, y)) = pos {
                en.move_mouse(x, y, Coordinate::Abs).map_err(ie)?;
            }
            en.scroll(len, axis).map_err(ie)
        })?;
        Ok(Out::Text("OK".into()))
    }

    // ---------- клавиатура ----------

    pub fn type_text(&mut self, text: &str) -> R {
        let w = self.live()?;
        self.ensure_keyboard(&w)?;
        policy::text_allowed(text)?;
        let stop = self.stop.clone();
        #[cfg(windows)]
        {
            // Переводы строк и табуляцию — клавишами, остальное — посимвольно через SendInput.
            let parts = text.split_inclusive(['\n', '\t']);
            for part in parts {
                let (body, special) = match part.chars().last() {
                    Some('\n') => (&part[..part.len() - 1], Some(Key::Return)),
                    Some('\t') => (&part[..part.len() - 1], Some(Key::Tab)),
                    _ => (part, None),
                };
                let body = body.trim_end_matches('\r');
                if !platform::type_unicode(body, &stop)? {
                    return Err(STOPPED.into());
                }
                if let Some(k) = special {
                    self.enigo()?.key(k, Direction::Click).map_err(ie)?;
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            return Ok(Out::Text("OK".into()));
        }
        #[allow(unreachable_code)]
        for c in text.chars() {
            if killswitch::stopped(&stop) {
                return Err(STOPPED.into());
            }
            let mut buf = [0u8; 4];
            self.enigo()?.text(c.encode_utf8(&mut buf)).map_err(ie)?;
            std::thread::sleep(Duration::from_millis(8));
        }
        Ok(Out::Text("OK".into()))
    }

    pub fn key(&mut self, combo: &str, repeat: f64) -> R {
        let w = self.live()?;
        self.ensure_keyboard(&w)?;
        let ks = keys::parse_combo(combo)?;
        let n = (repeat.round() as i32).clamp(1, 100);
        let stop = self.stop.clone();
        let en = self.enigo()?;
        for _ in 0..n {
            if killswitch::stopped(&stop) {
                return Err(STOPPED.into());
            }
            for k in &ks {
                en.key(*k, Direction::Press).map_err(ie)?;
            }
            for k in ks.iter().rev() {
                en.key(*k, Direction::Release).map_err(ie)?;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        Ok(Out::Text("OK".into()))
    }

    pub fn hold_key(&mut self, combo: &str, secs: f64) -> R {
        let w = self.live()?;
        self.ensure_keyboard(&w)?;
        let ks = keys::parse_combo(combo)?;
        let stop = self.stop.clone();
        let en = self.enigo()?;
        for k in &ks {
            en.key(*k, Direction::Press).map_err(ie)?;
        }
        let finished = killswitch::sleep(&stop, secs.clamp(0.0, 300.0));
        for k in ks.iter().rev() {
            let _ = en.key(*k, Direction::Release);
        }
        if finished { Ok(Out::Text("OK".into())) } else { Err(STOPPED.into()) }
    }

    pub fn wait(&mut self, secs: f64) -> R {
        if killswitch::stopped(&self.stop) {
            return Err(STOPPED.into());
        }
        if killswitch::sleep(&self.stop, secs.clamp(0.0, 300.0)) {
            Ok(Out::Text("OK".into()))
        } else {
            Err(STOPPED.into())
        }
    }

    pub fn focus_window(&mut self) -> R {
        let w = self.live()?;
        if platform::bring_to_front(self.tid()) || w.is_focused().unwrap_or(false) {
            Ok(Out::Text("Окно на переднем плане.".into()))
        } else {
            Ok(Out::Text("На этой ОС окно активируется кликом по нему (left_click внутри окна).".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_limits() {
        assert_eq!(fit(1000, 800, 1568, 1_150_000), (1000, 800));
        let (w, h) = fit(3840, 2160, 1568, 1_150_000);
        assert!(w <= 1568 && (w * h) <= 1_160_000);
        let (w, h) = fit(2880, 1800, 1568, 1_150_000); // Retina 2x
        assert!(w <= 1568 && h < 1000);
    }
}
