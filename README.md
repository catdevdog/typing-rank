# TypingRank

**얼마나 쳤나**를 겨루는 Windows 데스크탑 앱 + 웹 리더보드.

타자 *속도*가 아니라 **누적 입력량**이 지표입니다. `asdf` = 4.
개발자와 게이머를 위한, 모던하고 미니멀한 오픈소스 타이핑 카운터.

> 🚧 개발 초기 단계입니다. 아직 배포된 빌드가 없습니다.

---

## 무엇을 하는 앱인가

- 트레이에 상주하며 키 입력 **횟수**를 셉니다.
- 키보드 히트맵으로 "내가 가장 많이 누르는 키"를 보여줍니다.
- 온라인 리더보드에서 다른 사람들과 누적 입력량을 겨룹니다.

마우스·대역폭·업타임은 추적하지 않습니다. **타이핑만** 봅니다.

## 프라이버시

이 앱은 구조적으로 키로거와 인접한 카테고리에 있습니다. 그래서 설계로 증명합니다.

- 키 이벤트를 받으면 **카운터를 1 올릴 뿐**입니다. 입력 순서·단어·문장은 **저장하지 않으며, 재구성 자체가 불가능한 구조**입니다.
- 로컬 DB에는 키별 누적 횟수만 남습니다. 시퀀스 정보가 없습니다.
- 서버로는 **집계된 숫자만** 전송합니다. 원문과 순서는 전송되지 않습니다.
- **클라이언트 소스는 전부 공개되어 있습니다.** 직접 확인하세요 — 그게 이 앱이 신뢰를 요구하는 유일한 방식입니다.

## 기술 스택

| 영역 | 스택 |
|---|---|
| 데스크탑 | Tauri 2 + Rust + React + TypeScript |
| 키 후킹 | Windows `SetWindowsHookEx(WH_KEYBOARD_LL)` |
| 로컬 저장 | SQLite (rusqlite) |
| 웹 + API | Next.js (App Router) |
| DB | PostgreSQL + Drizzle ORM |

## 저장소 구조

```
typing-rank/
├─ apps/
│  ├─ desktop/     # Tauri 2 + React (Rust 코어: 키 후크 · SQLite · 배치 업로더)
│  └─ web/         # Next.js — 웹 리더보드 + /api
├─ packages/
│  ├─ ui/          # 공유 컴포넌트 (키보드 히트맵 SVG, 차트)
│  ├─ types/       # 공유 타입 (Pulse, KeyCounts, LeaderboardEntry)
│  └─ db/          # Drizzle 스키마 + 클라이언트
└─ docs/
   └─ PLAN.md      # 전체 기획서
```

## 개발 환경

- Node.js 22 LTS + pnpm
- Rust (stable-x86_64-pc-windows-msvc)
- Visual Studio Build Tools 2022 (C++ 데스크톱 워크로드)
- Docker (로컬 PostgreSQL)

자세한 설계와 로드맵은 [docs/PLAN.md](docs/PLAN.md)를 참고하세요.

## 라이선스

미정 (결정 예정)
