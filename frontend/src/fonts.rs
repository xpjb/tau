//! Platform fonts are part of the OS, not the Android/Windows download.
use sanscale::{FontData, FontHandle, TextService};
use std::{collections::HashMap, path::PathBuf};
use tau_markdown::Faces;

#[derive(Default)]
struct Files(HashMap<PathBuf, FontData>);
impl Files {
    fn map(
        &mut self,
        text: &mut TextService,
        path: PathBuf,
        index: u32,
    ) -> Result<FontHandle, String> {
        let data = if let Some(data) = self.0.get(&path) {
            data.clone()
        } else {
            let data =
                sanscale::read_font_file(&path).map_err(|e| e.to_string())?;
            self.0.insert(path, data.clone());
            data
        };
        text.map_font(data, index).map_err(|e| e.to_string())
    }
}

pub fn load(text: &mut TextService) -> Result<Faces, String> {
    let mut files = Files::default();
    let mut fallback = vec![];
    #[cfg(target_os = "android")]
    for sample in ["漢", "한", "😀", "ع", "अ", "ก"] {
        if let Ok((path, index)) = android::matched(c"sans-serif", 0, sample)
            && let Ok(font) = files.map(text, path, index)
            && !fallback.contains(&font)
        {
            fallback.push(font);
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        #[cfg(windows)]
        let paths = {
            let root =
                PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()))
                    .join("Fonts");
            ["msyh.ttc", "malgun.ttf", "seguiemj.ttf"].map(|name| root.join(name))
        };
        #[cfg(not(windows))]
        let paths = [
            PathBuf::from("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc"),
            PathBuf::from("/usr/share/fonts/noto/NotoColorEmoji.ttf"),
        ];
        for path in paths {
            if let Ok(font) = files.map(text, path, 0) {
                fallback.push(font);
            }
        }
    }
    let mut chains = vec![];
    for index in 0..8 {
        let primary = primary(text, &mut files, index)?;
        let mut chain = vec![primary];
        chain.extend(fallback.iter().copied().filter(|font| *font != primary));
        chains.push(text.register_chain(&chain).map_err(|e| e.to_string())?);
    }
    Ok(Faces {
        prose: chains[..4].try_into().unwrap(),
        mono: chains[4..].try_into().unwrap(),
    })
}

#[cfg(target_os = "android")]
fn primary(text: &mut TextService, files: &mut Files, index: usize) -> Result<FontHandle, String> {
    // Prefer the static Roboto faces where present. The current text engine has
    // no variation-axis API, so these retain the explicit bold/italic outlines.
    let name = if index < 4 {
        [
            "Roboto-Regular.ttf",
            "Roboto-Bold.ttf",
            "Roboto-Italic.ttf",
            "Roboto-BoldItalic.ttf",
        ][index]
    } else {
        [
            "RobotoMono-Regular.ttf",
            "RobotoMono-Bold.ttf",
            "RobotoMono-Italic.ttf",
            "RobotoMono-BoldItalic.ttf",
        ][index - 4]
    };
    if let Ok(font) = files.map(text, PathBuf::from("/system/fonts").join(name), 0) {
        return Ok(font);
    }
    // Don't depend on OEM file names or assume that a matched face is TTC index 0.
    let family = if index < 4 {
        c"sans-serif"
    } else {
        c"monospace"
    };
    let (path, face) = android::matched(family, index % 4, "A")?;
    files.map(text, path, face)
}

#[cfg(windows)]
fn primary(text: &mut TextService, _files: &mut Files, index: usize) -> Result<FontHandle, String> {
    let data = windows::face(if index < 4 { "Segoe UI" } else { "Consolas" }, index % 4)?;
    text.map_font(data, 0).map_err(|e| e.to_string())
}

// The development Linux build still has a deterministic fallback on machines
// without fonts. None of these bytes is compiled into Android or Windows.
#[cfg(not(any(target_os = "android", windows)))]
fn primary(text: &mut TextService, files: &mut Files, index: usize) -> Result<FontHandle, String> {
    let name = [
        "NotoSans-Regular.ttf",
        "NotoSans-Bold.ttf",
        "NotoSans-Italic.ttf",
        "NotoSans-BoldItalic.ttf",
        "NotoSansMono-Regular.ttf",
        "NotoSansMono-Bold.ttf",
        "NotoSansMono-Italic.ttf",
        "NotoSansMono-BoldItalic.ttf",
    ][index];
    if let Ok(font) = files.map(text, PathBuf::from("/usr/share/fonts/noto").join(name), 0) {
        return Ok(font);
    }
    let bundled: [&'static [u8]; 8] = [
        include_bytes!("../assets/DejaVuSans.ttf"),
        include_bytes!("../assets/DejaVuSans-Bold.ttf"),
        include_bytes!("../assets/DejaVuSans-Oblique.ttf"),
        include_bytes!("../assets/DejaVuSans-BoldOblique.ttf"),
        include_bytes!("../assets/DejaVuSansMono.ttf"),
        include_bytes!("../assets/DejaVuSansMono-Bold.ttf"),
        include_bytes!("../assets/DejaVuSansMono-Oblique.ttf"),
        include_bytes!("../assets/DejaVuSansMono-BoldOblique.ttf"),
    ];
    text.map_font(std::sync::Arc::new(bundled[index]), 0)
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "android")]
mod android {
    use std::{
        ffi::{CStr, c_char, c_void},
        path::PathBuf,
    };

