//! Phase 0 — WH_KEYBOARD_LL 키 후크 PoC (순수 Rust, Tauri 결합 전)
//!
//! 검증 목표 (docs/PLAN.md §14 Phase 0)
//!   `asdf` = 4 / Shift+A = 2 / 한글 `한` = 3 / 꾹누름 = 0
//!   관리자 권한 앱의 입력이 잡히는지, 안티치트 게임 중 동작하는지,
//!   8시간 연속 무누수, 입력 지연 0.
//!
//! 설계 원칙 (PLAN.md §15 리스크)
//!   훅 콜백은 채널에 push만 하고 즉시 반환한다. 콜백이 LowLevelHooksTimeout
//!   (기본 300ms)을 넘기면 OS가 훅을 조용히 무시하고 시스템 전체 입력이
//!   밀린다. 집계·판정·출력은 전부 별도 스레드에서 한다.
//!
//! 이 크레이트는 Phase 0 전용 검증 도구다. 본체(apps/desktop/src-tauri)로
//! 옮길 때 재작성 대상이며, 여기서 확정하려는 건 코드가 아니라 **동작**이다.

use std::ffi::c_void;
use std::mem;
use std::ptr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{RecvTimeoutError, Sender, unbounded};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
};
use windows_sys::Win32::System::Console::{
    ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle, STD_OUTPUT_HANDLE,
    SetConsoleMode,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyNameTextW, MAPVK_VSC_TO_VK_EX, MapVirtualKeyW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED,
    LLKHF_LOWER_IL_INJECTED, MSG, SetWindowsHookExW, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP,
    WM_SYSKEYDOWN, WM_SYSKEYUP,
};

/// 물리 키 하나를 가리키는 정규화 인덱스의 개수.
///
/// 스캔코드는 8비트지만 확장 키(오른쪽 Ctrl, 숫자패드 Enter, 방향키 등)는
/// 0xE0 프리픽스로 구분된다. 프리픽스는 scanCode가 아니라 flags로 들어오므로
/// `0x100` 비트에 접어 넣어 512칸 배열 하나로 다룬다.
/// 히트맵이 물리 키 기준이어야 하므로(PLAN.md §4) 이 구분은 생략할 수 없다.
const KEY_SLOTS: usize = 512;

/// F12 스캔코드. PoC에서 카운터 리셋에 쓴다.
const SC_F12: u16 = 0x58;

/// 훅 콜백 → 집계 스레드로 넘기는 최소 단위.
struct KeyEvent {
    key: u16,
    down: bool,
    injected: bool,
}

static EVENT_TX: OnceLock<Sender<KeyEvent>> = OnceLock::new();
/// 콜백 1회 소요 시간의 최대값(ns). LowLevelHooksTimeout 여유를 정량 확인한다.
static MAX_CB_NANOS: AtomicU64 = AtomicU64::new(0);
/// 콜백 호출 횟수. 훅이 조용히 죽었는지 판별하는 신호로 쓴다.
static CB_CALLS: AtomicU64 = AtomicU64::new(0);
/// 채널이 막혀 유실된 이벤트. 0이 아니면 설계가 틀린 것이다.
static DROPPED: AtomicU64 = AtomicU64::new(0);

/// 저수준 키보드 훅 콜백.
///
/// 여기서 하는 일은 필드 몇 개를 읽어 채널에 넣는 것뿐이다. 판정도, 집계도,
/// 출력도 하지 않는다. 늘리지 말 것.
unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    }

    let start = Instant::now();

    let kb = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
    let msg = wparam as u32;
    let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
    let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;

    if down || up {
        let extended = kb.flags & LLKHF_EXTENDED != 0;
        let sc = (kb.scanCode & 0xFF) as u16;
        let key = if extended { 0x100 | sc } else { sc };

        // SendInput 합성 입력(AHK 매크로 대부분)은 카운트에서 제외하고
        // 별도 집계한다 — PLAN.md §9 어뷰징 방어.
        let injected = kb.flags & (LLKHF_INJECTED | LLKHF_LOWER_IL_INJECTED) != 0;

        if let Some(tx) = EVENT_TX.get()
            && tx
                .try_send(KeyEvent {
                    key,
                    down,
                    injected,
                })
                .is_err()
        {
            DROPPED.fetch_add(1, Ordering::Relaxed);
        }
    }

    let ns = start.elapsed().as_nanos() as u64;
    MAX_CB_NANOS.fetch_max(ns, Ordering::Relaxed);
    CB_CALLS.fetch_add(1, Ordering::Relaxed);

    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

