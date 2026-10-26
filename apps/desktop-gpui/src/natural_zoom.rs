//! Natural auto zoom (fork): zoom where the work is, not on every click.
//!
//! Clicks are work at the cursor's position, key presses at the last
//! click's (where the field is). A zoom goes in only when work stays inside
//! one region (at most 60% of each frame dimension, at most 36% of its area)
//! for at least 1.5 s, holds while it stays there, and goes out when the work
//! moves away or stops for 2 s. An isolated click, a click on window chrome and a
//! drag make no zoom. Zoom-ins are at least 6 s apart and zoomed time stays
//! under 35% of the recording; the longest stretches of work win the budget.
//!
//! Std only, all times in ms: the acceptance driver includes this file by path.

pub const DWELL_MS: f64 = 1500.;
pub const STOP_MS: f64 = 2000.;
pub const SPACING_MS: f64 = 6000.;
pub const BUDGET: f64 = 0.35;
/// What the editor's zoom spring takes to settle (`NATURAL_SPRING`).
pub const EASE_MS: f64 = 750.;
/// The zoom starts moving this long before the first piece of work.
const LEAD_MS: f64 = 300.;
/// The longest side of a region, as a share of the frame's: 1.4x shows 71%
/// of each side, and 0.6 x 0.6 is 36% of the frame, under the 40% bound.
const MAX_SIDE: f64 = 0.6;
/// A press that travels this far before release is a drag, not work.
const DRAG: f64 = 0.08;

