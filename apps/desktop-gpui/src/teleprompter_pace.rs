//! The shot list's pace: how far a shot's lines have scrolled, from how long
//! they take to read. A shot that fits the view never moves; a longer one
//! scrolls only by what overflows. No gpui here, so the rule tests on its own.

/// Seconds to read `words` at `words_per_minute`, floored at 1s the way
/// `calculatePlaybackSpeed` is, so an empty shot still reaches its STOP cue.
pub fn reading_seconds(words: usize, words_per_minute: u32) -> f32 {
    (words.max(1) as f32 / words_per_minute.max(1) as f32 * 60.).max(1.)
}

/// How far through its reading time a shot is after `elapsed` more seconds,
/// 0 to 1. Kept as a fraction, not seconds, so a pace change mid-shot changes
/// the speed from there on instead of jumping the text.
pub fn advance_progress(
    progress: f32,
    elapsed_seconds: f32,
    words: usize,
    words_per_minute: u32,
) -> f32 {
    (progress.max(0.) + elapsed_seconds.max(0.) / reading_seconds(words, words_per_minute)).min(1.)
}

/// Scroll offset in pixels from the top: the overflow, spread evenly across
/// the reading time, so the line being read drifts down the view and never
/// leaves it.
pub fn scroll_offset(progress: f32, text_height: f32, view_height: f32) -> f32 {
    (text_height - view_height).max(0.) * progress.clamp(0., 1.)
}

/// The progress to resume from once playback restarts at `offset`: a reader
/// who scrolled by hand while paused carries on from there. A shot that fits
/// cannot scroll, so it keeps its reading-time progress.
pub fn progress_at_offset(offset: f32, text_height: f32, view_height: f32, progress: f32) -> f32 {
    let overflow = text_height - view_height;
    if overflow > 0. {
        (offset / overflow).clamp(0., 1.)
    } else {
        progress
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a shot for `seconds` in 16ms ticks, the window's playback tick.
    fn run(seconds: f32, words: usize, words_per_minute: u32) -> f32 {
        let mut progress = 0.;
        for _ in 0..(seconds / 0.016).round() as usize {
            progress = advance_progress(progress, 0.016, words, words_per_minute);
        }
        progress
    }

    #[test]
    fn text_that_fits_never_moves() {
        for seconds in [0., 1., 5., 11.9, 12., 60.] {
            let progress = run(seconds, 26, 130);
            assert_eq!(scroll_offset(progress, 150., 210.), 0.);
            assert_eq!(scroll_offset(progress, 210., 210.), 0.);
        }
        // It still ends, at the reading pace: 26 words at 130wpm is 12s.
        assert!(run(11.5, 26, 130) < 1.);
        assert_eq!(run(12.5, 26, 130), 1.);
    }

    #[test]
    fn long_text_scrolls_only_by_the_overflow_at_the_pace() {
        // 600px of lines in a 200px view: 400px to scroll over 12s.
        let at = |seconds| scroll_offset(run(seconds, 26, 130), 600., 200.);
        assert_eq!(at(0.), 0.);
        assert!((at(6.) - 200.).abs() < 1.);
        assert!((at(12.) - 400.).abs() < 1.);
        assert_eq!(at(30.), 400.);
        // Half the pace, half as far in the same time.
        assert!((scroll_offset(run(6., 26, 65), 600., 200.) - 100.).abs() < 1.);
    }

    #[test]
    fn a_pace_change_mid_shot_keeps_the_text_where_it_is() {
        let progress = run(6., 26, 130);
        let before = scroll_offset(progress, 600., 200.);
        let after = scroll_offset(advance_progress(progress, 0., 26, 65), 600., 200.);
        assert_eq!(before, after);
    }

    #[test]
    fn a_hand_scroll_while_paused_is_where_playback_resumes() {
        // 1000px of overflow, a quarter read, scrolled by hand to 700px.
        let progress = progress_at_offset(700., 1200., 200., 0.25);
        let next = scroll_offset(advance_progress(progress, 0.016, 26, 130), 1200., 200.);
        assert!((next - 700.).abs() < 2., "resumed at {next}");
        // A shot that fits keeps its reading time.
        assert_eq!(progress_at_offset(0., 150., 210., 0.25), 0.25);
    }

    #[test]
    fn an_empty_shot_still_ends() {
        assert_eq!(reading_seconds(0, 130), 1.);
        assert_eq!(advance_progress(0., 1., 0, 130), 1.);
    }
}
