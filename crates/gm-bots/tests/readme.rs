//! The README's bot count matches the shipped personas, so the docs cannot drift silently.

use gm_bots::ADAPTIVE_ID;
use gm_content::Lang;

const WORDS: [&str; 21] = [
    "Zero", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine", "Ten", "Eleven", "Twelve", "Thirteen",
    "Fourteen", "Fifteen", "Sixteen", "Seventeen", "Eighteen", "Nineteen", "Twenty",
];

#[test]
fn readme_bot_counts_match_personas() {
    let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../README.md")).expect("README.md");
    let bots = gm_bots::list(Lang::En);
    let coaches = bots.iter().filter(|b| b.category == "coach").count();
    let ladder = bots.iter().filter(|b| b.category != "coach" && b.id != ADAPTIVE_ID).count();
    assert!(bots.iter().any(|b| b.id == ADAPTIVE_ID), "the adaptive bot is listed");
    let rated = format!("{} rated opponents", WORDS.get(ladder).copied().unwrap_or("Many"));
    assert!(readme.contains(&rated), "README.md should say \"{rated}\" (Tour → Play against bots)");
    let coach_words = ["no", "one", "two", "three", "four"];
    let coach = format!("{} coach bots", coach_words.get(coaches).copied().unwrap_or("several"));
    assert!(readme.contains(&coach), "README.md should say \"{coach}\"");
}