/// The title bar at the top, the taskbar at the bottom.
pub fn is_chrome(y: f64) -> bool {
    !(0.04..=0.95).contains(&y)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Move {
    pub time_ms: f64,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Click {
    pub time_ms: f64,
    pub down: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zoom {
    pub start_ms: f64,
    pub end_ms: f64,
    pub amount: f64,
    pub x: f64,
    pub y: f64,
}

/// Tighter on a smaller region: 2.0x on a point, falling to 1.4x as the
/// region's longer side reaches [`MAX_SIDE`].
pub fn depth(side: f64) -> f64 {
    (2.0 - side).clamp(1.4, 2.0)
}

/// `quiet` spans (shots marked `zoom: none`, and zero-width cuts between
/// clips) take no zoom and no zoom crosses them.
pub fn natural_zoom(
    moves: &[Move],
    clicks: &[Click],
    keys: &[f64],
    duration_ms: f64,
    quiet: &[(f64, f64)],
) -> Vec<Zoom> {
    let mut moves: Vec<Move> = moves
        .iter()
        .filter(|m| m.time_ms.is_finite() && m.x.is_finite() && m.y.is_finite())
        .copied()
        .collect();
    moves.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
    if moves.is_empty() || !(duration_ms > 0.) {
        return Vec::new();
    }
    let at = |t: f64| moves[moves.partition_point(|m| m.time_ms <= t).saturating_sub(1)];
    let mut clicks: Vec<Click> = clicks
        .iter()
        .filter(|c| c.time_ms.is_finite())
        .copied()
        .collect();
    clicks.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
    let in_quiet = |t: f64| quiet.iter().any(|&(a, b)| a <= t && t <= b);

    let mut work: Vec<Move> = Vec::new();
    let mut pressed: Vec<Move> = Vec::new();
    for (index, click) in clicks.iter().enumerate().filter(|(_, c)| c.down) {
        let from = at(click.time_ms);
        let release = clicks[index + 1..]
            .iter()
            .find(|c| !c.down)
            .map_or(click.time_ms, |c| c.time_ms);
        let dragged = moves
            .iter()
            .filter(|m| m.time_ms > click.time_ms && m.time_ms <= release)
            .any(|m| (m.x - from.x).hypot(m.y - from.y) > DRAG);
        let press = Move {
            time_ms: click.time_ms,
            ..from
        };
        pressed.push(press);
        if !dragged {
            work.push(press);
        }
    }
    // Typing goes where the last click put the caret; before any click, at
    // the cursor.
    work.extend(keys.iter().filter(|t| t.is_finite()).map(|&t| {
        let place = pressed
            .partition_point(|p| p.time_ms <= t)
            .checked_sub(1)
            .map_or_else(|| at(t), |i| pressed[i]);
        Move {
            time_ms: t,
            ..place
        }
    }));
    // Off the frame (another display) is not work here either.
    work.retain(|w| (0.0..=1.0).contains(&w.x) && !is_chrome(w.y) && !in_quiet(w.time_ms));
    work.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));

    // Runs of work that stay in one region, each a candidate zoom.
    let mut candidates = Vec::new();
    let mut i = 0;
    while i < work.len() {
        let (mut x0, mut y0, mut x1, mut y1) = (work[i].x, work[i].y, work[i].x, work[i].y);
        let mut j = i;
        while let Some(next) = work.get(j + 1) {
            let previous = work[j].time_ms;
            let cut = quiet
                .iter()
                .any(|&(a, _)| previous < a && a <= next.time_ms);
            let (nx0, ny0) = (x0.min(next.x), y0.min(next.y));
            let (nx1, ny1) = (x1.max(next.x), y1.max(next.y));
            let fits = (nx1 - nx0).max(ny1 - ny0) <= MAX_SIDE;
            if next.time_ms - previous >= STOP_MS || cut || !fits {
                break;
            }
            (x0, y0, x1, y1) = (nx0, ny0, nx1, ny1);
            j += 1;
        }
        let (first, last) = (work[i].time_ms, work[j].time_ms);
        if last - first >= DWELL_MS {
            let quiet_after = quiet
                .iter()
                .map(|&(a, _)| a)
                .filter(|&a| a > last)
                .fold(f64::INFINITY, f64::min);
            let quiet_before = quiet
                .iter()
                .map(|&(_, b)| b)
                .filter(|&b| b < first)
                .fold(0., f64::max);
            let end = (last + STOP_MS)
                .min(work.get(j + 1).map_or(f64::INFINITY, |w| w.time_ms))
                .min(quiet_after)
                .min(duration_ms);
            candidates.push(Zoom {
                start_ms: (first - LEAD_MS).max(quiet_before),
                end_ms: end,
                amount: depth((x1 - x0).max(y1 - y0)),
                x: (x0 + x1) / 2.,
                y: (y0 + y1) / 2.,
            });
        }
        i = j + 1;
    }

    // The longest work first, while it keeps its distance and the budget.
    candidates.sort_by(|a, b| (b.end_ms - b.start_ms).total_cmp(&(a.end_ms - a.start_ms)));
    let mut budget = BUDGET * duration_ms;
    let mut kept: Vec<Zoom> = Vec::new();
    for mut zoom in candidates {
        let clear = kept.iter().all(|k| {
            (zoom.start_ms - k.start_ms).abs() >= SPACING_MS
                && (zoom.start_ms >= k.end_ms + EASE_MS || zoom.end_ms + EASE_MS <= k.start_ms)
        });
        if !clear || budget < DWELL_MS {
            continue;
        }
        zoom.end_ms = zoom.end_ms.min(zoom.start_ms + budget);
        budget -= zoom.end_ms - zoom.start_ms;
        kept.push(zoom);
    }
    kept.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
    kept
}

/// Cap's own click-per-zoom pass (`generate_zoom_segments_from_clicks_impl`,
/// `src-tauri/src/recording.rs`), kept for the editor's comparison and the
/// acceptance fixtures: every click (press or release) grows into a padded
/// interval (300 ms before, 2.5 s after), clicks in the last second are
/// ignored, intervals closer than 2.5 s merge, all clamped inside the
/// recording. `(start_ms, end_ms)`, rounded.
pub fn stock(clicks_ms: &[f64], duration_ms: f64) -> Vec<(f64, f64)> {
    const START_MIN_MS: f64 = 1.0;
    const CLICK_PRE_PADDING_MS: f64 = 300.0;
    const CLICK_POST_PADDING_MS: f64 = 2500.0;
    const CLICK_END_CLAMP_PADDING_MS: f64 = 800.0;
    const TRAILING_CLICK_IGNORE_MS: f64 = 1000.0;
    const MERGE_GAP_MS: f64 = 2500.0;

    let click_cutoff_ms = duration_ms - TRAILING_CLICK_IGNORE_MS;
    let end_limit_ms = duration_ms - CLICK_END_CLAMP_PADDING_MS;
    if !(duration_ms > 0.0) || click_cutoff_ms <= 0.0 || end_limit_ms <= START_MIN_MS {
        return Vec::new();
    }
    let mut intervals: Vec<(f64, f64)> = clicks_ms
        .iter()
        .map(|time| time.floor())
        .filter(|time| *time < click_cutoff_ms)
        .map(|time| {
            (
                (time - CLICK_PRE_PADDING_MS).max(START_MIN_MS),
                (time + CLICK_POST_PADDING_MS).min(end_limit_ms),
            )
        })
        .filter(|(start, end)| end > start)
        .collect();
    intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for interval in intervals {
        if let Some(last) = merged.last_mut()
            && interval.0 <= last.1 + MERGE_GAP_MS
        {
            last.1 = last.1.max(interval.1);
            continue;
        }
        merged.push(interval);
    }
    merged
        .into_iter()
        .map(|(start, end)| (start.round(), end.round()))
        .collect()
}

#[cfg(test)]
mod tests {
    // The spec's bounds as literals, so a changed constant goes red.
    use super::*;

    fn still(x: f64, y: f64) -> Vec<Move> {
        vec![Move { time_ms: 0., x, y }]
    }

    fn taps(times: impl IntoIterator<Item = f64>) -> Vec<Click> {
        times
            .into_iter()
            .flat_map(|t| {
                [
                    Click {
                        time_ms: t,
                        down: true,
                    },
                    Click {
                        time_ms: t + 80.,
                        down: false,
                    },
                ]
            })
            .collect()
    }

    /// Clicks every `step` ms from `from` to `to` at one spot.
    fn work(from: f64, to: f64, step: f64) -> Vec<Click> {
        taps(
            (0..)
                .map(|n| from + n as f64 * step)
                .take_while(|t| *t <= to),
        )
    }

    #[test]
    fn sustained_work_in_a_small_region_zooms_and_holds() {
        let zooms = natural_zoom(
            &still(0.4, 0.5),
            &work(10_000., 14_000., 500.),
            &[],
            60_000.,
            &[],
        );
        assert_eq!(zooms.len(), 1);
        let zoom = zooms[0];
        assert_eq!(zoom.start_ms, 9_700.);
        // Held through the last click, out 2 s after the work stops.
        assert_eq!(zoom.end_ms, 16_000.);
        assert_eq!(zoom.amount, 2.0);
        assert_eq!((zoom.x, zoom.y), (0.4, 0.5));
    }

    #[test]
    fn typing_counts_as_work_where_the_last_click_was() {
        let keys: Vec<f64> = (0..20).map(|n| 5_000. + n as f64 * 100.).collect();
        let zooms = natural_zoom(&still(0.3, 0.3), &[], &keys, 30_000., &[]);
        assert_eq!(zooms.len(), 1);
        assert_eq!(zooms[0].end_ms, 5_000. + 1_900. + 2_000.);
        // Clicked into a field, then the mouse wandered off while typing.
        let mut moves = still(0.3, 0.3);
        moves.push(Move {
            time_ms: 5_500.,
            x: 0.9,
            y: 0.9,
        });
        let zooms = natural_zoom(&moves, &taps([4_900.]), &keys, 30_000., &[]);
        assert_eq!(zooms.len(), 1);
        assert_eq!((zooms[0].x, zooms[0].y), (0.3, 0.3));
        // Typing after a click on the title bar is not work either.
        let zooms = natural_zoom(&still(0.3, 0.02), &taps([4_900.]), &keys, 30_000., &[]);
        assert!(zooms.is_empty());
    }

    #[test]
    fn short_or_isolated_or_chrome_or_dragged_work_makes_no_zoom() {
        let moves = still(0.5, 0.5);
        // Isolated click, and work that stops before 1.5 s.
        assert!(natural_zoom(&moves, &taps([5_000.]), &[], 30_000., &[]).is_empty());
        assert!(natural_zoom(&moves, &work(5_000., 6_400., 200.), &[], 30_000., &[]).is_empty());
        // A gap of 2 s or more breaks the run.
        assert!(natural_zoom(&moves, &taps([5_000., 7_000.]), &[], 30_000., &[]).is_empty());
        // Window chrome: the title bar (top 4%) and the taskbar (bottom 5%).
        for y in [0.03, 0.96] {
            let chrome = still(0.5, y);
            assert!(
                natural_zoom(&chrome, &work(5_000., 9_000., 300.), &[], 30_000., &[]).is_empty()
            );
        }
        for y in [0.05, 0.94] {
            let page = still(0.5, y);
            assert_eq!(
                natural_zoom(&page, &work(5_000., 9_000., 300.), &[], 30_000., &[]).len(),
                1
            );
        }
        // Drags: down, travel, up.
        let mut moves = Vec::new();
        let mut clicks = Vec::new();
        for n in 0..8 {
            let t = 5_000. + n as f64 * 600.;
            moves.push(Move {
                time_ms: t,
                x: 0.1,
                y: 0.5,
            });
            moves.push(Move {
                time_ms: t + 200.,
                x: 0.8,
                y: 0.5,
            });
            clicks.push(Click {
                time_ms: t,
                down: true,
            });
            clicks.push(Click {
                time_ms: t + 300.,
                down: false,
            });
        }
        assert!(natural_zoom(&moves, &clicks, &[], 30_000., &[]).is_empty());
    }

    #[test]
    fn work_spread_wider_than_the_region_makes_no_zoom() {
        let moves: Vec<Move> = (0..10)
            .map(|n| Move {
                time_ms: 5_000. + n as f64 * 400.,
                x: if n % 2 == 0 { 0.1 } else { 0.9 },
                y: if n % 2 == 0 { 0.2 } else { 0.8 },
            })
            .collect();
        let clicks = work(5_000., 8_600., 400.);
        assert!(natural_zoom(&moves, &clicks, &[], 30_000., &[]).is_empty());
        // A box over 40% of the frame, one under it, and strips too long
        // and short enough for the shallowest zoom.
        let spread = |w: f64, h: f64| -> Vec<Move> {
            (0..10)
                .map(|n| Move {
                    time_ms: 5_000. + n as f64 * 400. - 10.,
                    x: 0.2 + w * (n % 2) as f64,
                    y: 0.2 + h * (n / 2 % 2) as f64,
                })
                .collect()
        };
        for (w, h, zooms) in [
            (0.65, 0.65, 0),
            (0.55, 0.55, 1),
            (0.62, 0.05, 0),
            (0.05, 0.62, 0),
            (0.58, 0.05, 1),
        ] {
            let found = natural_zoom(&spread(w, h), &clicks, &[], 30_000., &[]).len();
            assert_eq!(found, zooms, "{w} x {h}");
        }
    }

    #[test]
    fn work_moving_away_zooms_out_there() {
        let mut moves = still(0.3, 0.3);
        moves.push(Move {
            time_ms: 9_900.,
            x: 0.95,
            y: 0.95,
        });
        let zooms = natural_zoom(&moves, &work(5_000., 10_500., 500.), &[], 60_000., &[]);
        assert_eq!(zooms.len(), 1);
        assert_eq!(zooms[0].end_ms, 10_000.);
    }

    #[test]
    fn depth_is_1_4_to_2_0_and_tighter_on_a_smaller_region() {
        let sides = [0., 0.04, 0.1, 0.3, 0.5, 0.59, 0.6];
        for pair in sides.windows(2) {
            assert!(depth(pair[0]) > depth(pair[1]), "{pair:?}");
        }
        for side in sides {
            assert!((1.4..=2.0).contains(&depth(side)), "{side}");
        }
    }

    #[test]
    fn zoom_ins_keep_their_distance_and_the_budget() {
        // Work every 5 s: only one in each 6 s can zoom.
        let mut clicks = Vec::new();
        for n in 0..10 {
            let from = 1_000. + n as f64 * 5_000.;
            clicks.extend(work(from, from + 1_600., 400.));
        }
        let zooms = natural_zoom(&still(0.5, 0.5), &clicks, &[], 50_000., &[]);
        assert!(zooms.len() > 1);
        for pair in zooms.windows(2) {
            assert!(pair[1].start_ms - pair[0].start_ms >= 6_000.);
        }
        let zoomed: f64 = zooms.iter().map(|z| z.end_ms - z.start_ms).sum();
        assert!(zoomed <= 0.35 * 50_000. + 1e-6);
        // One long stretch of work is cut to the budget.
        let long = natural_zoom(
            &still(0.5, 0.5),
            &work(1_000., 30_000., 500.),
            &[],
            40_000.,
            &[],
        );
        assert_eq!(long.len(), 1);
        assert!((long[0].end_ms - long[0].start_ms - 0.35 * 40_000.).abs() < 1e-6);
    }

    #[test]
    fn quiet_spans_take_no_zoom_and_cut_runs() {
        let clicks = work(5_000., 12_000., 500.);
        assert!(natural_zoom(&still(0.5, 0.5), &clicks, &[], 30_000., &[(0., 30_000.)]).is_empty());
        // A cut between clips at 8 s splits the work: the longer side after
        // the cut zooms from the cut, the side before is too close to.
        let zooms = natural_zoom(&still(0.5, 0.5), &clicks, &[], 30_000., &[(8_000., 8_000.)]);
        assert_eq!(zooms.len(), 1);
        assert_eq!((zooms[0].start_ms, zooms[0].end_ms), (8_200., 14_000.));
        let before = natural_zoom(
            &still(0.5, 0.5),
            &clicks,
            &[],
            30_000.,
            &[(11_000., 11_000.)],
        );
        assert_eq!(before.len(), 1);
        assert_eq!((before[0].start_ms, before[0].end_ms), (4_700., 11_000.));
    }
}
