# Piece set attribution

The SVG piece sets in this directory were downloaded unmodified from the
[lichess.org](https://github.com/lichess-org/lila) repository (`public/piece/<set>/`).
Licensing information below is taken from lila's `COPYING.md`.

| Directory | Author | License |
|---|---|---|
| `cburnett/` | [Colin M.L. Burnett](https://en.wikipedia.org/wiki/User:Cburnett) | [GPLv2+](https://www.gnu.org/licenses/gpl-2.0.txt) |
| `merida/` | Armando Hernandez Marroquin | [GPLv2+](https://www.gnu.org/licenses/gpl-2.0.txt) |
| `chessnut/` | [Alexis Luengas](https://github.com/LexLuengas/chessnut-pieces) | [Apache 2.0](chessnut/LICENSE.txt) (full text in `chessnut/LICENSE.txt`) |

All three sets may be used, modified and redistributed for any purpose, including commercially,
under the terms of their licences.

The `alpha` set that GrandMentor used to ship (Eric Bentzen, "free for personal non commercial use")
was removed because its licence does not allow commercial use. Players who had chosen it are moved
to `chessnut` automatically (`web/js/settings.js`).

File naming: `<color><piece>.svg` where color is `w`/`b` and piece is one of `K Q R B N P`.
