//! 트레이 아이콘 — MVP의 실시간 표시 수단 (PLAN.md §8).
//!
//! 아이콘에 오늘 카운트를 **직접 그려 넣는다.** 추가 창·권한·패키징 변경이
//! 0이라 가장 싸고 가장 안 깨지는 방식이다.
//!
//! 16×16 아이콘에 `45,231`은 읽히지 않으므로 **4자리를 넘으면 축약**하고
//! 전체 숫자는 툴팁에 넣는다(§8). 일시정지 중에는 숫자 대신 정지 표시를
//! 그린다 — 정지된 줄 모르고 하루치를 날리는 게 최악의 경험이다(§10).

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager, Runtime};

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS,
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
    DIB_RGB_COLORS, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW,
    HDC, OUT_TT_PRECIS, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};

use crate::counter::Snapshot;
use crate::hook::HookHandle;

/// 32×32로 그려 두면 고DPI 작업 표시줄에서 뭉개지지 않는다.
const ICON: i32 = 32;
const ICON_PX: usize = (ICON * ICON) as usize;

/// 숫자 색. 작업 표시줄은 다크가 기본이지만 라이트일 수도 있어, 양쪽에서
/// 모두 읽히는 accent 계열을 쓴다. 흰색·검정은 한쪽에서 사라진다.
const FG: (u8, u8, u8) = (0x34, 0xd3, 0x99);
const FG_PAUSED: (u8, u8, u8) = (0xa1, 0xa1, 0xaa);

/// 트레이에 그릴 문자열.
///
/// 4자리까지는 그대로, 그 위는 `45k` / `1.2M`. 이 규칙을 코드에서 즉흥적으로
/// 정하면 자릿수가 넘어가는 시점마다 재량이 갈리므로 §8에 못박아 뒀다.
fn format_count(n: u64) -> String {
    match n {
        0..=9_999 => n.to_string(),
        10_000..=999_999 => format!("{}k", n / 1_000),
        _ => {
            let m = n as f64 / 1_000_000.0;
            if m < 10.0 {
                format!("{m:.1}M")
            } else {
                format!("{}M", m.round() as u64)
            }
        }
    }
}

/// 흰 글자를 검은 배경에 그린 뒤 **밝기를 그대로 알파로 쓴다.**
///
/// GDI 텍스트 출력은 알파 채널을 건드리지 않아서, DIB를 그대로 RGBA로 읽으면
/// 전부 투명해진다. 안티에일리어싱된 밝기를 알파로 승격시키면 별도 마스크
/// 없이 부드러운 글자가 나온다.
fn render_text(text: &str, fg: (u8, u8, u8)) -> Option<Vec<u8>> {
    // 글자 수에 따라 크기를 줄인다. 4글자를 24px로 그리면 서로 붙어버린다.
    let font_px = match text.chars().count() {
        0..=2 => 26,
        3 => 20,
        _ => 16,
    };

    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let mut face: Vec<u16> = "Segoe UI\0".encode_utf16().collect();

    unsafe {
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            return None;
        }

        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: ICON,
            // 음수 = top-down. 양수면 세로로 뒤집힌 아이콘이 나온다.
            biHeight: -ICON,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };

        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if bmp.is_null() || bits.is_null() {
            DeleteDC(dc);
            return None;
        }
        let old_bmp = SelectObject(dc, bmp as _);

        let font = CreateFontW(
            font_px,
            0,
            0,
            0,
            600, // semibold — 가이드가 허용하는 3단계 중 가장 무거운 값
            0,
            0,
            0,
            DEFAULT_CHARSET.into(),
            OUT_TT_PRECIS.into(),
            CLIP_DEFAULT_PRECIS.into(),
            // ClearType은 서브픽셀이라 색 번짐이 생긴다. 밝기를 알파로 쓰는
            // 이 방식과 맞지 않으므로 일반 안티에일리어싱을 쓴다.
            ANTIALIASED_QUALITY.into(),
            DEFAULT_PITCH.into(),
            face.as_mut_ptr(),
        );
        let old_font = SelectObject(dc, font as _);

        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, 0x00FF_FFFF); // 흰색 (COLORREF는 0x00BBGGRR)

        let mut rect = RECT {
            left: 0,
            top: 0,
            right: ICON,
            bottom: ICON,
        };
        DrawTextW(
            dc as HDC,
            wide.as_mut_ptr(),
            wide.len() as i32,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );

        let src = std::slice::from_raw_parts(bits as *const u8, ICON_PX * 4);
        let mut rgba = vec![0u8; ICON_PX * 4];
        for i in 0..ICON_PX {
            // DIB는 메모리에 BGRA 순서로 놓인다.
            let lum = src[i * 4].max(src[i * 4 + 1]).max(src[i * 4 + 2]);
            rgba[i * 4] = fg.0;
            rgba[i * 4 + 1] = fg.1;
            rgba[i * 4 + 2] = fg.2;
            rgba[i * 4 + 3] = lum;
        }

        SelectObject(dc, old_font);
        DeleteObject(font as _);
        SelectObject(dc, old_bmp);
        DeleteObject(bmp as _);
        DeleteDC(dc);

        Some(rgba)
    }
}