/// 집계 스레드가 단독 소유하는 상태. 잠금이 필요 없도록 한 스레드에 가둔다.
struct Stats {
    /// 검증 대상. auto-repeat·injected를 제외한 물리 키 다운 횟수.
    total: u64,
    injected: u64,
    repeat_dropped: u64,
    watchdog_fixed: u64,
    per_key: [u64; KEY_SLOTS],
    down: [bool; KEY_SLOTS],
}

impl Stats {
    fn new() -> Self {
        Self {
            total: 0,
            injected: 0,
            repeat_dropped: 0,
            watchdog_fixed: 0,
            per_key: [0; KEY_SLOTS],
            down: [false; KEY_SLOTS],
        }
    }

    fn reset(&mut self) {
        // 눌림 상태는 보존한다. 리셋 시점에 눌려 있던 키를 함께 지우면
        // 그 키의 keyup이 왔을 때 상태가 어긋나고, 다음 keydown이
        // auto-repeat이 아닌데도 정상 카운트되어 검증이 흐려진다.
        let down = self.down;
        *self = Stats::new();
        self.down = down;
        MAX_CB_NANOS.store(0, Ordering::Relaxed);
        CB_CALLS.store(0, Ordering::Relaxed);
        DROPPED.store(0, Ordering::Relaxed);
    }

    fn apply(&mut self, ev: KeyEvent) {
        let i = ev.key as usize;
        if i >= KEY_SLOTS {
            return;
        }

        if ev.injected {
            if ev.down {
                self.injected += 1;
            }
            return;
        }

        if ev.down {
            // 저수준 훅에는 repeat 플래그가 없다. 키 상태 테이블을 직접
            // 유지해 "이미 down인 키의 keydown"을 auto-repeat으로 판정한다.
            if self.down[i] {
                self.repeat_dropped += 1;
            } else {
                self.down[i] = true;
                self.total += 1;
                self.per_key[i] += 1;
            }
        } else {
            self.down[i] = false;
        }
    }

    /// keyup 유실 보정.
    ///
    /// Win+L 잠금, UAC 승격, 데스크탑 전환 중에는 keyup을 놓칠 수 있다.
    /// 그대로 두면 그 키가 영구히 "눌림"으로 남아 이후 입력이 전부
    /// auto-repeat으로 오판되고 카운트가 조용히 샌다 — PLAN.md §15.
    fn watchdog(&mut self) {
        for i in 0..KEY_SLOTS {
            if !self.down[i] {
                continue;
            }
            let Some(vk) = scan_to_vk(i as u16) else {
                continue;
            };
            let pressed = unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 };
            if !pressed {
                self.down[i] = false;
                self.watchdog_fixed += 1;
            }
        }
    }
}

/// 정규화 인덱스 → 가상 키 코드. 워치독이 실제 눌림 상태를 되묻는 데 쓴다.
fn scan_to_vk(key: u16) -> Option<u16> {
    let sc = (key & 0xFF) as u32;
    let ext = key & 0x100 != 0;
    let arg = if ext { 0xE000 | sc } else { sc };
    let vk = unsafe { MapVirtualKeyW(arg, MAPVK_VSC_TO_VK_EX) };
    if vk == 0 { None } else { Some(vk as u16) }
}

/// 사람이 읽을 키 이름. OS가 주는 현지화 이름을 그대로 쓴다.
fn key_label(key: u16) -> String {
    let sc = (key & 0xFF) as i32;
    let ext = key & 0x100 != 0;
    let lparam = (sc << 16) | if ext { 1 << 24 } else { 0 };
    let mut buf = [0u16; 64];
    let len = unsafe { GetKeyNameTextW(lparam, buf.as_mut_ptr(), buf.len() as i32) };
    if len > 0 {
        String::from_utf16_lossy(&buf[..len as usize])
    } else {
        format!("SC{key:03X}")
    }
}

fn is_elevated() -> Option<bool> {
    unsafe {
        let mut token: HANDLE = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut returned = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut c_void,
            mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        );
        CloseHandle(token);
        if ok == 0 {
            None
        } else {
            Some(elevation.TokenIsElevated != 0)
        }
    }
}

fn working_set_bytes() -> Option<u64> {
    unsafe {
        let mut pmc: PROCESS_MEMORY_COUNTERS = mem::zeroed();
        pmc.cb = mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) == 0 {
            None
        } else {
            Some(pmc.WorkingSetSize as u64)
        }
    }
}

fn enable_vt() {
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0u32;
        if GetConsoleMode(h, &mut mode) != 0 {
            SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
        }
    }
}

