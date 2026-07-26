//! 집계 스레드 — 카운팅 규칙(PLAN.md §4)이 실제로 구현되는 곳.
//!
//! 물리 키 다운 1회 = 1, auto-repeat 제외, injected 제외, 모든 키 포함.
//! 상태를 이 스레드 하나에 가둬 잠금을 없앤다.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, TryRecvError, unbounded};
use serde::Serialize;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MAPVK_VSC_TO_VK_EX, MapVirtualKeyW,
};

use crate::hook::{self, RawEvent};
use crate::store::Store;

const KEY_SLOTS: usize = 512;
const FLUSH_INTERVAL: Duration = Duration::from_secs(10);
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(1);

/// 워치독이 키를 "눌린 채 방치됐다"고 판단하기까지 기다리는 시간.
///
/// **이 지연이 핵심이다.** Phase 0 PoC는 이게 없어서 채널에 아직 대기 중인
/// keyup을 워치독이 앞질러 보고 유실로 오판했다 — 203 카운트에 16회(§14).
/// 카운트는 틀리지 않았지만 보정 횟수가 건강 지표로서 쓸모없어졌다.
/// 사람이 1초 넘게 누르고 있는 키는 auto-repeat 대상이지 유실이 아니다.
const STUCK_THRESHOLD: Duration = Duration::from_secs(1);

/// 대시보드·트레이·오버레이가 함께 보는 단일 상태.
#[derive(Serialize, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub today: u64,
    pub total: u64,
    pub best_day: u64,
    pub best_day_date: String,
    /// 이번 실행에서 센 것. 앱이 살아 있음을 눈으로 확인하는 용도.
    pub session: u64,
    pub repeat_dropped: u64,
    pub injected: u64,
    pub watchdog_fixed: u64,
    pub paused: bool,
    // --- 건강 지표 (PLAN.md §14, 8시간 무누수 이관분) ---
    pub max_cb_us: u64,
    pub cb_calls: u64,
    pub dropped: u64,
    pub reinstalls: u64,
}

struct State {
    down: [bool; KEY_SLOTS],
    last_down_at: [Option<Instant>; KEY_SLOTS],
    /// 아직 SQLite에 반영하지 않은 델타.
    pending_total: u64,
    pending_keys: HashMap<u16, u64>,
    snap: Snapshot,
    date: String,
}

fn local_date() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// 정규화 인덱스 → 가상 키 코드. 워치독이 실제 눌림 상태를 되묻는 데 쓴다.
fn scan_to_vk(key: u16) -> Option<u16> {
    let sc = (key & 0xFF) as u32;
    let arg = if key & 0x100 != 0 { 0xE000 | sc } else { sc };
    match unsafe { MapVirtualKeyW(arg, MAPVK_VSC_TO_VK_EX) } {
        0 => None,
        vk => Some(vk as u16),
    }
}

impl State {
    fn new(store: &Store, date: String) -> Self {
        let totals = store.totals(&date).unwrap_or(crate::store::Totals {
            total: 0,
            today: 0,
            best_day: 0,
            best_day_date: String::new(),
        });

        Self {
            down: [false; KEY_SLOTS],
            last_down_at: [None; KEY_SLOTS],
            pending_total: 0,
            pending_keys: HashMap::new(),
            snap: Snapshot {
                today: totals.today,
                total: totals.total,
                best_day: totals.best_day,
                best_day_date: totals.best_day_date,
                ..Default::default()
            },
            date,
        }
    }

    fn apply(&mut self, ev: &RawEvent) {
        let i = ev.key as usize;
        if i >= KEY_SLOTS {
            return;
        }

        if ev.injected {
            if ev.down {
                self.snap.injected += 1;
            }
            return;
        }

        if !ev.down {
            self.down[i] = false;
            self.last_down_at[i] = None;
            return;
        }

        // 저수준 훅에는 repeat 플래그가 없다. 키 상태 테이블을 직접 유지해
        // "이미 down인 키의 keydown"을 auto-repeat으로 판정한다 (§4).
        if self.down[i] {
            self.snap.repeat_dropped += 1;
            return;
        }

        self.down[i] = true;
        self.last_down_at[i] = Some(Instant::now());

        self.snap.today += 1;
        self.snap.total += 1;
        self.snap.session += 1;
        self.pending_total += 1;
        *self.pending_keys.entry(ev.key).or_insert(0) += 1;

        // 개인 기록은 오늘이 넘어서는 순간 바로 갱신된다.
        if self.snap.today > self.snap.best_day {
            self.snap.best_day = self.snap.today;
            self.snap.best_day_date = self.date.clone();
        }
    }

