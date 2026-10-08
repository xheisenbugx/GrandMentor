#!/usr/bin/env python3
"""Select a stratified, high-quality tactics set from the Lichess puzzle DB (CC0).

Usage (streams; never stores the full ~1GB file):
  curl -sL https://database.lichess.org/lichess_db_puzzle.csv.zst | zstd -dc \
    | head -n 1500001 > /tmp/raw.csv
  python3 -I tools/puzzles/select.py /tmp/raw.csv data/puzzles.json [target=4200]

Filters: NbPlays>=500, Popularity>=85, RatingDeviation<=90, rating 400..2800,
2..14 moves. Stratified into 100-point rating buckets; within each bucket puzzles
are picked round-robin over target themes (rarest-first) to maximise variety,
best quality (popularity, then plays) first. Output: contract Puzzle shape.
"""
import csv, json, sys, collections

TARGET_THEMES = [
    "mateIn1", "mateIn2", "mateIn3", "mateIn4", "mateIn5",
    "fork", "pin", "skewer", "discoveredAttack", "doubleCheck", "hangingPiece",
    "sacrifice", "deflection", "attraction", "clearance", "interference",
    "intermezzo", "xRayAttack", "trappedPiece", "capturingDefender",
    "backRankMate", "smotheredMate", "anastasiaMate", "arabianMate", "bodenMate",
    "doubleBishopMate", "dovetailMate", "hookMate", "killBoxMate", "vukovicMate",
    "promotion", "underPromotion", "advancedPawn", "enPassant", "castling",
    "defensiveMove", "quietMove", "zugzwang", "equality", "exposedKing",
    "kingsideAttack", "queensideAttack", "attackingF2F7",
    "endgame", "rookEndgame", "pawnEndgame", "bishopEndgame", "knightEndgame",
    "queenEndgame", "queenRookEndgame", "opening", "middlegame",
]

def main():
    src, dst = sys.argv[1], sys.argv[2]
    target = int(sys.argv[3]) if len(sys.argv) > 3 else 4200
    buckets = collections.defaultdict(list)
    seen = 0
    with open(src, newline="") as f:
        for row in csv.DictReader(f):
            seen += 1
            try:
                rating = int(row["Rating"]); rd = int(row["RatingDeviation"])
                pop = int(row["Popularity"]); plays = int(row["NbPlays"])
            except (ValueError, KeyError, TypeError):
                continue
            if plays < 500 or pop < 85 or rd > 90 or not (400 <= rating < 2800):
                continue
            moves = row["Moves"].split()
            if not (2 <= len(moves) <= 14) or len(moves) % 2:
                continue
            fen = row["FEN"].strip()
            if len(fen.split()) != 6:
                continue
            themes = row["Themes"].split()
            buckets[rating // 100].append({
                "id": "lc_" + row["PuzzleId"], "fen": fen, "moves": moves,
                "rating": rating, "themes": themes, "popularity": pop, "_plays": plays,
            })
    nb = list(range(4, 28))
    per = target // len(nb)
    # global theme frequency among candidates: rare themes get picked first
    freq = collections.Counter(t for b in buckets.values() for p in b for t in p["themes"])
    order = sorted((t for t in TARGET_THEMES if freq[t]), key=lambda t: freq[t])
    out, deficit = [], 0
    for b in nb:
        cands = sorted(buckets.get(b, []), key=lambda p: (-p["popularity"], -p["_plays"]))
        quota = per + deficit
        by_theme = {t: [p for p in cands if t in p["themes"]] for t in order}
        used, picked = set(), []
        progress = True
        while len(picked) < quota and progress:
            progress = False
            for t in order:
                if len(picked) >= quota:
                    break
                lst = by_theme[t]
                while lst and lst[0]["id"] in used:
                    lst.pop(0)
                if lst:
                    p = lst.pop(0); used.add(p["id"]); picked.append(p); progress = True
        for p in cands:  # top up with best remaining
            if len(picked) >= quota:
                break
            if p["id"] not in used:
                used.add(p["id"]); picked.append(p)
        deficit = quota - len(picked)
        out.extend(picked)
    out.sort(key=lambda p: (p["rating"], p["id"]))
    for p in out:
        p.pop("_plays", None)
    with open(dst, "w") as f:
        f.write("[\n")
        f.write(",\n".join(json.dumps(p, separators=(",", ":")) for p in out))
        f.write("\n]\n")
    tc = collections.Counter(t for p in out for t in p["themes"])
    print(f"scanned {seen} rows, wrote {len(out)} puzzles (deficit carried at end: {deficit})", file=sys.stderr)
    print("per bucket:", dict(collections.Counter(p["rating"] // 100 * 100 for p in out)), file=sys.stderr)
    print("themes:", dict(sorted(tc.items(), key=lambda x: -x[1])), file=sys.stderr)

if __name__ == "__main__":
    main()
