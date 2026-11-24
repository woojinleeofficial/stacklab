# TL-S2 룰셋 수치 출처

조사일: 2026-11-24 · 대상 파일: `rulesets/TL-S2.toml`

## 사용한 공개 자료

| ID | 자료 | 접근 결과 |
|----|------|-----------|
| U1 | 사용자 확인 (2026-11-24, STEP 0 확인 지점) | PC 공격량 |
| S1 | tetris.wiki — "TETR.IO" 문서 (Mechanics 절: Multiplier 콤보, B2B Charging, All-Mini+, Clutch Clear, Opener Phase, Passthrough) | 읽음 |
| S2 | tetris.wiki — "SRS" 문서 (JLSTZ·I 표준 킥 표) | 읽음 |
| S3 | tetris.wiki 이미지 `TETR.IO_180kicks.png` (osk 게시 "TETR.IO 180 KICK TABLE") | 이미지 판독 |
| S4 | tetris.wiki 이미지 `TETR.IO_SRS+kicks.png` (osk 게시 "TETR.IO SRS+ I-kicks") | 확대 판독, 칸 좌표로 오프셋 계산 |
| S5 | tetris.wiki 이미지 `TETR.IO_Combo-Table.png` (콤보 0~20 공격량 표) | 이미지 판독 |
| S6 | TETR.IO FAQ "Mechanics" (tetrio.github.io/faq/mechanics.html) | 읽음 — Glicko/RD 설명만 있고 공격·가비지 수치는 없음 |
| S7 | tetrio.wiki.gg (TETRA LEAGUE, Garbage 등) | **접근 불가** (사이트의 자동 요청 차단, 403) |
| S8 | harddrop.com/wiki/TETR.IO | 읽음 — 수치 없음 |

공식 패치 노트(tetr.io)는 이 세션의 네트워크 허용 범위 밖이라 읽지 못했다.

## 값별 출처와 확인 상태

| 항목 | 값 | 상태 | 근거 |
|------|----|------|------|
| 프레임 | 60fps | verified | S1 "20 frames, or 0.333 seconds" |
| 서브프레임 | 10 / frame | unverified | 구현 선택 (DECISIONS D-004) |
| 필드 | 10×40, 표시 20 | verified | 명세 3.1 |
| 스폰 위치 | x=4, y=20 (3칸 블록 3~5열, 21행) | unverified | 가이드라인 관례 |
| Next / Hold / 7-Bag | 5 / 있음 / 7-Bag | verified | 명세 3.1 |
| Bag 기본 순서 | ZLOSIJT | unverified | 시드 호환용일 뿐 규칙과 무관 |
| JLSTZ 킥 | 표준 SRS | verified | S2 (+S4 그림에서 I 외 변경 없음) |
| I 킥 (SRS+) | 0→R (0,0)(+1,0)(−2,0)(−2,−1)(+1,+2) 외 7개 | verified | S4 판독. 표준 SRS 대비 0→R, R→0, 2→R, L→2, 0→L이 달라지며 전체가 좌우 대칭(테스트 `srs_plus_i_kicks_are_mirror_symmetric`) |
| 180° 킥 | N→S (0,0)(0,+1)(+1,+1)(−1,+1)(+1,0)(−1,0) 외 3개 | verified | S3 |
| I 블록의 180° 킥 | JLSTZ와 같은 표 적용 | unverified | S3은 T 그림만 있음 |
| 스핀 판정 | All-Mini+ (T 외 블록과 T 모두 고정 판정 시 Mini) | verified | S1 "allowing T-pieces to use immobile detection as well" |
| T 3코너 규칙·Full/Mini 구분·5번째 킥 승격 | 가이드라인 규칙 | unverified | 공개 문서에 세부 미기재 |
| 고정(immobile) 판정 방향 | 좌·우·위·아래 | unverified | |
| 공격표 | S0/D1/T2/Q4, TSMS0/TSS2/TSMD1/TSD4/TST6 | verified | S5 콤보 0 열 |
| 비T Mini 3·4줄 | 2 / 4 | unverified | |
| PC(All Clear) 공격 | 클리어 자체 공격 + 5줄 (예: Quad PC 4+5=9, TSD PC 4+5=9) | **user-confirmed** (2026-11-24) — B2B +2와 패킷 분리 여부는 unverified | S1은 표의 PC 값(10)이 "outdated"라고만 하고, QUICK PLAY는 "3 lines but add +2 B2B"라고 명시. TL 값은 미확인 |
| 콤보 | Multiplier: base×(1+0.25×combo), base 0이면 ln(1+1.25×combo) (2콤보 이상), 내림 | verified | S1 + S5로 0~20 콤보 전 구간 대조 (테스트 `combo_table_matches_public_chart`) |
| B2B | Charging: 연속 어려운 클리어마다 +1줄 | verified | S1 |
| Surge | B2B 4부터 충전, 시작 4줄(QP 제외), 레벨당 +1, 끊길 때 3분할(나머지는 앞쪽부터) | verified | S1 |
| B2B 카운터 의미 | 화면 표시 "B2B ×N" 기준 | verified(부분) | S1의 이미지에서 ×4부터 별 표시 |
| 오프너 페이즈 | 첫 14블록, 보낸 줄 < 대기 줄이면 상쇄 2배 | verified | S1 |
| 가비지 이동 시간 | 20프레임 | verified | S1 Passthrough 절 |
| 패스스루 | 없음 | verified | S1 (TL에서 제거) |
| 가비지 구멍 | change-on-attack | verified | S1 |
| 메시니스(공격 내/공격 간) | 0‰ / 1000‰, 이전 구멍과 다른 열 | unverified | |
| 가비지 캡 | 블록당 8줄 | unverified | |
| 가비지 진입 시점 | 줄을 지우지 않은 고정 시 | unverified | |
| 중력 / 락 딜레이 / 리셋 한도 | 0.02G / 30프레임 / 15회 | unverified | |
| 마진 타임 공격 배율 | 사용, 180초 후 초당 +0.008 | **unverified (존재 여부 포함)** | 켜고 끌 수 있게 구현 |
| Top Out | Block Out 사용, Lock Out 미사용 | verified | S1 "lock out was disabled by default" |
| Clutch Clear | Block Out 순간 줄을 지우면 다음 블록을 스택 위로 올려 생존 | verified | S1 |
| RD 범위·증가 | 하한 약 58~60, 상한 350, 7일 미사용 후 하루 1씩 | verified | S1, S6 |
| BO 형식 | TL은 랭크에 따라 FT3/FT5/FT7 | verified | S1 (앱은 명세대로 BO3로 축약) |

## 미확인 항목 처리

1. 모든 미확인 값은 `rulesets/TL-S2.toml`에서 바로 바꿀 수 있고, 엔진 코드에는 수치가 없다.
2. STEP 1의 14.1 "공격량 대조" 항목은 확인된 값(공격표·콤보·B2B·Surge·오프너 페이즈)으로 먼저 픽스처를 만든다.
3. 미확인 값은 사용자가 TETRA LEAGUE 리플레이(.ttrm)나 확인된 수치를 주면 대조해서 `verified`로 바꾼다. 우선순위: 가비지 캡·메시니스 → 마진 타임 → 중력·락 딜레이.
