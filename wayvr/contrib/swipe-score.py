#!/usr/bin/env python3
"""Score a WAYVR_SWIPE_LOG file against the words you meant to swipe.

Swipe the words of WORDS_FILE in order (one swipe per word; if a swipe goes
wrong, still move on to the next word). Then:

    python3 swipe-score.py swipes.jsonl words.txt

Prints per-word ranks and top-1/top-4 per engine.
"""
import json
import sys


def main(log_path, words_path):
    words = open(words_path).read().split()
    swipes = [
        json.loads(line)
        for line in open(log_path)
        if line.strip() and json.loads(line).get("type") == "swipe"
    ]
    if len(swipes) != len(words):
        print(f"warning: {len(swipes)} swipes vs {len(words)} words; aligning the first {min(len(swipes), len(words))}")

    engines = ("futo", "cleverkeys")
    hits = {e: {"top1": 0, "top4": 0, "ms": []} for e in engines}
    n = 0
    for word, swipe in zip(words, swipes):
        n += 1
        cells = []
        for e in engines:
            res = swipe.get(e) or {}
            got = [w.lower() for w in res.get("words", [])]
            rank = got.index(word.lower()) if word.lower() in got else None
            hits[e]["top1"] += rank == 0
            hits[e]["top4"] += rank is not None and rank < 4
            if "ms" in res:
                hits[e]["ms"].append(res["ms"])
            cells.append(f"{e}={'-' if rank is None else rank + 1}:{','.join(got[:4]) or res.get('error', 'n/a')}")
        print(f"{word:>12}  {len(swipe['points']):3d}pts  " + "  ".join(cells))

    for e in engines:
        ms = sorted(hits[e]["ms"])
        median = f"{ms[len(ms) // 2]:.0f}ms" if ms else "n/a"
        print(f"{e:>10}: top-1 {hits[e]['top1']}/{n}  top-4 {hits[e]['top4']}/{n}  median latency {median}")


if __name__ == "__main__":
    main(*sys.argv[1:3])
