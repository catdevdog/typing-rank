import { listen } from "@tauri-apps/api/event";

/**
 * Rust 집계 스레드가 내보내는 단일 상태.
 *
 * 대시보드·오버레이·트레이가 **같은 스냅샷**을 본다. 표시 수단마다 카운터를
 * 따로 두면 셋이 서로 다른 숫자를 보여주는 순간이 반드시 온다.
 *
 * 필드 이름은 Rust의 `counter::Snapshot`과 1:1이다 — 한쪽만 고치면 조용히
 * undefined가 된다. 서버 계약(`packages/types`)이 생기면 Rust 구조체에서
 * 타입을 생성하는 쪽으로 옮긴다.
 */
export interface Snapshot {
  today: number;
  total: number;
  best_day: number;
  best_day_date: string;
  session: number;
  repeat_dropped: number;
  injected: number;
  watchdog_fixed: number;
  paused: boolean;
  // 건강 지표 (PLAN.md §14 — 8시간 무누수 이관분)
  max_cb_us: number;
  cb_calls: number;
  dropped: number;
  reinstalls: number;
}

export const EMPTY_SNAPSHOT: Snapshot = {
  today: 0,
  total: 0,
  best_day: 0,
  best_day_date: "",
  session: 0,
  repeat_dropped: 0,
  injected: 0,
  watchdog_fixed: 0,
  paused: false,
  max_cb_us: 0,
  cb_calls: 0,
  dropped: 0,
  reinstalls: 0,
};

export function onSnapshot(fn: (s: Snapshot) => void) {
  return listen<Snapshot>("snapshot", (e) => fn(e.payload));
}

/** 오버레이 표시 상태. 트레이 메뉴와 단축키가 같은 값을 바꾼다. */
export interface OverlayState {
  visible: boolean;
  variant: "minimal" | "normal" | "maximum";
}

export function onOverlayState(fn: (s: OverlayState) => void) {
  return listen<OverlayState>("overlay-state", (e) => fn(e.payload));
}
