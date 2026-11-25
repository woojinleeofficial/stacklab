# 진행 기록

중단 후 이어서 작업할 때 이 파일부터 읽는다.

## 현재 상태

- **현재 단계:** STEP 7 클라우드 범위 완료 → Windows PC 성능 확인·사용자 인수(12.4 확인 지점 2) 대기. STEP 0은 2026-11-24 사용자 확인 완료.
- **작업 기기:** 클라우드 리눅스(Windows PC 미연결). Windows 빌드·테스트는 GitHub Actions `windows-latest`.
- **마지막 CI 결과:** run #24 (eaf5e16, STEP 7) ubuntu ✓ windows ✓ — 설치 프로그램 빌드·무인 설치·자가 점검·제거 ✓. 아티팩트 `stacklab-windows-installer`, `stacklab-windows-x64`.
- **Windows PC에서 확인이 필요한 항목:** `docs/ACCEPTANCE_WINDOWS.md` (설치, 60FPS·고주사율, 입력 지연 16ms 이하, 조작감, 1세트 완주).

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

## STEP 4 — 오프너 훈련

### 완료
- 오프너 라이브러리 `data/openers/`: TKI 3(분기 4 + 대칭 4), PCO(I 포함/미포함 + 대칭), `pc-first`(PC 템플릿), `free-transition`(자유 스태킹 전환 템플릿). 출처·버전·요구 능력·특성값 포함. DT Cannon은 보류(D-040)
- 실행 가능성(`crates/opener/src/feasibility.rs`): fullSequenceFeasible, visibleInfoFeasible(AND-OR, Bag 카운팅), coverage(전수/표본 + 95% 구간 + 검증 범위 문자열), abandonRule(주력 → 예외 → 자유 스태킹)
- 커버리지 데이터 `data/openers/coverage.toml`: TKI 69.0%, PCO 85.2%(첫 Bag 5,040가지 전수), 템플릿은 표본
- 로드아웃(`loadout.rs`): 주력≠예외, 주력은 암기형만, 룰셋 유효성, 버전 충돌 감지. 역할 이력
- E 영역 문제(`training/src/opener_gen.rs`): 주력 / 전환 판단 / 예외, 다중 정답 분기, TSD 마무리 판정
- 오프너 숙련 MMR(오프너별, 역할 무관)·분기 통계, 사이클 종료 시 갱신
- 저장소: 로드아웃 트랜잭션(버전 확인 → 변경 이력 → 로드아웃), 역할 이력 추가 전용
- 앱 UI-07 오프너 관리: 주력·예외 선택, 적용 조건, 커버리지와 검증 범위('100% 가능'은 전수 검증 + 전부 가능일 때만), 숙련 MMR, 분기 도식(블록 색)과 분기별 성공 기록, 출처. 스크린샷 `docs/screenshots/step4_openers.png`
- 도구: `tools/fumen_decode.py`(fumen v115 디코더)

### 테스트 (14.3 오프너 항목)
- 주력·예외는 서로 다른 항목만(앱·도메인·DB 세 곳), 주력 불가 → 예외, 둘 다 불가 → 자유 스태킹
- '100% 가능'은 전수 범위에서만(표본은 불가), 커버리지 파일 버전 일치
- 로드아웃 변경 후 비선택 오프너의 MMR·분기 기록 그대로, 과거 역할 소급 변경 없음, 버전 충돌·DB 제약 위반 시 변경 없음
- 역할별 E 문제 생성·검증(3가지 조합 × 3역할), 전환 문제에서 주력 분기는 정답이 아님
- 워크스페이스 88개 테스트 통과, clippy·fmt 통과

### 남은 문제
- DT Cannon 도식 확보 필요(사용자 fumen 또는 다른 출처)
- 오프너 문제 난이도 범위가 좁음(약 1,350~1,700). 실전 관측과 함께 STEP 5에서 보정
- 최초 진단의 "선택 오프너 실행 과제"(6.4)는 로드아웃을 고른 뒤 첫 사이클에서 E로 대신함

## STEP 5 — AI·3사이클