    // NDK font_matcher.h/font.h, available since API 29 (our minimum SDK).
    #[link(name = "android")]
    unsafe extern "C" {
        fn AFontMatcher_create() -> *mut c_void;
        fn AFontMatcher_destroy(matcher: *mut c_void);
        fn AFontMatcher_setStyle(matcher: *mut c_void, weight: u16, italic: bool);
        fn AFontMatcher_match(
            matcher: *const c_void,
            family: *const c_char,
            text: *const u16,
            length: u32,
            run_length: *mut u32,
        ) -> *mut c_void;
        fn AFont_getFontFilePath(font: *const c_void) -> *const c_char;
        fn AFont_getCollectionIndex(font: *const c_void) -> usize;
        fn AFont_close(font: *mut c_void);
    }

    pub fn matched(family: &CStr, style: usize, sample: &str) -> Result<(PathBuf, u32), String> {
        let sample: Vec<u16> = sample.encode_utf16().collect();
        if sample.is_empty() {
            return Err("Empty font sample".into());
        }
        // SAFETY: all pointers are NDK-owned objects, strings/buffers outlive the
        // calls, and every successful allocation is released before returning.
        unsafe {
            let matcher = AFontMatcher_create();
            if matcher.is_null() {
                return Err("Android font matcher unavailable".into());
            }
            AFontMatcher_setStyle(
                matcher,
                if style & 1 != 0 { 700 } else { 400 },
                style & 2 != 0,
            );
            let font = AFontMatcher_match(
                matcher,
                family.as_ptr(),
                sample.as_ptr(),
                sample.len() as u32,
                std::ptr::null_mut(),
            );
            AFontMatcher_destroy(matcher);
            if font.is_null() {
                return Err("Android system font unavailable".into());
            }
            let path = AFont_getFontFilePath(font);
            let path = if path.is_null() {
                None
            } else {
                Some(PathBuf::from(
                    CStr::from_ptr(path).to_string_lossy().into_owned(),
                ))
            };
            let index = AFont_getCollectionIndex(font);
            AFont_close(font);
            let path = path.ok_or("Android system font has no readable file")?;
            let index = u32::try_from(index).map_err(|e| e.to_string())?;
            Ok((path, index))
        }
    }
}

#[cfg(windows)]
mod windows {
    use sanscale::FontData;
    use std::{ptr::null_mut, sync::Arc};
    use windows_sys::Win32::Graphics::Gdi::*;

    pub fn face(family: &str, style: usize) -> Result<FontData, String> {
        let family: Vec<u16> = family.encode_utf16().chain(Some(0)).collect();
        // Ask GDI for the actual installed face rather than guessing filenames.
        // It also resolves a usable substitute if a requested family is absent.
        // SAFETY: the DC/font are private to this call. Restore the selected
        // object before destroying them; neither handle nor buffer escapes.
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            if dc.is_null() {
                return Err("Windows font DC unavailable".into());
            }
            let font = CreateFontW(
                -16,
                0,
                0,
                0,
                if style & 1 != 0 { 700 } else { 400 },
                u32::from(style & 2 != 0),
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_TT_ONLY_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                DEFAULT_QUALITY as u32,
                DEFAULT_PITCH as u32,
                family.as_ptr(),
            );
            if font.is_null() {
                DeleteDC(dc);
                return Err("Windows system font unavailable".into());
            }
            let previous = SelectObject(dc, font);
            let size = GetFontData(dc, 0, 0, null_mut(), 0);
            let result = if size == u32::MAX || size == 0 {
                Err("Windows system font data unavailable".into())
            } else {
                let mut bytes = vec![0_u8; size as usize];
                if GetFontData(dc, 0, 0, bytes.as_mut_ptr().cast(), size) == size {
                    Ok(Arc::new(bytes) as FontData)
                } else {
                    Err("Could not read Windows system font".into())
                }
            };
            SelectObject(dc, previous);
            DeleteObject(font);
            DeleteDC(dc);
            result
        }
    }
}
