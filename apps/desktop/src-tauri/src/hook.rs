//! `WH_KEYBOARD_LL` 저수준 키보드 훅.
//!
//! **이 모듈은 판정을 하지 않는다.** 콜백은 필드 몇 개를 읽어 채널에 넣고 즉시
//! 반환한다. auto-repeat 판정·집계·저장은 전부 [`crate::counter`]가 별도
//! 스레드에서 처리한다.
//!
//! 콜백이 `LowLevelHooksTimeout`(기본 300ms)을 넘기면 OS가 훅을 **조용히
//! 제거하고** 시스템 전체 입력이 밀린다. Phase 0 실측 최대 79µs로 여유는
//! 충분하지만(PLAN.md §14), 시스템 부하로 한 번 넘기면 이후로 카운트가 0이
//! 되므로 [주기적 재설치](Command::Reinstall)로 방어한다.

use std::mem;
use std::ptr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;

use crossbeam_channel::{Receiver, Sender, unbounded};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, HHOOK, KBDLLHOOKSTRUCT, KillTimer, LLKHF_EXTENDED,
    LLKHF_INJECTED, LLKHF_LOWER_IL_INJECTED, MSG, PostThreadMessageW, SetTimer,
    SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_APP, WM_KEYDOWN, WM_KEYUP,
    WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER,
};

/// 훅이 살아 있어도 OS가 조용히 떼어냈을 수 있다. 감지할 API가 없으므로
/// 주기적으로 떼었다 다시 건다. 재설치 비용은 마이크로초 단위고, 그 사이
/// 이벤트를 놓칠 확률보다 훅이 죽은 채 도는 쪽이 훨씬 비싸다.
const REINSTALL_INTERVAL_MS: u32 = 5 * 60 * 1000;
const TIMER_ID: usize = 1;

const MSG_PAUSE: u32 = WM_APP + 1;
const MSG_RESUME: u32 = WM_APP + 2;

/// 훅 콜백 → 집계 스레드로 넘기는 최소 단위.
pub struct RawEvent {
    /// 확장 키(0xE0 프리픽스)를 `0x100` 비트에 접어 넣은 정규화 인덱스.
    /// 히트맵이 물리 키 기준이어야 하므로(PLAN.md §4) 이 구분은 생략할 수 없다.
    pub key: u16,
    pub down: bool,
    /// `SendInput` 합성 입력. 카운트에서 제외하고 따로 센다 (PLAN.md §9).
    pub injected: bool,
}

static EVENT_TX: OnceLock<Sender<RawEvent>> = OnceLock::new();
static MAX_CB_NANOS: AtomicU64 = AtomicU64::new(0);
static CB_CALLS: AtomicU64 = AtomicU64::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static REINSTALLS: AtomicU64 = AtomicU64::new(0);
static PAUSED: AtomicBool = AtomicBool::new(false);

