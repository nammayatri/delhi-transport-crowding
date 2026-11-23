//! Route and stop name normalisation and fuzzy matching.
//!
//! Ticket machines and GTFS spell the same stop differently ("Badarpur Border(T)" vs
//! "Badarpur Border (T)", "Mahipal Pur (Nh8)" vs "Mahipal Pur (NH-8)", "Laxmi Nagar Xing"
//! vs "Laxmi Nagar Crossing"). We normalise both sides, expand common abbreviations, and
//! score with token-based similarity. Matching is always restricted to the stops of the
//! ticket's own route, which keeps false matches rare.

use std::collections::BTreeSet;

/// Normalise a route name: upper case, no spaces, `DWN` spelled `DOWN`.
pub fn norm_route(s: &str) -> String {
    let r: String = s
        .trim()
        .to_ascii_uppercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    match r.strip_suffix("DWN") {
        Some(b) => format!("{b}DOWN"),
        None => r,
    }
}

/// Route names to try for a ticket's route, most specific first.
pub fn route_candidates(s: &str) -> Vec<String> {
    let n = norm_route(s);
    let mut v = vec![n.clone()];
    let t = n.trim_end_matches(|c: char| c.is_ascii_digit());
    if t.len() != n.len() && (t.ends_with("UP") || t.ends_with("DOWN")) {
        v.push(t.to_string());
    }
    v
}

/// The same route in the other direction (`8UP` <-> `8DOWN`).
pub fn opposite_direction(n: &str) -> Option<String> {
    if let Some(b) = n.strip_suffix("DOWN") {
        Some(format!("{b}UP"))
    } else {
        n.strip_suffix("UP").map(|b| format!("{b}DOWN"))
    }
}

const SYNONYMS: &[(&str, &str)] = &[
    ("xing", "crossing"),
    ("crsg", "crossing"),
    ("crossin", "crossing"),
    ("bdr", "border"),
    ("brdr", "border"),
    ("mkt", "market"),
    ("stn", "station"),
    ("sation", "station"),
    ("rd", "road"),
    ("sec", "sector"),
    ("sect", "sector"),
    ("ph", "phase"),
    ("vill", "village"),
    ("vlg", "village"),
    ("dpt", "depot"),
    ("hosp", "hospital"),
    ("t", "terminal"),
    ("term", "terminal"),
    ("ext", "extension"),
    ("extn", "extension"),
    ("jn", "junction"),
    ("jnc", "junction"),
    ("chk", "chowk"),
    ("mor", "more"),
    ("clny", "colony"),
    ("opp", "opposite"),
    ("encl", "enclave"),
    ("apts", "apartments"),
    ("apt", "apartment"),
    ("instt", "institute"),
    ("inst", "institute"),
    ("govt", "government"),
    ("sch", "school"),
    ("blk", "block"),
    ("hr", "higher"),
    ("sr", "senior"),
];

fn synonym(tok: &str) -> &str {
    SYNONYMS
        .iter()
        .find(|(a, _)| *a == tok)
        .map(|(_, b)| *b)
        .unwrap_or(tok)
}

/// Normalise a stop name to lower-case tokens with abbreviations expanded.
pub fn norm_stop(s: &str) -> String {
    let lower = s.to_lowercase().replace('&', " and ");
    let mut buf = String::with_capacity(lower.len() + 8);
    let mut prev = ' ';
    for ch in lower.chars() {
        let c = if ch.is_alphanumeric() { ch } else { ' ' };
        if (prev.is_ascii_digit() && c.is_alphabetic())
            || (prev.is_alphabetic() && c.is_ascii_digit())
        {
            buf.push(' ');
        }
        buf.push(c);
        prev = c;
    }
    let toks: Vec<&str> = buf.split_whitespace().map(synonym).collect();
    toks.join(" ")
}

/// Alternative spellings packed into one GTFS name: "Saidul-A-Jab / Saket Metro Station"
/// yields the whole name and each side of the slash; a trailing "(...)" is also dropped.
pub fn alternates(raw: &str) -> Vec<String> {
    let mut out = vec![norm_stop(raw)];
    if raw.contains('/') {
        out.extend(raw.split('/').map(norm_stop).filter(|s| !s.is_empty()));
    }
    if let Some(i) = raw.find('(') {
        let base = norm_stop(&raw[..i]);
        if !base.is_empty() {
            out.push(base);
        }
    }
    out.sort();
    out.dedup();
    out
}

