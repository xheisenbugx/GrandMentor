//! Content translation overlays: `data/i18n/<lang>/{courses,openings,endgames}*.json`.
//!
//! Every file whose name starts with `courses`, `openings` or `endgames` and ends in `.json` is
//! merged (later files, in name order, win on conflicts). Overlays only ever replace *text*:
//! titles, descriptions, step text, task prompts/hints/success messages (plus guess notes and
//! choice options), opening names/ideas,
//! endgame hints... Moves, FENs, arrows, highlights and solutions always come from the English
//! source. Missing ids, fields or steps fall back to English; unknown ids are ignored (and
//! counted in the load log). Schema: `docs/I18N.md`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::{Content, Lang};

/// Bounds for hostile / broken overlay folders.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FILES: usize = 64;
const MAX_TEXT_CHARS: usize = 20_000;
const MAX_LIST_ITEMS: usize = 64;

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct TaskOverlay {
    pub prompt: Option<String>,
    pub hint: Option<String>,
    pub success: Option<String>,
    /// `guess` tasks: game caption and one note per learner move (same order; `null` = keep English).
    pub game: Option<String>,
    pub notes: Vec<Option<String>>,
    /// `choice` tasks: option text/explanation, same order as the English options.
    pub options: Vec<Option<ChoiceOverlay>>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ChoiceOverlay {
    pub text: Option<String>,
    pub explain: Option<String>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct StepOverlay {
    pub text: Option<String>,
    pub task: Option<TaskOverlay>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct LessonOverlay {
    pub title: Option<String>,
    pub summary: Option<String>,
    /// Same order/length as the English steps; `null` = untranslated step.
    pub steps: Vec<Option<StepOverlay>>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct CourseOverlay {
    pub title: Option<String>,
    pub description: Option<String>,
    pub lessons: BTreeMap<String, LessonOverlay>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct OpeningOverlay {
    pub name: Option<String>,
    pub family: Option<String>,
    pub description: Option<String>,
    pub ideas: Option<Vec<String>>,
    pub traps: Option<Vec<String>>,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct EndgameOverlay {
    pub title: Option<String>,
    pub description: Option<String>,
    pub hint: Option<String>,
    pub technique: Option<Vec<String>>,
}

/// All overlays for one language.
#[derive(Clone, Debug, Default)]
pub struct Overlay {
    pub courses: BTreeMap<String, CourseOverlay>,
    pub openings: BTreeMap<String, OpeningOverlay>,
    pub endgames: BTreeMap<String, EndgameOverlay>,
}

/// What applying an overlay did (for the load log and tests).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OverlayStats {
    /// Entries (courses, lessons, steps, openings, endgames) whose text was replaced.
    pub applied: usize,
    /// Entries ignored: unknown ids, surplus steps, unparsable entries.
    pub ignored: usize,
}

impl Overlay {
    pub fn is_empty(&self) -> bool {
        self.courses.is_empty() && self.openings.is_empty() && self.endgames.is_empty()
    }
}

fn read_object(path: &Path) -> Option<serde_json::Map<String, serde_json::Value>> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() > MAX_FILE_BYTES => {
            tracing::warn!("{}: overlay larger than {MAX_FILE_BYTES} bytes; ignored", path.display());
            return None;
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!("{}: {e}", path.display());
            return None;
        }
    }
    let bytes = std::fs::read(path).map_err(|e| tracing::warn!("{}: {e}", path.display())).ok()?;
    match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(serde_json::Value::Object(m)) => Some(m),
        Ok(_) => {
            tracing::warn!("{}: overlay must be a JSON object keyed by id; ignored", path.display());
            None
        }
        Err(e) => {
            tracing::warn!("{}: invalid JSON ({e}); ignored", path.display());
            None
        }
    }
}

/// Deep-merge `new` into `old`: objects merge per key, arrays merge per index (a `null`
/// element keeps the earlier value), `null` never erases, anything else replaces.
fn deep_merge(old: &mut serde_json::Value, new: serde_json::Value) {
    use serde_json::Value;
    match (old, new) {
        (_, Value::Null) => {}
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                match a.get_mut(&k) {
                    Some(slot) => deep_merge(slot, v),
                    None => {
                        a.insert(k, v);
                    }
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            for (i, v) in b.into_iter().enumerate() {
                match a.get_mut(i) {
                    Some(slot) => deep_merge(slot, v),
                    None => a.push(v),
                }
            }
        }
        (slot, v) => *slot = v,
    }
}

/// Collect the entries of one file into `into` (deep-merged with earlier files).
fn collect_entries(obj: serde_json::Map<String, serde_json::Value>, into: &mut BTreeMap<String, serde_json::Value>) {
    for (id, v) in obj {
        if id.starts_with('_') || id.starts_with('$') {
            continue; // comments / metadata keys like "_comment" or "$schema"
        }
        match into.get_mut(&id) {
            Some(slot) => deep_merge(slot, v),
            None => {
                into.insert(id, v);
            }
        }
    }
}

/// Deserialize merged entries, skipping (and counting) malformed ones.
fn typed<T: serde::de::DeserializeOwned>(kind: &str, lang: Lang, raw: BTreeMap<String, serde_json::Value>, bad: &mut usize) -> BTreeMap<String, T> {
    let mut out = BTreeMap::new();
    for (id, v) in raw {
        match serde_json::from_value::<T>(v) {
            Ok(t) => {
                out.insert(id, t);
            }
            Err(e) => {
                *bad += 1;
                tracing::warn!("i18n/{lang}: {kind} overlay entry {id:?} skipped: {e}");
            }
        }
    }
    out
}

/// Load `<dir>/<lang>/` overlays. A missing folder is an empty overlay. Returns the overlay and
/// the number of files / entries that could not be parsed.
pub fn load_overlay(i18n_dir: &Path, lang: Lang) -> (Overlay, usize) {
    let mut bad = 0usize;
    let dir = i18n_dir.join(lang.code());
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return (Overlay::default(), 0);
    };
    let mut files: Vec<std::path::PathBuf> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();
    if files.len() > MAX_FILES {
        tracing::warn!("{}: more than {MAX_FILES} overlay files; extra files ignored", dir.display());
        files.truncate(MAX_FILES);
    }
    let (mut courses, mut openings, mut endgames) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    for path in files {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
        let target = if name.starts_with("courses") {
            &mut courses
        } else if name.starts_with("openings") {
            &mut openings
        } else if name.starts_with("endgames") {
            &mut endgames
        } else {
            tracing::debug!("{}: not an overlay file; ignored", path.display());
            continue;
        };
        match read_object(&path) {
            Some(obj) => collect_entries(obj, target),
            None => bad += 1,
        }
    }
    let ov = Overlay {
        courses: typed("course", lang, courses, &mut bad),
        openings: typed("opening", lang, openings, &mut bad),
        endgames: typed("endgame", lang, endgames, &mut bad),
    };
    (ov, bad)
}

/// Replace `dst` with a usable translation (non-blank, bounded); keep English otherwise.
fn set_text(dst: &mut String, src: &Option<String>) -> bool {
    match src.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() && s.chars().count() <= MAX_TEXT_CHARS => {
            *dst = s.to_string();
            true
        }
        _ => false,
    }
}