/// 일시정지 표시 — 막대 두 개. GDI 없이 버퍼에 직접 그린다.
///
/// 숫자를 흐리게 하는 정도로는 "정지됐다"가 전달되지 않는다. 형태가 완전히
/// 달라야 흘끗 봐도 안다.
fn render_paused() -> Vec<u8> {
    let mut rgba = vec![0u8; ICON_PX * 4];
    let put = |rgba: &mut Vec<u8>, x: i32, y: i32| {
        let i = (y * ICON + x) as usize * 4;
        rgba[i] = FG_PAUSED.0;
        rgba[i + 1] = FG_PAUSED.1;
        rgba[i + 2] = FG_PAUSED.2;
        rgba[i + 3] = 0xff;
    };
    for y in 7..25 {
        for x in 9..14 {
            put(&mut rgba, x, y);
        }
        for x in 18..23 {
            put(&mut rgba, x, y);
        }
    }
    rgba
}

/// 트레이를 만든다. 반환된 핸들로 아이콘·툴팁을 갱신한다.
pub fn build<R: Runtime>(app: &tauri::App<R>) -> tauri::Result<TrayIcon<R>> {
    let open = MenuItem::with_id(app, "open", "대시보드 열기", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "일시정지", true, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &PredefinedMenuItem::separator(app)?,
            &pause,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let tray = TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().expect("번들 아이콘 없음"))
        .tooltip("TypingRank")
        .menu(&menu)
        // 좌클릭은 대시보드를 연다. 상주 앱에서 가장 잦은 동작이라
        // 메뉴를 한 단계 거치게 하지 않는다.
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "open" => show_dashboard(app),
            "pause" => {
                let hook = app.state::<HookHandle>();
                // CheckMenuItem의 표시 상태가 아니라 훅의 실제 상태를 뒤집는다.
                // 둘이 갈라지면 "정지했는데 세고 있다"가 된다.
                if crate::hook::is_paused() {
                    hook.resume();
                } else {
                    hook.pause();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_dashboard(tray.app_handle());
            }
        })
        .build(app)?;

    app.manage(pause);
    Ok(tray)
}

fn show_dashboard<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("dashboard") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// 스냅샷을 트레이에 반영한다. 호출 빈도는 호출자가 제한한다 —
/// 키마다 비트맵을 다시 그리면 상주 앱의 이점이 없어진다.
pub fn update<R: Runtime>(app: &AppHandle<R>, tray: &TrayIcon<R>, snap: &Snapshot) {
    let rgba = if snap.paused {
        Some(render_paused())
    } else {
        render_text(&format_count(snap.today), FG)
    };
    if let Some(rgba) = rgba {
        let _ = tray.set_icon(Some(Image::new_owned(rgba, ICON as u32, ICON as u32)));
    }

    let tip = if snap.paused {
        "TypingRank — 일시정지 중 (키 후크 해제됨)".to_string()
    } else {
        format!("TypingRank — 오늘 {} · 누적 {}", snap.today, snap.total)
    };
    let _ = tray.set_tooltip(Some(&tip));

    // 메뉴 체크 표시를 훅의 실제 상태에 맞춘다.
    if let Some(item) = app.try_state::<CheckMenuItem<R>>() {
        let _ = item.set_checked(snap.paused);
    }
}
