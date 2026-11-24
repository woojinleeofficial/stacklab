# 진행 기록

중단 후 이어서 작업할 때 이 파일부터 읽는다.

## 현재 상태

- **현재 단계:** STEP 3 완료 → 다음 STEP 4(오프너 훈련). STEP 0은 2026-11-24 사용자 확인 완료.
- **작업 기기:** 클라우드 리눅스(Windows PC 미연결). Windows 빌드·테스트는 GitHub Actions `windows-latest`.
- **마지막 CI 결과:** run #5 (d04bc46, STEP 2) ubuntu ✓ windows ✓ — fmt·clippy·test(release) 전부 통과.
- **Windows PC에서 확인이 필요한 항목:** 없음(STEP 0). STEP 7에서 14.4 성능 항목.

## STEP 0 — 설계 확정

### 완료
- 명세를 `docs/SPEC_v5.5.md`로 저장
- TL-S2 수치 수집·출처 기록: `rulesets/TL-S2.toml`, `docs/RULESET_SOURCES.md`
- 정책값: `config/policy.toml`
- 틱 레이트·DAS/ARR 변환 규칙: DECISIONS D-004
- DB 스키마: `crates/store/migrations/0001_init.sql` (SQLite에서 실행 확인)
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

## STEP 2 — 문제 프레임워크

### 완료
- `crates/training`: 문제 포맷(`problem.rs`: seed·rulesetVersion·generatorVersion·validationVersion·goal·witness·difficulty·fingerprint·subtypeTags·paramBucket·제한시간), 정책 로더(`policy.rs`)
- 판정 세션 `session.rs`: 실제 엔진으로 플레이, 목표 판정(피네스: 배치 + 최소 입력 별도 / 스태킹: 구멍·높이·Well), 제한시간, 1블록 되감기, 처음부터 재시작, 힌트, 정답 보기, MMR 유효 여부
- 생성기: 피네스 `finesse.rs`(입력 그래프 최단 비용, 동일 비용 복수 정답), 스태킹 `stacking.rs`(9-0/6-3/7-2/자유형, Hold)
- 검증기 `validator.rs`(정답 입력 재실행), 캐시 `cache.rs`, 출제 서비스 `service.rs`(생성 → 검증 → 최근 5문제 중복 제외 → 캐시 대체 → 다른 영역 + 진단 로그)
- `crates/store`: SQLite 마이그레이션(트랜잭션, 실패 시 롤백), 문제·시도 저장, 슬롯 1회 집계 제약, JSON 백업/복원

### 테스트
- `training`: 출제 240문제 전부 검증 통과(5개 난이도 × A·B), 난이도 요청 반영, JSON 왕복 후 재검증, 최근 중복 제외, 캐시·다른 영역 대체, 변조 문제 거부, 피네스 동일 비용 경로 모두 인정·비최적 구분·오배치 실패, 되감기 후 보드 정확 복원·MMR 제외, 제한시간 실패
- `store`: 스키마·룰셋 등록, 문제 왕복, 슬롯 1회 집계, 디스크 백업/복원(깨진 백업은 변경 없음), 미래 마이그레이션 데이터 보존·실패 롤백
- 생성 보고서 `docs/reports/step2_generation.txt`: A·B 각 500개 후보 생성률 100%, 검증률 100%, 생성 시간 A < 1ms, B 평균 25~90ms
- 워크스페이스 전체 65개 테스트 통과(release), clippy·fmt 통과

### 남은 문제
- 피네스 난이도 하한은 약 1,200(하드 드롭만으로 끝나는 문제). 더 쉬운 문제가 필요하면 STEP 3 보정에서 판단.
- 스태킹 판정은 "보이는 Next 수"와 무관하게 전체 블록열로 해를 증명함(시야 제한은 난이도 요인으로만 사용).

## STEP 3 — 기술 영역 확대