### 완료
- 봇(`crates/bot`): 공개 정보만 담은 `BotView`로 결정, `BotPlayer`가 실제 키 입력으로 틱 엔진 조작, 반응 지연·PPS 페이싱. 프리셋 8단계 `data/bots/presets.toml`
- 봇 리그: 인접 프리셋 쌍당 200판(실제 엔진 대국, 진영 교대), Bradley–Terry 최대우도(Glicko 눈금, 기준 L4 = 1500) → `data/bots/calibration.toml`, 보고서 `docs/reports/step5_bot_league.txt`
- 세트 상태 머신(`training/src/set.rs`): 진단 → 12문제 ×3 사이 미니 2회 → BO3 → 결과, 미니 건너뛰기·나중에, 평가 연기·평가 없이 종료, 무효 라운드
- 실전 MMR(평가전 1세트 = 1 레이팅 기간), 상대 선택(예상 승률 40~60% 중 50%에 가장 가까운 프리셋, 범위 밖이면 표시)
- TL 앵커: 초기값·RD 하한 150·재입력 가중 합성·입력 이력 보존·봇 눈금 오프셋 학습, "TL Glicko 환산 추정치" 표시
- 사용 중 보정: 평가전마다 상대 봇 등급 Glicko-2 갱신(RD 하한 50), calibrationVersion 증가
- 엔진 관측 약점(공격 효율·가비지 Top Out·스핀 0·오프너 미완성) → 다음 사이클 영역 가중치
- 앱: 대결 화면 UI-05(양쪽 필드, 대기 가비지 계기, Hold/Next, B2B, 라운드 표시, 예상 승률), 대시보드 단계별 버튼, 평가전 결과 UI-06(BO3, 실전 MMR 변화, 확인된 약점), TL 기록 입력. 스크린샷 `docs/screenshots/step5_match.png`

### 봇 리그 결과 (쌍당 200판)
| 프리셋 | PPS | 등급 ± RD | 아래 단계 상대 승률 |
|---|---|---|---|
| L0 | 0.5 | 497 ± 32 | — |
| L1 | 0.8 | 770 ± 22 | 83% |
| L2 | 1.1 | 974 ± 24 | 76% |
| L3 | 1.25 | 1362 ± 22 | 90% |
| L4 (기준) | 1.4 | 1500 | 69% |
| L5 | 1.8 | 1704 ± 21 | 76% |
| L6 | 2.3 | 1954 ± 21 | 81% |
| L7 | 3.0 | 2135 ± 28 | 74% |
- 강도 순서와 단조 일치, 시간 상한 도달 0판

### 테스트 (14.2)
- BO3 2승 선착·최대 3판·무효 라운드 미집계, 스킵·연기·미평가 처리, 12문제 전 대결 불가
- 미니 대결로 실전 MMR 불변, 평가전 완료 시에만 갱신, Glicko-2 Glickman 예제(STEP 3)
- TL 앵커 초기값·재입력·이력·오프셋, 상대 선택 범위·범위 밖 표시, 사용 중 보정
- 엔진 관측 약점 → 다음 사이클 가중치, 패배만으로는 약점 아님
- 봇이 숨은 블록을 보지 않음, 리그 등급 강도 순서 일치(보정 파일 검사), 봇이 사용자 역할로 미니 2회 + BO3 완주
- 워크스페이스 103개 테스트 통과, clippy·fmt 통과

### 남은 문제
- L2→L3 간격이 큼(약 390점, 90%): 이 구간 사용자는 40~60% 상대가 없을 수 있음(가장 가까운 상대 + 표시로 대응). 실사용 데이터로 프리셋 재조정 가능
- 봇 결정은 UI 스레드에서 블록마다 수 ms 계산 → STEP 7에서 별도 스레드 + 지연 측정
- 대결 화면은 실제 키 입력 검증을 Windows PC에서 해야 함

## STEP 6 — 오프너 추천

### 완료
- 적합도(`training/src/recommend.rs`): 능력-요구 35 · 관측 성과 25 · 상호 보완 20 · 공격/후속 10 · 학습 부담 10(정책값), 주력·예외 조합 단위 평가, 항목별 불확실성 → 80% 구간, 8점 + 구간 비중첩 규칙, 모델 추정값 표시
- 상호 보완 계산용 Bag별 가능 여부 비트셋(`data/openers/coverage.toml`)
- 1회 자동 이벤트 트리거(최소 데이터 조건 + 정상 완료 평가전), 상태 머신(PENDING → ANALYZING → READY/NO_CHANGE/UNAVAILABLE → APPLIED/DECLINED/DEFERRED)
- 저장소 스키마 3: 추천 이벤트 저장, 자동 이벤트 유일 제약, 적용 트랜잭션(버전 확인·멱등·롤백)
- 앱 UI-08 추천 화면(2단계 승인), UI-06/UI-07 진입, 수동 재분석. 스크린샷 `docs/screenshots/step6_recommendation.png`

