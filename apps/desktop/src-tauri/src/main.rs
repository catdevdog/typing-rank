// 상주 앱이라 콘솔 창을 띄우지 않는다.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! TypingRank 데스크탑 클라이언트 (Windows).
//!
//! **계정 없이 완결된다.** 설치하면 바로 카운트가 시작되고, 대시보드·트레이·
//! 오버레이가 전부 로컬로 동작한다. 랭킹 참여는 선택이며, 참여하기 전에는
//! 타이핑 데이터가 서버로 나가지 않는다 (PLAN.md §7 진입 흐름, §10).
//!
//! 스레드 구성 — 셋 다 서로를 기다리지 않는다.
//!   1. 훅 스레드   ([`hook`])    : 콜백은 채널 push만. 판정 없음
//!   2. 집계 스레드 ([`counter`]) : 카운팅 규칙 + 워치독 + 주기적 flush
//!   3. 메인 스레드                : Tauri/WebView. 스냅샷을 받아 그리기만

mod counter;
mod hook;
mod store;
mod tray;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager};

/// 트레이 아이콘을 다시 그리는 최소 간격.
///
/// 스냅샷은 키를 누를 때마다 갱신되지만 트레이는 흘끗 보는 지표다. 키마다
/// 32×32 비트맵을 다시 그리면 "풋프린트가 강점"이라는 말이 무색해진다.
/// 실시간 표시는 오버레이가 맡는다 (PLAN.md §8).
const TRAY_UPDATE_INTERVAL: Duration = Duration::from_secs(1);

/// `%APPDATA%\TypingRank`
///
/// Tauri의 `app_data_dir()`은 번들 식별자를 그대로 폴더명으로 쓴다
/// (`%APPDATA%\dev.typingrank.app`). PLAN.md §12가 런타임 설정 파일 위치를
/// `%APPDATA%\TypingRank\config.json`으로 지정했으므로 같은 디렉터리를 쓴다.
fn data_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("TypingRank")
}

fn main() {
    let store = match store::Store::open(&data_dir().join("typingrank.db")) {
        Ok(s) => s,
        Err(e) => {
            // 저장을 못 하면 카운트가 의미를 잃는다. 조용히 도는 것보다
            // 즉시 멈추는 편이 낫다.
            eprintln!("로컬 DB를 열지 못했다: {e}");
            std::process::exit(1);
        }
    };

    let (hook_handle, events) = hook::start();
    let snapshots = counter::start(events, store);

    // 창이 뜬 직후 한 번 그려 주기 위한 최신값 보관소.
    let latest = Arc::new(Mutex::new(counter::Snapshot::default()));
    let load_latest = Arc::clone(&latest);

    tauri::Builder::default()
        // 중복 실행 방지는 **가장 먼저** 등록해야 한다.
        //
        // 두 인스턴스가 각자 훅을 걸고 같은 SQLite에 델타를 밀어 넣으면
        // 카운트가 조용히 두 배가 된다 — 개발 중 실제로 재현됐다. 두 번째
        // 실행은 기존 창을 띄우고 즉시 종료된다.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("dashboard") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        // 첫 키를 누르기 전에도 누적 수치가 보여야 한다. 이게 없으면
        // 재시작 직후 대시보드가 0으로 보인다.
        .on_page_load(move |webview, _| {
            let snap = load_latest.lock().unwrap().clone();
            let _ = webview.emit("snapshot", snap);
        })
        .setup(move |app| {
            // 일시정지(트레이)에서 쓴다 — PLAN.md §10.
            app.manage(hook_handle);

            let window = app
                .get_webview_window("dashboard")
                .expect("tauri.conf.json의 dashboard 창을 찾지 못했다");

            // 창을 닫아도 앱은 죽지 않는다. 24시간 상주가 목적이고,
            // 종료 수단은 트레이 메뉴다.
            let hide_target = window.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = hide_target.hide();
                }
            });

            let tray = tray::build(app)?;
            let handle = app.handle().clone();

            std::thread::spawn(move || {
                let mut last_tray = Instant::now() - TRAY_UPDATE_INTERVAL;
                let mut last_paused = None;

                for snap in snapshots {
                    *latest.lock().unwrap() = snap.clone();

                    // 일시정지는 유저가 방금 누른 결과라 즉시 보여야 한다.
                    // 나머지는 1초에 한 번으로 줄인다.
                    let paused_changed = last_paused != Some(snap.paused);
                    if paused_changed || last_tray.elapsed() >= TRAY_UPDATE_INTERVAL {
                        tray::update(&handle, &tray, &snap);
                        last_tray = Instant::now();
                        last_paused = Some(snap.paused);
                    }

                    if window.emit("snapshot", snap).is_err() {
                        return;
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tauri 앱 실행 실패");
}
