# GrandMentor PR evidence

This orphan branch holds screenshots and GIFs used as evidence in pull requests, so they don't
bloat the code history. Files live under `prs/<branch-name>/` (slashes in the branch name become `-`).

Publish with `tools/screenshots/publish-evidence.sh <branch> <files...>` from the main repo and embed
the printed `https://raw.githubusercontent.com/xheisenbugx/GrandMentor/media/prs/...` URLs in the PR.

Images used by the README and docs live in `docs/media/` on `dev` instead.
