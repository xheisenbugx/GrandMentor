# Data attribution

## Puzzles (`data/puzzles.json`)

The tactics puzzles (ids prefixed `lc_`) come from the **Lichess puzzle database**
(https://database.lichess.org/#puzzles), released by lichess.org under
**CC0 1.0 (public domain dedication)**. Thank you to Lichess and its players.

Selection (see `tools/puzzles/select.py`): a streamed prefix of
`lichess_db_puzzle.csv.zst`, filtered to well-tested puzzles (NbPlays >= 500,
Popularity >= 85, RatingDeviation <= 90), rating 400-2799, stratified into 100-point
rating buckets (~175 each) and balanced across tactical themes. The lichess `PuzzleId`
is kept after the `lc_` prefix, so `https://lichess.org/training/<PuzzleId>` links back
to the original puzzle. Fields mapped: FEN, Moves (split, UCI; moves[0] is the
opponent's setup move), Rating, Themes (split), Popularity.
