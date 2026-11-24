# StackLab

블록 스태킹(테트로미노) 적응형 훈련 프로그램. Windows 10/11용 1인 오프라인 앱.

- 명세: [docs/SPEC_v5.5.md](docs/SPEC_v5.5.md)
- 진행 상황: [docs/PROGRESS.md](docs/PROGRESS.md) · 결정 기록: [docs/DECISIONS.md](docs/DECISIONS.md)
- 룰셋 수치와 출처: [rulesets/TL-S2.toml](rulesets/TL-S2.toml), [docs/RULESET_SOURCES.md](docs/RULESET_SOURCES.md)

## 빌드·테스트

```
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
```

## STEP 0 기술 검증 실행

```
cargo run --release -p stacklab-solver --example pc_spike -- 200 11
cargo run --release -p stacklab-opener --example opener_spike
cargo run --release -p stacklab-bot    --example bot_strength_spike -- 40
```
