// Window-query resolution: how a user-supplied query string (ccectl
// focus-window / center-window, window-stream subscriptions) picks a window.
//
// The candidate list is the mechanism's job: it passes only MAPPED windows,
// in window order. This module owns just the matching rules.

/// A mapped window's identity, as query resolution sees it.
#[derive(Debug, Clone)]
pub struct QueryCandidate {
    /// The numeric window id users see (the slotmap key index).
    pub index: u32,
    pub app_id: Option<String>,
}

/// Resolve a query to a position in `candidates`, or None.
///
/// An all-numeric query is tried as an exact window id first. Failing that
/// (non-numeric, or no window has that id), it is matched case-insensitively
/// against app_ids: an exact match beats a substring match, and the first
/// window at the best score wins.
pub fn find_window(candidates: &[QueryCandidate], query: &str) -> Option<usize> {
    let query = query.to_lowercase();

    if let Ok(id) = query.parse::<u32>() {
        if let Some(pos) = candidates.iter().position(|c| c.index == id) {
            return Some(pos);
        }
    }

    let mut best: Option<usize> = None;
    let mut best_score = 0;
    for (pos, c) in candidates.iter().enumerate() {
        let Some(aid) = &c.app_id else { continue };
        let aid = aid.to_lowercase();
        let score = if aid == query {
            100
        } else if aid.contains(&query) {
            50
        } else {
            0
        };
        if score > best_score {
            best_score = score;
            best = Some(pos);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(index: u32, app_id: Option<&str>) -> QueryCandidate {
        QueryCandidate { index, app_id: app_id.map(str::to_string) }
    }

    #[test]
    fn numeric_query_matches_window_id_first() {
        let c = [cand(3, Some("2048-game")), cand(7, None)];
        // "7" is window id 7, even though nothing app_id-matches it.
        assert_eq!(find_window(&c, "7"), Some(1));
        // "3" is window id 3, beating the app_id containing "3"... (none here)
        assert_eq!(find_window(&c, "3"), Some(0));
    }

    #[test]
    fn numeric_query_without_id_match_falls_to_app_ids() {
        // No window has id 2048, but an app_id contains "2048".
        let c = [cand(1, Some("2048-game"))];
        assert_eq!(find_window(&c, "2048"), Some(0));
    }

    #[test]
    fn exact_beats_substring_and_first_best_wins() {
        let c = [
            cand(1, Some("cce-mail-helper")),
            cand(2, Some("cce-mail")),
            cand(3, Some("cce-mail")),
        ];
        // Exact match at position 1 outranks the earlier substring match;
        // the later equal-score exact match doesn't displace it.
        assert_eq!(find_window(&c, "CCE-Mail"), Some(1));
        // Pure substring query: first container wins.
        assert_eq!(find_window(&c, "mail"), Some(0));
    }

    #[test]
    fn no_match_is_none() {
        let c = [cand(1, Some("cce-files")), cand(2, None)];
        assert_eq!(find_window(&c, "firefox"), None);
        assert_eq!(find_window(&[], "anything"), None);
    }
}