### 테스트 (14.3 추천 항목)
- 최소 데이터 조건을 처음 충족한 정상 평가전 후에만 1회(연기·미평가·미완료·데이터 부족 제외, 이후 자동 추천 없음)
- 적용은 READY/DEFERRED에서만, 재시작 후 상태 유지, 중복 클릭 1회만 변경, 수동 변경 후 버전 충돌, 트랜잭션 중간 실패 시 롤백, 허용되지 않은 상태 전이 거부
- 근거 부족 시 UNAVAILABLE, 데이터 없음 → 유지 권고(가짜 확신 없음), 명확한 근거가 있을 때만 변경 추천
- 워크스페이스 114개 테스트 통과, clippy·fmt 통과

### 남은 문제
- 경기 기록(Match/Round, 리플레이) DB 저장은 STEP 7에서(현재 세트 상태는 app_state JSON)
- 적합도 가중치·환산식은 초기값이라 실사용 데이터로 보정 필요

## STEP 7 — 제품화 (클라우드 범위 완료, Windows PC 확인 대기)

### 완료
- 학습 흐름 상태 머신 `training/src/flow.rs`(앱·테스트 공용), 진단 건너뛰기, 대시보드 최초 설정 안내(UI-02)
- 자동 플레이 스크립트 `crates/bot/tests/playthrough.rs`: 봇이 사용자 역할로 진단 + 36문제 + 미니 2회 + BO3 + 1회 추천(저장·2단계 승인·적용·멱등)까지 완주, 모든 라운드 리플레이 재검증
- 엔진 셀 색상 레이어(고정 블록 색 유지, 결정성 무관)
- 경기·라운드 DB 저장(리플레이·관측 통계), 문제 시도 저장, 원자적 체크포인트와 강제 종료 복구(무효 라운드 처리)
- UI-09: 설정(키 바인딩 중복 검사, DAS/ARR/DCD/SDF를 TETR.IO 단위·범위로), 통계 탭(기술별·경기·40라인·독립 측정·TL 기록·백업/복원)
- 40라인 모드(카운트다운, 기록·리플레이 저장), 독립 실력 측정(3세트마다 제안, 언제든 실행)
- 봇 탐색 작업 스레드 분리(결정성 유지), 일시정지, 성능 측정 오버레이 + CSV 로그
- 설치 프로그램 `installer/stacklab.iss`(Inno Setup), `stacklab.exe --self-check`, CI 무인 설치/자가 점검/제거 테스트
- 사용 설명서 `docs/USER_GUIDE.md`, 제3자 라이선스 자동 생성 `tools/licenses.py`(CI 검사), Windows 인수 체크리스트 `docs/ACCEPTANCE_WINDOWS.md`
- 스크린샷: `docs/screenshots/step7_settings.png`, `step7_stats.png`, `step7_match_perf.png`

### 테스트 (14.4)
- 자동 플레이 완주 ✓, 강제 종료(자식 프로세스 abort) 후 체크포인트 복구 ✓, JSON 백업/복원 ✓, 스키마 마이그레이션 ✓
- 비동기 봇 = 동기 봇(같은 최종 상태) ✓, 색 레이어 불변식(퍼즈) ✓, 설정 검증·독립 측정·기록 통계·40라인 정렬/리플레이 ✓
- Windows CI 빌드·테스트·설치/제거 ✓ (run #24)

### Windows PC에서 확인이 필요한 항목 (12.4 확인 지점 2)
- 60FPS 이상·고주사율 모니터, 입력 처리 추가 지연 16ms 이하(성능 오버레이·CSV로 측정), 실제 조작감, 사용자가 직접 1세트 완주
- 절차: `docs/ACCEPTANCE_WINDOWS.md`

## 다음 단계
1. Windows PC 연결 후 `docs/ACCEPTANCE_WINDOWS.md` 절차로 성능 확인(14.4 앞 두 항목)
2. 사용자 1세트 직접 완주와 인수(12.4 확인 지점 2) → 피드백 반영 → 1.0.0
