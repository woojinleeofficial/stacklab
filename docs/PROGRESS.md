# 진행 기록

중단 후 이어서 작업할 때 이 파일부터 읽는다.

## 현재 상태

- **현재 단계:** STEP 1 완료 → 다음 STEP 2(문제 프레임워크). STEP 0은 2026-11-24 사용자 확인 완료.
- **작업 기기:** 클라우드 리눅스(Windows PC 미연결). Windows 빌드·테스트는 GitHub Actions `windows-latest`.
- **마지막 CI 결과:** 1차(STEP 0 커밋) 실패 — CI의 새 clippy 린트. 툴체인 1.100.0 고정 후 재실행(D-027)...
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

### STEP 0 사용자 확인 결과 (2026-11-24)
- PC 공격 = 클리어 공격 + 5줄 (D-018) · PC 범위 확대 (D-019) · 와이어프레임 위임 (D-020) · STEP 1 진행 승인

## STEP 1 — 공통 엔진

### 완료
- 고정 틱 게임 루프 `crates/engine/src/game.rs`: 60Hz × 서브프레임 10, DAS/ARR/SDF/DCD, 중력·소프트 드롭, 락 딜레이·리셋, Hold, 스폰, Block Out, Clutch Clear, ARE 설정
- 공격·가비지: 마진 배율, 상쇄(오프너 페이즈 ×2), 이동 20프레임, 블록당 캡, change-on-attack 구멍, Surge 3분할 패킷, PC +5
- 엔진 이벤트 17종 `events.rs` (틱·서브프레임·블록 번호·상태 해시·룰셋 버전 포함)
- API: `Game::new`(reset) / `step` / `snapshot` / `restore` / `events` / `ReplayLog::run`(replay), `Recorder`, JSON 직렬화, 룰셋 버전 레지스트리
- 2인 대전 `versus.rs` + `VersusReplay`
- 40라인 모드 `Mode::Sprint`
- 정답 경로: `movegen::find_path` + `game::path_inputs`

### 테스트 (14.1)
- 픽스처 21개 `crates/engine/tests/fixtures.rs`: 스폰, 충돌·벽, DAS/ARR/DCD, 락 딜레이, 180° 킥, TSD, All-Mini+ S스핀, Quad PC = 9, TSD/Double PC, B2B 차징·Surge 3/3/2, 콤보 배율, 오프너 페이즈 상쇄·종료, 가비지 이동·캡·구멍, Block Out, Clutch Clear, Hold 1회, 40라인 종료, 이벤트 룰셋 버전
- 결정성 `determinism.rs`: 1,000 시드 무작위 입력 리플레이 일치(+불변식 퍼즈), JSON 왕복 50개, 스냅샷/복원, 대전 100판 리플레이 일치(가비지 발생 확인), 이전 빌드 고정 리플레이 4개
- 공개 자료 대조: 콤보 표 0~20 전 구간(단위 테스트), 사용자 확인 PC 예시(Quad PC 9, TSD PC 9)
- `cargo fmt --check` ✓ `cargo clippy -D warnings` ✓ `cargo test --workspace --release` ✓

### 남은 문제
- 공개 자료로 확인 못 한 룰 수치(가비지 캡·메시니스·마진·중력·락) — 사용자 리플레이로 대조 대기
- 사용자 TETRA LEAGUE 리플레이 대조(14.1 2번째 항목의 "또는")는 리플레이 파일을 받으면 추가

## 다음 단계 (STEP 2 문제 프레임워크)
1. 문제 포맷(seed·rulesetVersion·generatorVersion·goal·witness·difficulty·fingerprint·subtypeTags), `training` 크레이트
2. ProblemGenerator / ProblemValidator(정답 경로를 틱 엔진에서 재실행) / 캐시 / 중복 검사
3. 피네스(입력 그래프 최단 비용, 동일 비용 복수 정답)·스태킹 생성기, 1블록 되감기
4. `store` 크레이트(SQLite 스키마 0001)
