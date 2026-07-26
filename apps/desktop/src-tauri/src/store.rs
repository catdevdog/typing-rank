//! 로컬 SQLite 저장소.
//!
//! **집계만 기록한다.** 키가 눌린 순서·시각을 남기지 않으며, 저장된 것으로
//! 입력 내용을 재구성하는 것이 구조적으로 불가능하다 (PLAN.md §10 프라이버시
//! 철칙). 스키마에 시퀀스를 담는 컬럼을 추가하려 한다면 그 시점에 이 철칙이
//! 깨지는 것이므로 다시 생각할 것.
//!
//! 서버 스키마(§5)와는 별개다. 여기는 "내 PC의 내 기록"이고, 서버로 무엇을
//! 보낼지는 [`crate::counter`]가 별도로 결정한다. 랭킹에 참여하지 않으면
//! 이 파일 밖으로 나가는 것은 아무것도 없다.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

const SCHEMA_VERSION: i64 = 1;

pub struct Totals {
    /// 전체 누적.
    pub total: u64,
    /// 오늘(로컬 날짜 기준) 누적.
    pub today: u64,
    /// 개인 기록 — 하루 최다 타수. 리더보드와 무관하게 매일 갱신될 수 있는
    /// 성취라 MVP에 넣었다 (PLAN.md §7).
    pub best_day: u64,
    pub best_day_date: String,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;

        // WAL — 상주 앱이라 쓰기가 잦고, 읽기(대시보드)와 겹친다.
        // synchronous=NORMAL은 WAL에서 안전하며 SSD 쓰기를 크게 줄인다.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS daily_counts (
                 date  TEXT PRIMARY KEY,   -- 로컬 날짜 YYYY-MM-DD
                 total INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS key_counts (
                 scan_code INTEGER PRIMARY KEY,  -- 확장 키는 0x100 비트로 구분
                 count     INTEGER NOT NULL DEFAULT 0
             );",
        )?;
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)
             ON CONFLICT(key) DO NOTHING",
            params![SCHEMA_VERSION.to_string()],
        )?;

        Ok(Self { conn })
    }

    pub fn totals(&self, today: &str) -> rusqlite::Result<Totals> {
        // SQLite의 INTEGER는 i64다. 카운트는 음수가 될 수 없으므로 경계에서만
        // u64로 바꾼다 — 앱 안쪽까지 i64를 끌고 들어가면 뺄셈 실수가 조용히 통과한다.
        let total: i64 = self
            .conn
            .query_row("SELECT COALESCE(SUM(total), 0) FROM daily_counts", [], |r| {
                r.get(0)
            })?;

        let today_count: i64 = self
            .conn
            .query_row(
                "SELECT total FROM daily_counts WHERE date = ?1",
                params![today],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);

        let best: Option<(String, i64)> = self
            .conn
            .query_row(
                "SELECT date, total FROM daily_counts ORDER BY total DESC, date DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (best_day_date, best_day) = best.unwrap_or_else(|| (String::new(), 0));

        Ok(Totals {
            total: total.max(0) as u64,
            today: today_count.max(0) as u64,
            best_day: best_day.max(0) as u64,
            best_day_date,
        })
    }

    /// 누적 델타를 한 트랜잭션으로 반영한다.
    ///
    /// 키마다 쓰지 않고 [`crate::counter`]가 메모리에 모아 주기적으로 넘긴다 —
    /// 24시간 상주 앱이 키 하나에 디스크를 때리면 SSD 수명과 배터리에 그대로
    /// 청구된다.
    pub fn flush(
        &mut self,
        date: &str,
        total_delta: u64,
        key_deltas: &HashMap<u16, u64>,
    ) -> rusqlite::Result<()> {
        if total_delta == 0 && key_deltas.is_empty() {
            return Ok(());
        }

        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO daily_counts (date, total) VALUES (?1, ?2)
             ON CONFLICT(date) DO UPDATE SET total = total + excluded.total",
            params![date, total_delta as i64],
        )?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO key_counts (scan_code, count) VALUES (?1, ?2)
                 ON CONFLICT(scan_code) DO UPDATE SET count = count + excluded.count",
            )?;
            for (&scan_code, &count) in key_deltas {
                stmt.execute(params![scan_code as i64, count as i64])?;
            }
        }
        tx.commit()
    }
}
