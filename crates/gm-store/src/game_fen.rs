//! `games.final_fen`: the position after the last move, so the games list can draw thumbnails
//! without fetching every full game.
//!
//! * Written by the store on every insert / move change ([`final_fen_of`]).
//! * [`schema`] (migration v3) adds the column and backfills existing rows; [`backfill`] also
//!   runs after a backup import, so rows from older backups get one too.
//! * Robust to bad rows: an unreadable start position falls back to the standard start, and
//!   replay stops at the first illegal move (the thumbnail shows the last legal position).

use rusqlite::{params, Connection};
use shakmaty::{Chess, Position};

use crate::pgn;

/// Rows processed per backfill batch (bounds memory on big libraries).
const BATCH: usize = 500;

/// Adds `games.final_fen` and fills it for existing games. Idempotent.
pub(crate) fn schema(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    let has: bool = tx
        .prepare("SELECT 1 FROM pragma_table_info('games') WHERE name = 'final_fen'")?
        .exists([])?;
    if !has {
        tx.execute_batch("ALTER TABLE games ADD COLUMN final_fen TEXT NOT NULL DEFAULT ''")?;
    }
    backfill(tx)?;
    Ok(())
}

/// Full FEN after replaying `moves` (UCI, space separated or a list) from `start_fen`.
/// Never fails: see the module docs for how bad input is handled.
pub fn final_fen_of<S: AsRef<str>>(start_fen: &str, moves: &[S]) -> String {
    let mut pos: Chess = pgn::parse_position(start_fen).unwrap_or_default();
    for m in moves.iter().take(pgn::MAX_PLIES) {
        match pgn::uci_to_move(&pos, m.as_ref()) {
            Ok(mv) => pos.play_unchecked(&mv),
            Err(_) => break,
        }
    }
    pgn::position_fen(&pos)
}

/// Fills `final_fen` for every game that has none. Returns the number of rows updated.
pub(crate) fn backfill(conn: &Connection) -> rusqlite::Result<usize> {
    let mut updated = 0usize;
    let mut last_id = i64::MIN;
    loop {
        let rows: Vec<(i64, String, String)> = {
            let mut stmt = conn.prepare(
                "SELECT id, start_fen, moves FROM games WHERE final_fen = '' AND id > ?1 ORDER BY id LIMIT ?2",
            )?;
            let it = stmt.query_map(params![last_id, BATCH as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            it.collect::<Result<_, _>>()?
        };
        let Some(last) = rows.last() else { break };
        last_id = last.0;
        let mut upd = conn.prepare_cached("UPDATE games SET final_fen = ?2 WHERE id = ?1")?;
        for (id, start, moves) in &rows {
            let moves: Vec<&str> = moves.split_whitespace().collect();
            upd.execute(params![id, final_fen_of(start, &moves)])?;
            updated += 1;
        }
        if rows.len() < BATCH {
            break;
        }
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GameQuery, NewGame, Store};

    const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    #[test]
    fn final_fen_handles_bad_input() {
        assert_eq!(final_fen_of::<&str>("", &[]), START);
        assert_eq!(final_fen_of("garbage", &["e2e4"]), "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1");
        // Stops at the first illegal move.
        assert_eq!(final_fen_of("start", &["e2e4", "e2e4", "e7e5"]), final_fen_of("start", &["e2e4"]));
    }

    #[test]
    fn stored_and_backfilled() {
        let s = Store::open_in_memory().unwrap();
        let g = NewGame { moves: vec!["e2e4".into(), "e7e5".into(), "g1f3".into()], ..Default::default() };
        let id = s.create_game(&g).unwrap().id;
        let want = "rnbqkbnr/pppp1ppp/8/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq - 1 2";
        let list = s.list_games(&GameQuery::default()).unwrap();
        assert_eq!(list[0].final_fen, want);
        assert_eq!(list[0].last_move, "g1f3");
        // Moves edited -> final_fen follows.
        let patch = crate::GamePatch { moves: Some(vec!["d2d4".into()]), ..Default::default() };
        s.update_game(id, &patch).unwrap();
        let after = &s.list_games(&GameQuery::default()).unwrap()[0];
        assert_eq!(after.final_fen, final_fen_of("start", &["d2d4"]));
        assert_eq!(after.last_move, "d2d4");
        let promo = NewGame {
            start_fen: "8/4P3/8/8/8/8/k7/7K w - - 0 1".into(),
            moves: vec!["e7e8q".into()],
            ..Default::default()
        };
        s.create_game(&promo).unwrap();
        assert_eq!(s.list_games(&GameQuery::default()).unwrap()[0].last_move, "e7e8q");
        let empty = s.create_game(&NewGame::default()).unwrap();
        let list = s.list_games(&GameQuery::default()).unwrap();
        let e = list.iter().find(|g| g.id == empty.id).unwrap();
        assert_eq!((e.last_move.as_str(), e.final_fen.as_str()), ("", START));
        for g in list.iter().filter(|g| g.id != id) {
            s.delete_game(g.id).unwrap();
        }
        // Old rows (and bad rows) are backfilled.
        {
            let conn = s.conn.lock();
            conn.execute_batch(
                "UPDATE games SET final_fen = ''; \
                 INSERT INTO games (start_fen, moves) VALUES ('not a fen', 'e2e4 zzzz e7e5');",
            )
            .unwrap();
            assert_eq!(backfill(&conn).unwrap(), 2);
            assert_eq!(backfill(&conn).unwrap(), 0);
        }
        let list = s.list_games(&GameQuery::default()).unwrap();
        assert!(list.iter().all(|g| !g.final_fen.is_empty()));
        assert!(list.iter().any(|g| g.final_fen == final_fen_of("start", &["e2e4"])));
    }
}