fn fmt_uptime(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

fn render(stats: &Stats, started: Instant, elevated: Option<bool>) {
    let mut top: Vec<(u16, u64)> = stats
        .per_key
        .iter()
        .enumerate()
        .filter(|&(_, &c)| c > 0)
        .map(|(i, &c)| (i as u16, c))
        .collect();
    top.sort_by_key(|e| std::cmp::Reverse(e.1));
    let top_str = if top.is_empty() {
        "(없음)".to_string()
    } else {
        top.iter()
            .take(8)
            .map(|(k, c)| format!("{} {}", key_label(*k), c))
            .collect::<Vec<_>>()
            .join(" · ")
    };

    let held: Vec<String> = stats
        .down
        .iter()
        .enumerate()
        .filter(|&(_, &d)| d)
        .map(|(i, _)| key_label(i as u16))
        .collect();
    let held_str = if held.is_empty() {
        "(없음)".to_string()
    } else {
        held.join(" + ")
    };

    let max_ms = MAX_CB_NANOS.load(Ordering::Relaxed) as f64 / 1_000_000.0;
    let calls = CB_CALLS.load(Ordering::Relaxed);
    let dropped = DROPPED.load(Ordering::Relaxed);
    let rss = working_set_bytes()
        .map(|b| format!("{:.1} MB", b as f64 / 1_048_576.0))
        .unwrap_or_else(|| "?".into());
    let elev = match elevated {
        Some(true) => "관리자 (elevated)",
        Some(false) => "일반 사용자 — 관리자 권한 앱의 입력은 안 잡힘",
        None => "확인 실패",
    };

    // 화면 맨 위로 이동한 뒤 아래를 지운다. 스크롤 없이 제자리 갱신.
    print!("\x1b[H\x1b[J");
    println!("  TypingRank — Phase 0 키 후크 PoC");
    println!("  ─────────────────────────────────────────────────────");
    println!("  권한          {elev}");
    println!("  가동 시간     {}", fmt_uptime(started.elapsed()));
    println!();
    println!("  총 카운트     {}", stats.total);
    println!("  auto-repeat   {} 드롭", stats.repeat_dropped);
    println!("  injected      {} 제외", stats.injected);
    println!("  워치독 보정   {} 회", stats.watchdog_fixed);
    println!();
    println!("  콜백 최대     {max_ms:.3} ms   (OS 임계 300 ms)");
    println!("  콜백 호출     {calls} 회");
    println!("  이벤트 유실   {dropped} 개");
    println!("  메모리 RSS    {rss}");
    println!();
    println!("  상위 키       {top_str}");
    println!("  현재 눌림     {held_str}");
    println!();
    println!("  ─────────────────────────────────────────────────────");
    println!("  F12 리셋 · Ctrl+C 종료");
    println!();
    println!("  검증  asdf=4 · Shift+A=2 · 한(ㅎㅏㄴ)=3 · 꾹누름=0");
}

fn main() {
    enable_vt();

    let elevated = is_elevated();
    let (tx, rx) = unbounded::<KeyEvent>();
    EVENT_TX
        .set(tx)
        .unwrap_or_else(|_| unreachable!("EVENT_TX는 한 번만 설정된다"));

    // 집계 · 워치독 · 출력을 한 스레드에 모아 잠금 없이 소유한다.
    std::thread::spawn(move || {
        let started = Instant::now();
        let mut stats = Stats::new();
        let mut last_tick = Instant::now();
        render(&stats, started, elevated);

        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(ev) => {
                    // F12도 물리 입력이므로 먼저 카운트한 뒤 리셋한다.
                    let is_reset = ev.down && !ev.injected && ev.key == SC_F12;
                    stats.apply(ev);
                    if is_reset {
                        stats.reset();
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }

            if last_tick.elapsed() >= Duration::from_secs(1) {
                stats.watchdog();
                render(&stats, started, elevated);
                last_tick = Instant::now();
            }
        }
    });

    // WH_KEYBOARD_LL은 훅을 설치한 스레드의 메시지 루프에서 콜백이 호출된다.
    // 메인 스레드가 GetMessageW로 펌프를 돌지 않으면 콜백은 영원히 오지 않는다.
    unsafe {
        let hmod = GetModuleHandleW(ptr::null());
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), hmod, 0);
        if hook.is_null() {
            eprintln!("SetWindowsHookExW 실패 — 훅을 설치하지 못했다.");
            std::process::exit(1);
        }

        let mut msg: MSG = mem::zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {}
    }
}