fn set_list(dst: &mut Vec<String>, src: &Option<Vec<String>>) -> bool {
    match src {
        Some(items) if !items.is_empty() && items.len() <= MAX_LIST_ITEMS => {
            let clean: Vec<String> = items
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty() && s.chars().count() <= MAX_TEXT_CHARS)
                .collect();
            if clean.is_empty() {
                return false;
            }
            *dst = clean;
            true
        }
        _ => false,
    }
}

/// Text of the interactive task kinds (game caption, guess notes, choice options).
fn apply_task_extra(extra: &mut crate::steps::TaskExtra, to: &TaskOverlay) -> bool {
    let mut any = false;
    if let Some(g) = extra.game.as_mut() {
        any |= set_text(g, &to.game);
    }
    for (note, tr) in extra.notes.iter_mut().zip(&to.notes) {
        any |= set_text(note, tr);
    }
    for (opt, tr) in extra.options.iter_mut().zip(&to.options) {
        if let Some(tr) = tr {
            any |= set_text(&mut opt.text, &tr.text);
            any |= set_text(&mut opt.explain, &tr.explain);
        }
    }
    any
}

/// Apply `ov` to `content` in place (text only). Returns what was applied / ignored.
pub fn apply_overlay(content: &mut Content, ov: &Overlay) -> OverlayStats {
    let mut st = OverlayStats::default();

    for (id, co) in &ov.courses {
        let Some(course) = content.courses.iter_mut().find(|c| &c.id == id) else {
            st.ignored += 1;
            continue;
        };
        let mut any = set_text(&mut course.title, &co.title);
        any |= set_text(&mut course.description, &co.description);
        if any {
            st.applied += 1;
        }
        for (lid, lo) in &co.lessons {
            let Some(lesson) = course.lessons.iter_mut().find(|l| &l.id == lid) else {
                st.ignored += 1;
                continue;
            };
            let mut any = set_text(&mut lesson.title, &lo.title);
            any |= set_text(&mut lesson.summary, &lo.summary);
            if any {
                st.applied += 1;
            }
            if lo.steps.len() > lesson.steps.len() {
                st.ignored += lo.steps.len() - lesson.steps.len();
            }
            for (step, so) in lesson.steps.iter_mut().zip(&lo.steps) {
                let Some(so) = so else { continue };
                let mut any = set_text(&mut step.text, &so.text);
                if let (Some(task), Some(to)) = (step.task.as_mut(), so.task.as_ref()) {
                    any |= set_text(&mut task.prompt, &to.prompt);
                    any |= set_text(&mut task.success, &to.success);
                    any |= apply_task_extra(&mut task.extra, to);
                    if let Some(h) = task.hint.as_mut() {
                        any |= set_text(h, &to.hint);
                    } else if to.hint.as_deref().is_some_and(|h| !h.trim().is_empty()) {
                        let mut h = String::new();
                        if set_text(&mut h, &to.hint) {
                            task.hint = Some(h);
                            any = true;
                        }
                    }
                }
                if any {
                    st.applied += 1;
                }
            }
        }
    }

    for (id, oo) in &ov.openings {
        let Some(o) = content.openings.iter_mut().find(|o| &o.id == id) else {
            st.ignored += 1;
            continue;
        };
        let mut any = set_text(&mut o.name, &oo.name);
        any |= set_text(&mut o.family, &oo.family);
        any |= set_text(&mut o.description, &oo.description);
        any |= set_list(&mut o.ideas, &oo.ideas);
        any |= set_list(&mut o.traps, &oo.traps);
        if any {
            st.applied += 1;
        }
    }

    for (id, eo) in &ov.endgames {
        let Some(d) = content.endgames.iter_mut().find(|d| &d.id == id) else {
            st.ignored += 1;
            continue;
        };
        let mut any = set_text(&mut d.title, &eo.title);
        any |= set_text(&mut d.description, &eo.description);
        any |= set_text(&mut d.hint, &eo.hint);
        any |= set_list(&mut d.technique, &eo.technique);
        if any {
            st.applied += 1;
        }
    }
    st
}
