//! Распознавание текста на снимке окна — для защиты боевых баз и других запретных экранов.
//! Windows: встроенный Windows.Media.Ocr (русский, затем языки профиля). Другие ОС — пока нет.

use image::RgbaImage;

/// Текст на изображении. Err — распознавание недоступно или не удалось.
#[cfg(windows)]
pub fn recognize(img: &RgbaImage) -> Result<String, String> {
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
    let scaled;
    let src = if img.width() > max || img.height() > max {
        let k = max as f64 / img.width().max(img.height()) as f64;
        scaled = image::imageops::resize(
            img,
            ((img.width() as f64 * k) as u32).max(1),
            ((img.height() as f64 * k) as u32).max(1),
            image::imageops::FilterType::Triangle,
        );
        &scaled
    } else {
        img
    };
    let buf = CryptographicBuffer::CreateFromByteArray(src.as_raw()).map_err(e)?;
    let bmp = SoftwareBitmap::CreateCopyFromBuffer(&buf, BitmapPixelFormat::Rgba8, src.width() as i32, src.height() as i32)
        .map_err(e)?;
    let res = engine.RecognizeAsync(&bmp).map_err(e)?.join().map_err(e)?;
    Ok(res.Text().map_err(e)?.to_string())
}

#[cfg(not(windows))]
pub fn recognize(_img: &RgbaImage) -> Result<String, String> {
    Err("распознавание текста на этой ОС пока не поддерживается".into())
}

/// Для сравнения: нижний регистр, латинские двойники и цифры 3/0 → русские буквы,
/// без пробелов и знаков. Ошибка распознавания не должна пропустить запретное название.
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

/// Первая запретная фраза, найденная в тексте.
pub fn find_protected<'a>(text: &str, phrases: &'a [String]) -> Option<&'a str> {
    let hay = normalize(text);
    phrases
        .iter()
        .find(|p| {
            let n = normalize(p);
            !n.is_empty() && hay.contains(&n)
        })
        .map(|p| p.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_matching() {
        let p = vec!["ТОО ФРК РОЗНИЦА".to_string()];
        assert!(find_protected("Запуск 1С\nТОО ФРК РОЗНИЦА 8.3\nTEST_MAIN", &p).is_some());
        assert!(find_protected("TOO ФPK P0ЗНИЦА", &p).is_some()); // латиница и ноль от распознавания
        assert!(find_protected("тоо  фрк\nрозница", &p).is_some());
        assert!(find_protected("ТОО ФРК Р03НИЦА", &p).is_some());
        assert!(find_protected("TEST_MAIN\nWorkTime_TEST\nТОО ФРК БУХ 8.3\nРОЗНИЦА 8.3", &p).is_none());
    }
}
