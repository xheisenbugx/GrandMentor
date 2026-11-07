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

## Piece images (`web/img/pieces/`)

The board pieces are the *cburnett* and *merida* sets (GPLv2+) and the *chessnut* set by
Alexis Luengas (Apache 2.0), taken unmodified from the lichess.org repository. Details and
links: [`web/img/pieces/LICENSE.md`](../web/img/pieces/LICENSE.md).

## Font (`web/fonts/`)

The interface font is **Inter** by Rasmus Andersson and the Inter Project Authors
(https://github.com/rsms/inter), licensed under the **SIL Open Font License 1.1**
(full text: [`web/fonts/Inter-OFL.txt`](../web/fonts/Inter-OFL.txt)). The Latin and Latin Extended
variable subsets (`inter-latin-wght-normal.woff2`, `inter-latin-ext-wght-normal.woff2`) are taken
unmodified from the `@fontsource-variable/inter` 5.3.0 npm package (tarball sha512 checked against the
registry). They are served by the app itself, so nothing is loaded from Google Fonts and the app looks
the same offline.

## chess.js (`web/vendor/chess.js`)

Move generation in the browser uses **chess.js** v1.4.0 by Jeff Hlywa
(https://github.com/jhlywa/chess.js), licensed under the **BSD 2-Clause License**. The file is the
unmodified ESM build; its copyright notice and the full license text are kept in the file header.
