//! 투명 항상-위 오버레이 (PLAN.md §8).
//!
//! Phase 0 곁다리 스파이크(`poc/overlay`)에서 검증한 내용을 본체로 옮긴 것이다.
//! 스파이크는 자체 후크를 들고 있었지만 여기서는 대시보드·트레이와 **같은
//! 스냅샷 스트림**을 쓴다. 표시 수단마다 카운터를 따로 두면 셋이 서로 다른
//! 숫자를 보여주는 순간이 반드시 온다.
//!
//! 실측으로 확정된 것 — 승격된 프로세스에서 투명 창이 정상 표시되고,
//! borderless 게임 위에 뜨며, 게임에 포커스가 있는 동안에도 갱신된다.
//! exclusive fullscreen에서 안 보이는 것은 OS 표시 규칙이라 앱이 해결할 수 없다.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, Runtime};

/// 표시 타입. 게임 중엔 숫자만, 작업 중엔 지표까지.
#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Variant {
    Minimal,
    Normal,
    Maximum,
}

impl Variant {
    fn next(self) -> Self {
        match self {
            Variant::Minimal => Variant::Normal,
            Variant::Normal => Variant::Maximum,
            Variant::Maximum => Variant::Minimal,
        }
    }

    /// 창 크기는 **CSS가 아니라 여기가 정한다.**
    ///
    /// 창이 내용보다 작으면 잘리고, 크면 클릭 통과를 껐을 때 빈 영역이
    /// 마우스를 먹는다. `ui/overlay.html`의 레이아웃을 고치면 이 수치도
    /// 같이 봐야 한다 — 둘이 어긋나면 바로 티가 난다.
    ///
    /// 계산 근거(기본 타입): 표면 여백·테두리·패딩 46 + eyebrow 13 + 숫자
    /// 34 + 구분선 있는 지표 2행 66 + 힌트 23 ≈ 183. **창은 투명이라 남는
    /// 높이는 보이지 않지만 모자라면 곧바로 잘리므로** 한쪽으로만 여유를 준다.
    fn size(self) -> LogicalSize<f64> {
        match self {
            Variant::Minimal => LogicalSize::new(176.0, 100.0),
            Variant::Normal => LogicalSize::new(300.0, 200.0),
            Variant::Maximum => LogicalSize::new(300.0, 248.0),
        }
    }
}

/// 오버레이의 현재 상태. 트레이 메뉴와 단축키가 같은 값을 본다.
#[derive(Serialize, Clone, Copy)]
pub struct OverlayState {
    pub visible: bool,
    pub variant: Variant,
}

pub struct Shared(pub Mutex<OverlayState>);

/// 창을 초기 상태로 맞춘다. `setup`에서 한 번 호출한다.
pub fn init<R: Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    let state = OverlayState {
        visible: true,
        variant: Variant::Normal,
    };

    if let Some(w) = app.get_webview_window("overlay") {
        // 오버레이의 기본은 클릭 통과다. 아래 게임·에디터가 마우스를 그대로
        // 받아야 오버레이라고 부를 수 있다. 위치 드래그 이동(v2)을 붙이려면
        // 이걸 잠시 끄는 이동 모드가 선행돼야 한다 — §8.
        w.set_ignore_cursor_events(true)?;
        w.set_size(state.variant.size())?;
    }

    app.manage(Shared(Mutex::new(state)));
    Ok(())
}

pub fn toggle_visible<R: Runtime>(app: &AppHandle<R>) {
    let Some(w) = app.get_webview_window("overlay") else {
        return;
    };
    let state = {
        let shared = app.state::<Shared>();
        let mut s = shared.0.lock().unwrap();
        s.visible = !s.visible;
        *s
    };
    let _ = if state.visible { w.show() } else { w.hide() };
    broadcast(app, state);
}

pub fn cycle_variant<R: Runtime>(app: &AppHandle<R>) {
    let Some(w) = app.get_webview_window("overlay") else {
        return;
    };
    let state = {
        let shared = app.state::<Shared>();
        let mut s = shared.0.lock().unwrap();
        s.variant = s.variant.next();
        *s
    };
    let _ = w.set_size(state.variant.size());
    broadcast(app, state);
}

/// 창이 `setup`보다 먼저 로드를 시작할 수 있어, 상태가 아직 없을 때도
/// 안전하게 기본값을 돌려준다.
pub fn current<R: Runtime>(app: &AppHandle<R>) -> OverlayState {
    app.try_state::<Shared>()
        .map(|s| *s.0.lock().unwrap())
        .unwrap_or(OverlayState {
            visible: true,
            variant: Variant::Normal,
        })
}

pub fn broadcast<R: Runtime>(app: &AppHandle<R>, state: OverlayState) {
    let _ = app.emit("overlay-state", state);
}
