# Contracts changelog

Semver: a major bump breaks readers (ingest refuses it, D19); a minor bump adds optional fields.

## 0.1.0 (2026-10-26)

First release: shotlist, shots, windows (one `windows.jsonl` line), stop, take-manifest, events,
story, edit, cut and gate. Times are integer milliseconds on the take's start-gate clock. Ids are
names (`^[a-z][a-z0-9_-]*$`), never numbers, and `edit` (`{"shots": [{shot_id, take_id, sentence_ids, zoom_event_ids}]}`) is closed, so a model cannot write a time
into it (D8).

`cut` requires what build.py renders: `size` is `[width, height]`, a card needs `duration`, a split
needs `duration` and a `left` and `right` that each name a `src`, a clip needs `src`, `segments`
(`[start, end]` pairs) and a `panel` with 4-number `rect` and `crop`, and `music` needs a `src`. `cap-events` is the raw
`cap events <take> --json` output (move, click and key, Cap's f64 `time_ms`). It is kept apart from
`events`, which is ingest's semantic output (click, key burst, window and shot, each with a crop).
