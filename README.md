# StackLab

블록 스태킹(테트로미노) 적응형 훈련 프로그램. Windows 10/11용 1인 오프라인 앱 (Rust · egui · SQLite).

- 사용 설명서: [docs/USER_GUIDE.md](docs/USER_GUIDE.md)
- 명세: [docs/SPEC_v5.5.md](docs/SPEC_v5.5.md)
- 진행 상황: [docs/PROGRESS.md](docs/PROGRESS.md) · 결정 기록: [docs/DECISIONS.md](docs/DECISIONS.md)
- 룰셋 수치와 출처: [rulesets/TL-S2.toml](rulesets/TL-S2.toml), [docs/RULESET_SOURCES.md](docs/RULESET_SOURCES.md)
- Windows 인수 확인 절차: [docs/ACCEPTANCE_WINDOWS.md](docs/ACCEPTANCE_WINDOWS.md)
- 제3자 라이선스: [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md)

## 구조

| 경로 | 내용 |
|---|---|
| `crates/engine` | 결정적 게임 엔진(TL-S2 룰, 리플레이, 2인 대전) |
| `crates/solver` | PC·지형 탐색 |
| `crates/opener` | 오프너 라이브러리·실행 가능성·로드아웃 |
| `crates/training` | 문제 생성·검증, Glicko-2, 스케줄러, 세트 흐름, 추천, 설정, 독립 측정 |
| `crates/bot` | AI(실제 키 입력, 가시 정보), 봇 리그 보정 |
| `crates/store` | SQLite 스키마·마이그레이션·백업/복원·체크포인트 |
| `app` | egui 앱 (UI-01~09) |
| `installer` | Inno Setup 스크립트, CI 설치 테스트 |

## 빌드·테스트

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --release
cargo build --release -p stacklab-app        # target/release/stacklab(.exe)
```

설치 프로그램(Windows, Inno Setup 6):

```
ISCC.exe /DAppVersion=0.9.0 installer\stacklab.iss   # target\installer\StackLab-Setup-0.9.0.exe
```

CI(GitHub Actions, ubuntu + windows)가 위 검사 전부와 설치 프로그램 무인 설치·자가 점검·제거를 실행하고,
`stacklab-windows-x64`(실행 파일)·`stacklab-windows-installer`(설치 프로그램) 아티팩트를 올립니다.

## 개발용 환경 변수

| 변수 | 용도 |
|---|---|
| `STACKLAB_DATA_DIR` | 데이터 폴더 지정(기본 `%APPDATA%\StackLab`) |
| `STACKLAB_RENDERER=glow` | OpenGL 렌더러(가상 디스플레이·그래픽 문제 대응) |
| `STACKLAB_PERF=1` | 성능 오버레이·CSV 로그 강제 켜기 |
| `STACKLAB_AUTOSHOT=out.ppm` + `STACKLAB_AUTOSCREEN=openers\|match\|settings\|stats\|sprint` | 화면 확인용 스크린샷 후 종료 |

## 보고서·도구

```
cargo run --release -p stacklab-bot    --example league -- --write      # 봇 리그 보정
cargo run --release -p stacklab-opener --example library_report -- --write
python3 tools/licenses.py                                             # 제3자 라이선스 목록 갱신
```
