#!/usr/bin/env python3
"""Minimal fumen (v115) decoder: prints every page's field as text.

Used in STEP 4 to transcribe public opener diagrams (harddrop wiki) into data/openers/*.toml.
Usage: python3 tools/fumen_decode.py 'v115@...'
Cell letters: I L O Z T J S, X = garbage, . = empty. Bottom row last.
"""
import sys

ENC = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
PIECES = ".ILOZTJSX"
FIELD_TOP = 23
FIELD_W = 10
FIELD_BLOCKS = (FIELD_TOP + 1) * FIELD_W  # incl. the garbage row below the floor

# piece shapes (fumen convention: rotation 0=spawn,1=R,2=180,3=L; y up, x right)
SHAPES = {
    1: [(-1, 0), (0, 0), (1, 0), (2, 0)],   # I
    2: [(-1, 0), (0, 0), (1, 0), (1, 1)],   # L
    3: [(0, 0), (1, 0), (0, 1), (1, 1)],    # O
    4: [(-1, 1), (0, 1), (0, 0), (1, 0)],   # Z
    5: [(-1, 0), (0, 0), (1, 0), (0, 1)],   # T
    6: [(-1, 1), (-1, 0), (0, 0), (1, 0)],  # J
    7: [(-1, 0), (0, 0), (0, 1), (1, 1)],   # S
}


def rot(cells, r):
    for _ in range(r):
        cells = [(y, -x) for (x, y) in cells]
    return cells


def piece_cells(t, r, x, y):
    cells = SHAPES[t]
    if t == 1:  # I rotation centre handling in fumen
        base = {0: [(-1, 0), (0, 0), (1, 0), (2, 0)], 1: [(0, 1), (0, 0), (0, -1), (0, -2)],
                2: [(-2, 0), (-1, 0), (0, 0), (1, 0)], 3: [(0, 2), (0, 1), (0, 0), (0, -1)]}[r]
        return [(x + a, y + b) for a, b in base]
    if t == 3:
        base = {0: [(0, 0), (1, 0), (0, 1), (1, 1)], 1: [(0, 0), (1, 0), (0, -1), (1, -1)],
                2: [(-1, 0), (0, 0), (-1, -1), (0, -1)], 3: [(-1, 0), (0, 0), (-1, 1), (0, 1)]}[r]
        return [(x + a, y + b) for a, b in base]
    # fumen rotation: 0 spawn, 1 = R (clockwise) ... our rot() is clockwise
    return [(x + a, y + b) for a, b in rot(cells, r)]


class Reader:
    def __init__(self, s):
        self.s = s.replace("?", "")
        self.i = 0

    def poll(self, n):
        v = 0
        for k in range(n):
            v += ENC.index(self.s[self.i + k]) * (64 ** k)
        self.i += n
        return v

    def done(self):
        return self.i >= len(self.s)


PARTIAL = []


def decode(data):
    assert data.startswith("v115@"), "only v115 supported"
    r = Reader(data[5:])
    pages = PARTIAL
    pages.clear()
    prev = [0] * FIELD_BLOCKS
    repeat = 0
    while not r.done():
        start = r.i
        field = prev[:]
        if repeat > 0:
            repeat -= 1
        else:
            i = 0
            while i < FIELD_BLOCKS:
                v = r.poll(2)
                diff, count = v // FIELD_BLOCKS, v % FIELD_BLOCKS + 1
                if diff == 8 and count == FIELD_BLOCKS and i == 0:
                    repeat = r.poll(1)
                if i + count > FIELD_BLOCKS:
                    raise ValueError(f"corrupt field data at offset {start} (page {len(pages) + 1})")
                for k in range(count):
                    field[i + k] = prev[i + k] + diff - 8
                i += count
        v = r.poll(3)
        t = v % 8; v //= 8
        rr = v % 4; v //= 4
        coord = v % FIELD_BLOCKS; v //= FIELD_BLOCKS
        v //= 2  # block up
        v //= 2  # mirror
        v //= 2  # colour (first page)
        has_comment = v % 2 == 1; v //= 2
        lock = v % 2 == 0
        comment = ""
        if has_comment:
            n = r.poll(2)
            chars = []
            for _ in range((n + 3) // 4):
                cv = r.poll(5)
                for _ in range(4):
                    chars.append(cv % 96)
                    cv //= 96
            comment = "".join(chr(c + 32) for c in chars[:n])
        pages.append({"field": field[:], "piece": (t, rr, coord), "comment": comment, "lock": lock})
        nxt = field[:]
        if t != 0:
            x = coord % FIELD_W
            y = FIELD_TOP - 1 - coord // FIELD_W
            for (cx, cy) in piece_cells(t, rr, x, y):
                idx = (FIELD_TOP - 1 - cy) * FIELD_W + cx
                if 0 <= idx < FIELD_BLOCKS:
                    nxt[idx] = t
        if lock:
            rows = [nxt[k * FIELD_W:(k + 1) * FIELD_W] for k in range(FIELD_TOP)]
            kept = [row for row in rows if not all(c != 0 for c in row)]
            nxt = [0] * ((FIELD_TOP - len(kept)) * FIELD_W) + sum(kept, []) + nxt[FIELD_TOP * FIELD_W:]
        prev = nxt
    return pages


def show(field, with_piece=None):
    rows = []
    for k in range(FIELD_TOP):
        row = field[k * FIELD_W:(k + 1) * FIELD_W]
        if any(row):
            rows.append("".join(PIECES[c] for c in row))
    return rows


if __name__ == "__main__":
    try:
        decoded = decode(sys.argv[1])
    except ValueError as e:
        print("!!", e)
        decoded = PARTIAL
    for n, p in enumerate(decoded):
        print(f"-- page {n + 1} piece={p['piece']} comment={p['comment']!r}")
        for line in show(p["field"]):
            print("   " + line)