    /// keyup 유실 보정.
    ///
    /// Win+L 잠금·UAC 승격·데스크탑 전환 중에는 keyup을 놓칠 수 있다. 그대로
    /// 두면 그 키가 영구히 "눌림"으로 남아 이후 입력이 전부 auto-repeat으로
    /// 오판되고 카운트가 조용히 샌다 (§15).
    fn watchdog(&mut self) {
        let now = Instant::now();
        for i in 0..KEY_SLOTS {
            let Some(since) = self.last_down_at[i] else {
                continue;
            };
            if now.duration_since(since) < STUCK_THRESHOLD {
                continue;
            }
            let Some(vk) = scan_to_vk(i as u16) else {
                continue;
            };
            if unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 == 0 } {
                self.down[i] = false;
                self.last_down_at[i] = None;
                self.snap.watchdog_fixed += 1;
            }
        }
    }
}

/// 집계 스레드를 띄우고 스냅샷 채널을 돌려준다.
pub fn start(events: Receiver<RawEvent>, mut store: Store) -> Receiver<Snapshot> {
    let (snap_tx, snap_rx) = unbounded::<Snapshot>();

    std::thread::spawn(move || {
        let mut st = State::new(&store, local_date());
        let mut last_watchdog = Instant::now();
        let mut last_flush = Instant::now();
        let mut last_sent: Option<Snapshot> = None;

        loop {
            // 1) 채널을 먼저 비운다. 워치독보다 반드시 앞이어야 대기 중인
            //    keyup을 앞질러 보는 일이 없다 (STUCK_THRESHOLD와 함께 이중 방어).
            match events.recv_timeout(Duration::from_millis(100)) {
                Ok(ev) => st.apply(&ev),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            loop {
                match events.try_recv() {
                    Ok(ev) => st.apply(&ev),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }

            if last_watchdog.elapsed() >= WATCHDOG_INTERVAL {
                st.watchdog();
                last_watchdog = Instant::now();
            }

            // 2) 날짜가 넘어가면 **이전 날짜로** 먼저 기록하고 오늘을 0으로 연다.
            let today = local_date();
            if today != st.date {
                flush(&mut st, &mut store);
                st.date = today;
                st.snap.today = 0;
            }

            if last_flush.elapsed() >= FLUSH_INTERVAL {
                flush(&mut st, &mut store);
                last_flush = Instant::now();
            }

            let h = hook::health();
            st.snap.max_cb_us = h.max_cb_us;
            st.snap.cb_calls = h.cb_calls;
            st.snap.dropped = h.dropped;
            st.snap.reinstalls = h.reinstalls;
            st.snap.paused = hook::is_paused();

            if last_sent.as_ref() != Some(&st.snap) {
                last_sent = Some(st.snap.clone());
                if snap_tx.send(st.snap.clone()).is_err() {
                    break;
                }
            }
        }

        // 종료 경로에서도 마지막 델타는 반드시 남긴다.
        flush(&mut st, &mut store);
    });

    snap_rx
}

fn flush(st: &mut State, store: &mut Store) {
    if let Err(e) = store.flush(&st.date, st.pending_total, &st.pending_keys) {
        // 저장에 실패해도 카운팅은 계속한다. 델타를 유지하므로 다음 flush에서
        // 함께 반영된다 — 일시적 잠금·디스크 문제로 기록이 사라지지 않게.
        eprintln!("로컬 저장 실패(다음 flush에서 재시도): {e}");
        return;
    }
    st.pending_total = 0;
    st.pending_keys.clear();
}
