import { useEffect, useState } from "react";
import { EMPTY_SNAPSHOT, onSnapshot, type Snapshot } from "./snapshot";

const nf = new Intl.NumberFormat("ko-KR");

/** 지표 하나. **박스가 아니다** — 구분선과 타이포만으로 나눈다 (anti-card). */
function Stat({ label, value, note }: { label: string; value: string; note?: string }) {
  return (
    <div className="flex-1 pr-5 pt-[18px] [&+&]:border-l [&+&]:border-border [&+&]:pl-5">
      <p className="t-eyebrow text-text-subtle">{label}</p>
      <p className="t-h3 mt-2 tabular-nums text-text">{value}</p>
      {note && <p className="t-small mt-[3px] text-text-subtle">{note}</p>}
    </div>
  );
}

function DiagRow({ label, value, alert }: { label: string; value: string; alert?: boolean }) {
  return (
    <>
      <dt>{label}</dt>
      <dd className={`text-right ${alert ? "text-danger" : "text-text"}`}>{value}</dd>
    </>
  );
}

export default function App() {
  const [s, setS] = useState<Snapshot>(EMPTY_SNAPSHOT);

  useEffect(() => {
    const un = onSnapshot(setS);
    return () => {
      un.then((f) => f());
    };
  }, []);

  const today = new Date().toLocaleDateString("ko-KR", {
    year: "numeric",
    month: "long",
    day: "numeric",
    weekday: "long",
  });

  return (
    <main className="max-w-[640px] px-10 pb-8 pt-11 font-sans text-text">
      <p className="t-eyebrow text-text-subtle">오늘</p>
      <p className="t-display-lg mt-[10px] tabular-nums">
        {s.paused ? "일시정지" : nf.format(s.today)}
      </p>
      <p className="t-small mt-1.5 text-text-muted">{today}</p>

      <section className="mt-10 flex border-t border-border">
        <Stat label="전체 누적" value={nf.format(s.total)} />
        <Stat
          label="개인 기록"
          value={nf.format(s.best_day)}
          note={s.best_day_date || "아직 없음"}
        />
        <Stat label="이번 실행" value={nf.format(s.session)} />
      </section>

      {/* 카드가 아니라 accent 규칙선. 고지문이 박스가 되면 광고처럼 읽힌다. */}
      <p className="t-small mt-10 border-l-2 border-accent-border pl-3.5 text-text-muted">
        랭킹에 참여하기 전에는{" "}
        <strong className="font-medium text-text">타이핑 데이터가 서버로 나가지 않습니다.</strong>{" "}
        이 화면의 모든 수치는 이 PC에만 저장되며, 키가 눌린 순서와 내용은 기록되지 않습니다.
      </p>

      <details className="mt-9 border-t border-border pt-4">
        <summary className="t-small cursor-pointer list-none text-text-subtle transition-colors duration-fast ease-enter hover:text-text-muted">
          진단
        </summary>
        <dl className="t-small mt-4 grid grid-cols-[1fr_auto] gap-x-6 gap-y-1.5 tabular-nums text-text-muted">
          <DiagRow label="auto-repeat 드롭" value={nf.format(s.repeat_dropped)} />
          <DiagRow label="injected 제외" value={nf.format(s.injected)} />
          <DiagRow label="워치독 보정" value={nf.format(s.watchdog_fixed)} />
          <DiagRow label="콜백 최대" value={`${nf.format(s.max_cb_us)} µs`} />
          <DiagRow label="콜백 호출" value={nf.format(s.cb_calls)} />
          <DiagRow label="훅 재설치" value={nf.format(s.reinstalls)} />
          <DiagRow label="이벤트 유실" value={nf.format(s.dropped)} alert={s.dropped > 0} />
          <DiagRow label="일시정지" value={s.paused ? "예 — 후크 해제됨" : "아니오"} />
        </dl>
      </details>
    </main>
  );
}
