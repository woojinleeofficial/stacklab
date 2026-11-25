#!/usr/bin/env python3
"""Regenerates THIRD_PARTY_LICENSES.md from `cargo metadata` (spec 15 (11), 12.3 no GPL).

Only crates linked into the Windows x64 build of the shipped binary are listed (normal and
build dependencies reachable from the workspace members; dev-dependencies excluded).

  python3 tools/licenses.py          # rewrite the file
  python3 tools/licenses.py --check  # CI: fail on copyleft-only licences or a stale file
"""
import json, re, subprocess, sys

TARGET = "x86_64-pc-windows-msvc"
OUT = "THIRD_PARTY_LICENSES.md"
COPYLEFT = re.compile(r"\b(A?GPL|LGPL)", re.I)

meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", TARGET]))
pk = {p["id"]: p for p in meta["packages"]}
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
members = set(meta["workspace_members"])
seen, stack = set(), list(members)
while stack:
    i = stack.pop()
    if i in seen:
        continue
    seen.add(i)
    for d in nodes[i]["deps"]:
        if any(k["kind"] in (None, "build") for k in d["dep_kinds"]):
            stack.append(d["pkg"])

rows, bad = [], []
for i in sorted(seen - members, key=lambda i: (pk[i]["name"], pk[i]["version"])):
    p = pk[i]
    lic = p.get("license") or ("see " + p["license_file"] if p.get("license_file") else "UNKNOWN")
    # copyleft is acceptable only as one alternative of an OR expression
    alts = [a.strip(" ()") for a in re.split(r"\bOR\b|/", lic)]
    if all(COPYLEFT.search(a) for a in alts) or lic == "UNKNOWN":
        bad.append(f"{p['name']} {p['version']}: {lic}")
    rows.append(f"| {p['name']} | {p['version']} | {lic} |")

text = "\n".join([
    "# 제3자 라이선스 (Third-party licenses)",
    "",
    "StackLab 실행 파일(Windows x64)에 포함되는 크레이트 목록입니다. `python3 tools/licenses.py`로 생성되며",
    "CI가 최신 여부와 GPL 계열 단독 라이선스가 없음을 확인합니다(명세 12.3).",
    "",
    "그 외 구성요소: SQLite (Public Domain, rusqlite `bundled`로 포함).",
    "참고만 한 프로젝트(코드 미사용): solution-finder (MIT, Java) — PC 탐색 가지치기 아이디어.",
    "",
    f"총 {len(rows)}개",
    "",
    "| 크레이트 | 버전 | 라이선스 |",
    "|---|---|---|",
    *rows,
    "",
])

if bad:
    print("copyleft-only or unknown licences:\n  " + "\n  ".join(bad), file=sys.stderr)
    sys.exit(1)
if "--check" in sys.argv:
    cur = open(OUT, encoding="utf-8").read().replace("\r\n", "\n")
    if cur != text:
        print(f"{OUT} is out of date: run python3 tools/licenses.py", file=sys.stderr)
        sys.exit(1)
    print(f"{OUT}: {len(rows)} crates, no copyleft-only licences")
else:
    open(OUT, "w", encoding="utf-8", newline="\n").write(text)
    print(f"wrote {OUT}: {len(rows)} crates")