### 완료
- 문제 영역 7개(오프너 E 제외): A 피네스, B 스태킹, C 스핀(TSS/TSD/TST/TSM/All-Mini+, 준비 블록 0~2, Hold 경로), D PC(2줄, 4줄 진행 중, n번째 PC 1~7), F 공격(B2B·Surge·스핀), G 방어(정렬/지저분한 가비지, 대기 가비지), H 판단(다중 정답, 비평가용 표시)
- 목표 판정 5종 추가: 스핀 / PC / 공격량 / 다운스택 / 배치 선택
- Glicko-2 (`glicko.rs`): Glickman 예제 일치, 75% 역산, TETR.IO식 미사용 RD 증가
- 난이도 보정 (`adaptive.rs`): 생성 매개변수 구간별 레이팅, RD 하한, 앵커(D=1500 고정)와 앵커 진단, 영역별 추정 오차 학습
- 스케줄러: 실패·지연 기반 가중치(0.8~1.8), 영역 최대 5 / 최소 3, PC 최소 2(D-019), 약점 하위 태그 집계
- 사이클 = 레이팅 기간: 12슬롯, 재도전 미집계, 보조 사용 시 MMR 제외, 사이클 종료 시 일괄 갱신
- 앱 골격 `app/` (egui): 대시보드, 진단·사이클 진행, 문제 화면(UI-03: 필드·고스트·Hold·Next·목표·남은 시간·힌트·재시작), 결과 화면(UI-04: 성공/실패 사유, 피네스 결과, 1블록 되감기, 재도전, 정답 보기, 상세), 백그라운드 문제 생성, 한국어 문자열 리소스, 프로필 저장. 스크린샷 `docs/screenshots/step3_*.png`
- DB 스키마 2(`app_state`), CI에 Windows 실행 파일 아티팩트 업로드 추가

### 테스트 (14.2 일부)
- 7개 영역 출제 문제 전부 검증 통과(난이도 3단계 × 시드), JSON 왕복 후 재검증
- 실제 생성 문제로 12문제 사이클 끝까지 진행 → 레이팅 갱신
- 스케줄러 제약 2,000회, 실패 시 가중치 상승, 보조 사용 시 MMR 불변, 13번째 문제 거부
- 시뮬레이션 사용자 10명 × 60사이클: 난이도 맞춘 문제 정답률이 사이클 11 이후 약 72~76%(목표 75% ± 10%p), 레이팅 평균 오차 66점 (`docs/reports/step3_convergence.txt`)
- 생성 보고서 `docs/reports/step3_generation.txt`
- 워크스페이스 77개 테스트 통과, clippy·fmt 통과

### 남은 문제
- 난이도 범위 한계: C 하한 약 1,410, H는 대부분 1,650~1,750(보드 평가 차이가 좁음), D 4줄 PC 생성은 평균 0.4~0.6초·최대 2.4초(백그라운드 생성 + 캐시 전제)
- 하위 태그 약점은 집계만 하고 아직 생성기에 "이 태그로 내 달라"고 전달하지 않음 → STEP 5에서 실전 관측과 함께 연결
- 앱은 골격 수준: 키 설정·DAS/ARR 설정 화면, 블록 색 유지(고정 블록은 회색), 입력 전용 스레드는 STEP 7
- 사용 기기: 이번 화면 확인은 클라우드 가상 디스플레이(OpenGL). Windows 실제 실행·성능은 STEP 7에서 PC로 확인

## 다음 단계 (STEP 4 오프너 훈련)
1. 오프너 분기 라이브러리(TKI, DT Cannon, PCO, pc-first, 자유 스태킹 전환) — 공개 도식과 대조, 모든 분기 엔진 재실행 증명
2. 주력·예외 선택, 실행 가능성 판정(fullSequence / visibleInfo / coverage / abandonRule), '100% 가능'은 검증 범위와 함께
3. E 영역 문제(주력 70 / 전환 판단 20 / 예외 10), 오프너 숙련 MMR·분기 통계, OpenerProfile/Loadout 분리 저장
