# 진행 기록

중단 후 이어서 작업할 때 이 파일부터 읽는다.

## 현재 상태

- **현재 단계:** STEP 0 완료, **사용자 확인 대기**(12.4 확인 지점 1) — `docs/STEP0_SUMMARY.md`
- **작업 기기:** 클라우드 리눅스(Windows PC 미연결). Windows 빌드·테스트는 GitHub Actions `windows-latest`.
- **마지막 CI 결과:** 첫 푸시 후 확인 필요(워크플로 `.github/workflows/ci.yml` 추가됨).
- **Windows PC에서 확인이 필요한 항목:** 없음(STEP 0). STEP 7에서 14.4 성능 항목.

## STEP 0 — 설계 확정

### 완료
- 명세를 `docs/SPEC_v5.5.md`로 저장
- TL-S2 수치 수집·출처 기록: `rulesets/TL-S2.toml`, `docs/RULESET_SOURCES.md`
- 정책값: `config/policy.toml`
- 틱 레이트·DAS/ARR 변환 규칙: DECISIONS D-004
- DB 스키마: `docs/db/0001_init.sql` (SQLite에서 실행 확인)
- 와이어프레임: `docs/WIREFRAMES.md`
- 난이도 앵커 정의: `docs/DIFFICULTY_ANCHORS.md`
- 오프너 범위·메타 점검: `docs/OPENER_SCOPE.md`
- 기술 검증 3종(코드 + 측정 보고서 `docs/reports/`)
  - PC 탐색: `crates/solver` + `examples/pc_spike.rs`
  - 오프너 가시정보 판정: `crates/opener` + `examples/opener_spike.rs`
  - 봇 강도 조절: `crates/bot` + `examples/bot_strength_spike.rs`
- 엔진 코어(STEP 1의 기반): 룰셋 로딩, 보드, SRS+ 회전·180° 킥, All-Mini+ 스핀 판정, 공격(콤보·B2B 차징·Surge·PC), 7-Bag, 배치 탐색
- CI 워크플로(ubuntu + windows, fmt/clippy/test)

### 테스트
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace --release` 통과 (결과는 STEP0_SUMMARY 참고)

### 남은 문제
- 미확인 룰 수치(PC 공격량, 가비지 캡·메시니스, 마진 타임, 중력·락 딜레이) — 사용자 확인 또는 리플레이 대조 필요
- PC 탐색 최악 사례가 수 초 → STEP 2에서 비트보드 이동 생성으로 최적화, 문제 생성은 백그라운드·캐시 전제

## 다음 단계 (STEP 1 공통 엔진)
1. 고정 틱 게임 루프(입력 이벤트·DAS/ARR/SDF/DCD·중력·락 딜레이·리셋), Hold, Clutch Clear, Top Out
2. 가비지 큐(이동·상쇄·오프너 페이즈·캡·구멍), 마진 배율, Surge 분할 전송, 엔진 이벤트 17종
3. `reset/step/snapshot/restore/events/replay`, 리플레이 직렬화, 40라인 모드
4. 14.1 픽스처·1,000시드 리플레이·퍼즈 테스트