fn lcs_len(a: &[u8], b: &[u8]) -> usize {
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let mut prev = vec![0usize; b.len() + 1];
    let mut cur = vec![0usize; b.len() + 1];
    for &x in a {
        for (j, &y) in b.iter().enumerate() {
            cur[j + 1] = if x == y {
                prev[j] + 1
            } else {
                prev[j + 1].max(cur[j])
            };
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Normalised indel similarity, 0..100.
pub fn ratio(a: &str, b: &str) -> f64 {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.is_empty() && b.is_empty() {
        return 100.0;
    }
    200.0 * lcs_len(a, b) as f64 / (a.len() + b.len()) as f64
}

pub fn token_sort_ratio(a: &str, b: &str) -> f64 {
    let sort = |s: &str| {
        let mut t: Vec<&str> = s.split_whitespace().collect();
        t.sort_unstable();
        t.join(" ")
    };
    ratio(&sort(a), &sort(b))
}

pub fn token_set_ratio(a: &str, b: &str) -> f64 {
    let sa: BTreeSet<&str> = a.split_whitespace().collect();
    let sb: BTreeSet<&str> = b.split_whitespace().collect();
    let inter: Vec<&str> = sa.intersection(&sb).copied().collect();
    let da: Vec<&str> = sa.difference(&sb).copied().collect();
    let db: Vec<&str> = sb.difference(&sa).copied().collect();
    let t0 = inter.join(" ");
    let join = |x: &str, y: &[&str]| {
        if y.is_empty() {
            x.to_string()
        } else if x.is_empty() {
            y.join(" ")
        } else {
            format!("{x} {}", y.join(" "))
        }
    };
    let t1 = join(&t0, &da);
    let t2 = join(&t0, &db);
    let mut best = ratio(&t1, &t2);
    if !t0.is_empty() {
        best = best.max(ratio(&t0, &t1)).max(ratio(&t0, &t2));
    }
    best
}

/// Similarity of an already-normalised ticket name with one normalised GTFS alternate.
pub fn score_norm(t: &str, c: &str) -> f64 {
    if t == c {
        return 100.0;
    }
    if t.is_empty() || c.is_empty() {
        return 0.0;
    }
    let blended = 0.5 * (token_set_ratio(t, c) + token_sort_ratio(t, c));
    let nospace = |s: &str| s.replace(' ', "");
    blended.max(ratio(&nospace(t), &nospace(c)))
}

/// Best similarity of a normalised ticket name with any alternate of a GTFS stop name.
pub fn score(t_norm: &str, alts: &[String]) -> f64 {
    alts.iter()
        .map(|c| score_norm(t_norm, c))
        .fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_names() {
        assert_eq!(norm_route(" 8 dwn"), "8DOWN");
        assert_eq!(route_candidates("522BDOWN2"), vec!["522BDOWN2", "522BDOWN"]);
        assert_eq!(route_candidates("8UP"), vec!["8UP"]);
        assert_eq!(opposite_direction("8ADOWN").as_deref(), Some("8AUP"));
        assert_eq!(
            opposite_direction("RL-77EXTUP").as_deref(),
            Some("RL-77EXTDOWN")
        );
    }

    #[test]
    fn stop_normalisation() {
        assert_eq!(norm_stop("Badarpur Border(T)"), "badarpur border terminal");
        assert_eq!(norm_stop("Mahipal Pur (Nh8)"), "mahipal pur nh 8");
        assert_eq!(norm_stop("Mahipal Pur (NH-8)"), "mahipal pur nh 8");
        assert_eq!(norm_stop("Laxmi Nagar Xing"), "laxmi nagar crossing");
        assert_eq!(norm_stop("Kalindi Kunj Bdr"), "kalindi kunj border");
    }

    fn s(t: &str, g: &str) -> f64 {
        score(&norm_stop(t), &alternates(g))
    }

    #[test]
    fn fuzzy_scores_rank_the_right_stop_first() {
        assert_eq!(s("Badarpur Border(T)", "Badarpur Border (T)"), 100.0);
        assert!(s("Badarpur Border", "Badarpur Border (T)") >= 90.0);
        assert!(
            s("Badarpur Border", "Badarpur Border (T)") > s("Badarpur Border", "Badarpur Village")
        );
        assert_eq!(
            s("Saket Metro Station", "Saidul-A-Jab / Saket Metro Station"),
            100.0
        );
        assert!(
            s("Mahipal Pur Village", "Mahipal Pur")
                > s("Mahipal Pur Village", "Masood Pur Village")
        );
        assert!(s("Mahipalpur", "Mahipal Pur") >= 95.0);
        assert!(s("Ali Village", "Ali Ganj") < 82.0);
    }

    #[test]
    fn token_set_handles_subsets() {
        assert_eq!(token_set_ratio("a b", "a b c"), 100.0);
        assert!(token_sort_ratio("a b", "a b c") < 100.0);
    }
}