/// 훅 콜백. 늘리지 말 것 — 여기서 하는 일이 늘어나는 만큼 시스템 입력 지연
/// 위험이 커진다.
unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    }

    let start = std::time::Instant::now();

    let kb = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
    let msg = wparam as u32;
    let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
    let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;

    if down || up {
        let sc = (kb.scanCode & 0xFF) as u16;
        let key = if kb.flags & LLKHF_EXTENDED != 0 {
            0x100 | sc
        } else {
            sc
        };
        let injected = kb.flags & (LLKHF_INJECTED | LLKHF_LOWER_IL_INJECTED) != 0;

        if let Some(tx) = EVENT_TX.get()
            && tx
                .try_send(RawEvent {
                    key,
                    down,
                    injected,
                })
                .is_err()
        {
            DROPPED.fetch_add(1, Ordering::Relaxed);
        }
    }

    MAX_CB_NANOS.fetch_max(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
    CB_CALLS.fetch_add(1, Ordering::Relaxed);

    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

/// 훅 스레드 제어 핸들. 트레이의 일시정지가 이걸 호출한다.
#[derive(Clone, Copy)]
pub struct HookHandle {
    thread_id: u32,
}

impl HookHandle {
    /// 일시정지 — **플래그로 무시하는 게 아니라 훅 자체를 뗀다.**
    ///
    /// 비밀번호를 칠 때 끌 수 있어야 앱을 깔 마음이 든다(PLAN.md §10). 그렇다면
    /// "무시한다"가 아니라 "받지 않는다"여야 한다. 오픈소스라 이 차이를 유저가
    /// 직접 확인할 수 있고, 그게 이 프로젝트의 신뢰 장치다.
    pub fn pause(&self) {
        unsafe { PostThreadMessageW(self.thread_id, MSG_PAUSE, 0, 0) };
    }

    pub fn resume(&self) {
        unsafe { PostThreadMessageW(self.thread_id, MSG_RESUME, 0, 0) };
    }
}

/// 콜백 건강 지표. 8시간 무누수 검증을 Phase 1로 이관하면서 본체에 남긴
/// 계측이다 (PLAN.md §14).
pub struct Health {
    pub max_cb_us: u64,
    pub cb_calls: u64,
    pub dropped: u64,
    pub reinstalls: u64,
}

pub fn health() -> Health {
    Health {
        max_cb_us: MAX_CB_NANOS.load(Ordering::Relaxed) / 1_000,
        cb_calls: CB_CALLS.load(Ordering::Relaxed),
        dropped: DROPPED.load(Ordering::Relaxed),
        reinstalls: REINSTALLS.load(Ordering::Relaxed),
    }
}

pub fn is_paused() -> bool {
    PAUSED.load(Ordering::Relaxed)
}

unsafe fn install() -> HHOOK {
    unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_hook),
            GetModuleHandleW(ptr::null()),
            0,
        )
    }
}

/// 훅 스레드를 띄우고 원시 이벤트 채널과 제어 핸들을 돌려준다.
///
/// 훅은 **메시지 루프가 있는 스레드**에 설치해야 한다. Tauri가 메인 스레드를
/// 소유하므로 전용 스레드를 판다 — Phase 0 오버레이 스파이크에서 검증된 구조다.
pub fn start() -> (HookHandle, Receiver<RawEvent>) {
    let (ev_tx, ev_rx) = unbounded::<RawEvent>();
    let _ = EVENT_TX.set(ev_tx);

    let (id_tx, id_rx) = crossbeam_channel::bounded::<u32>(1);

    thread::spawn(move || unsafe {
        let _ = id_tx.send(GetCurrentThreadId());

        let mut hook = install();
        if hook.is_null() {
            eprintln!("SetWindowsHookExW 실패 — 카운트가 동작하지 않는다");
            return;
        }
        SetTimer(
            ptr::null_mut::<HWND>() as HWND,
            TIMER_ID,
            REINSTALL_INTERVAL_MS,
            None,
        );

        let mut msg: MSG = mem::zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            match msg.message {
                // 주기적 재설치. OS가 훅을 떼어 갔는지 알 방법이 없으므로
                // 살아 있든 아니든 다시 건다.
                WM_TIMER if !PAUSED.load(Ordering::Relaxed) => {
                    UnhookWindowsHookEx(hook);
                    hook = install();
                    REINSTALLS.fetch_add(1, Ordering::Relaxed);
                }
                MSG_PAUSE if !PAUSED.load(Ordering::Relaxed) => {
                    UnhookWindowsHookEx(hook);
                    hook = ptr::null_mut();
                    PAUSED.store(true, Ordering::Relaxed);
                }
                MSG_RESUME if PAUSED.load(Ordering::Relaxed) => {
                    hook = install();
                    PAUSED.store(false, Ordering::Relaxed);
                }
                _ => {}
            }
        }

        KillTimer(ptr::null_mut::<HWND>() as HWND, TIMER_ID);
        if !hook.is_null() {
            UnhookWindowsHookEx(hook);
        }
    });

    let thread_id = id_rx.recv().expect("훅 스레드가 시작되지 않았다");
    (HookHandle { thread_id }, ev_rx)
}
