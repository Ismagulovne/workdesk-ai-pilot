//! Распознавание текста в окне — для списка запретов пользователя (например, боевая база 1С).
//! Windows: встроенный Windows.Media.Ocr (русский, затем языки профиля). Другие ОС — пока нет.

use image::RgbaImage;

/// Строка текста и её прямоугольник (x, y, ширина, высота) в пикселях исходного снимка.
pub type Line = (String, (f32, f32, f32, f32));

/// Подготовка для распознавания: оттенки серого, тёмная тема → светлая, мелкое — увеличить в 2 раза.
fn prepare(img: &RgbaImage, max_dim: u32) -> (RgbaImage, f32) {
    let mut g = img.clone();
    let mut sum: u64 = 0;
    for p in g.pixels_mut() {
        let l = (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) as u8;
        *p = image::Rgba([l, l, l, 255]);
        sum += l as u64;
    }
    let dark = sum / (g.width() as u64 * g.height() as u64).max(1) < 110;
    if dark {
        for p in g.pixels_mut() {
            let l = 255 - p[0];
            *p = image::Rgba([l, l, l, 255]);
        }
    }
    let mut k = if g.width().max(g.height()) * 2 <= max_dim { 2.0 } else { 1.0 };
    if (g.width().max(g.height()) as f32 * k) > max_dim as f32 {
        k = max_dim as f32 / g.width().max(g.height()) as f32;
    }
    if (k - 1.0).abs() > f32::EPSILON {
        g = image::imageops::resize(
            &g,
            ((g.width() as f32 * k) as u32).max(1),
            ((g.height() as f32 * k) as u32).max(1),
            image::imageops::FilterType::CatmullRom,
        );
    }
    (g, k)
}

/// Строки текста с координатами. Err — распознавание недоступно.
#[cfg(windows)]
pub fn recognize_lines(img: &RgbaImage) -> Result<Vec<Line>, String> {
    use windows::core::HSTRING;
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Security::Cryptography::CryptographicBuffer;
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    });
    let e = |x: windows::core::Error| format!("распознавание текста: {}", x.message());
    let engine = Language::CreateLanguage(&HSTRING::from("ru"))
        .and_then(|l| OcrEngine::TryCreateFromLanguage(&l))
        .or_else(|_| OcrEngine::TryCreateFromUserProfileLanguages())
        .map_err(e)?;
    let max = OcrEngine::MaxImageDimension().unwrap_or(4096);
    let (src, k) = prepare(img, max);
    let buf = CryptographicBuffer::CreateFromByteArray(src.as_raw()).map_err(e)?;
    let bmp = SoftwareBitmap::CreateCopyFromBuffer(&buf, BitmapPixelFormat::Rgba8, src.width() as i32, src.height() as i32)
        .map_err(e)?;
    let res = engine.RecognizeAsync(&bmp).map_err(e)?.join().map_err(e)?;
    let lines = res.Lines().map_err(e)?;
    let mut out = Vec::new();
    for i in 0..lines.Size().map_err(e)? {
        let line = lines.GetAt(i).map_err(e)?;
        let text = line.Text().map_err(e)?.to_string();
        let words = line.Words().map_err(e)?;
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for j in 0..words.Size().map_err(e)? {
            let r = words.GetAt(j).map_err(e)?.BoundingRect().map_err(e)?;
            x0 = x0.min(r.X);
            y0 = y0.min(r.Y);
            x1 = x1.max(r.X + r.Width);
            y1 = y1.max(r.Y + r.Height);
        }
        if x0 <= x1 {
            out.push((text, (x0 / k, y0 / k, (x1 - x0) / k, (y1 - y0) / k)));
        }
    }
    Ok(out)
}

#[cfg(not(windows))]
pub fn recognize_lines(_img: &RgbaImage) -> Result<Vec<Line>, String> {
    let _ = prepare;
    Err("распознавание текста на этой ОС пока не поддерживается".into())
}

/// Весь текст окна (для диагностики `--ocr`).
pub fn recognize(img: &RgbaImage) -> Result<String, String> {
    Ok(recognize_lines(img)?.into_iter().map(|l| l.0).collect::<Vec<_>>().join("\n"))
}

/// Для сравнения: нижний регистр, латинские двойники и цифры 3/0 → русские буквы,
/// без пробелов и знаков.
pub fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter_map(|c| {
            let c = match c {
                'a' => 'а', 'b' => 'в', 'c' => 'с', 'e' => 'е', 'h' => 'н', 'k' => 'к',
                'm' => 'м', 'o' | '0' => 'о', 'p' => 'р', 't' => 'т', 'x' => 'х', 'y' => 'у',
                '3' => 'з', 'ё' => 'е', 'u' | 'n' => 'и',
                c => c,
            };
            c.is_alphanumeric().then_some(c)
        })
        .collect()
}

/// Наименьшее число правок, чтобы фраза встретилась в тексте как подстрока (алгоритм Селлерса).
fn fuzzy_distance(needle: &[char], hay: &[char]) -> usize {
    let m = needle.len();
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut best = m;
    for &h in hay {
        let mut cur = vec![0usize; m + 1];
        for i in 1..=m {
            let sub = prev[i - 1] + usize::from(needle[i - 1] != h);
            cur[i] = sub.min(prev[i] + 1).min(cur[i - 1] + 1);
        }
        best = best.min(cur[m]);
        prev = cur;
    }
    best
}

/// Первая цель из списка запретов, найденная в тексте. Распознавание ошибается,
/// поэтому допускается до 1 ошибки на каждые 6 букв названия.
pub fn find_protected<'a>(text: &str, phrases: &'a [String]) -> Option<&'a str> {
    let hay: Vec<char> = normalize(text).chars().collect();
    phrases
        .iter()
        .find(|p| {
            let n: Vec<char> = normalize(p).chars().collect();
            !n.is_empty() && fuzzy_distance(&n, &hay) <= n.len() / 6
        })
        .map(|p| p.as_str())
}

/// Похоже ли на кнопку запуска или входа (1С, вход, подключение).
pub fn is_launch_control(text: &str) -> bool {
    let n = normalize(text);
    ["предприятие", "конфигуратор", "войти", "открыть", "запуск", "подключ"]
        .iter()
        .any(|w| n.contains(&normalize(w)))
        || n == "ок"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_matching() {
        let p = vec!["ТОО ФРК РОЗНИЦА".to_string()];
        assert!(find_protected("Запуск 1С\nТОО ФРК РОЗНИЦА 8.3\nTEST_MAIN", &p).is_some());
        assert!(find_protected("TOO ФPK P0ЗНИЦА", &p).is_some());
        assert!(find_protected("ТОО ОРК РОЗНИЦА 8.3", &p).is_some()); // ошибка распознавания Ф→О
        assert!(find_protected("тоо  фрк\nрозница", &p).is_some());
        assert!(find_protected("TEST_MAIN\nWorkTime_TEST\nТОО ФРК БУХ 8.3\nРОЗНИЦА 8.3", &p).is_none());
        assert!(find_protected("ТОО ОРК БУХ 8.3 TEST МАТ", &p).is_none());
    }

    #[test]
    fn launch_controls() {
        assert!(is_launch_control("1С:Предприятие"));
        assert!(is_launch_control("OK"));
        assert!(!is_launch_control("Блокнот"));
        assert!(!is_launch_control("TEST_MAIN"));
    }
}
