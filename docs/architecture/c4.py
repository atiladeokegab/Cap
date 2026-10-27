# /// script
# requires-python = ">=3.11"
# dependencies = ["diagrams"]
# ///
"""C4 diagrams of this fork, levels 1 to 4, and the README section that shows them.

    uv run docs/architecture/c4.py            # writes c4_context.png ... c4_code_tools.png beside this file
    uv run docs/architecture/c4.py context    # just one level

Level 4 is read from the code on every run: each fork file that exists is drawn with its public
functions (a workflow with its steps), and one not yet in the tree is drawn as "not built yet". A
level 4 arrow is drawn only when the call it names is in the source file it starts from. Work still
in an open pull request is tagged "(PR #N)" at every level until the file or the text that marks it
is in the tree, so the tag drops by itself once the PR merges. Level 4 is two diagrams, the desktop
app and then the mirror crate, CLI and CI, each showing calls into the other as one grey box. The README
section between the fork markers is rewritten too, so its Known limitations list always matches the
`// shortcut:` comments in the fork's code. This repo is public: no bucket, key, host or address
appears here.
"""

import re
import shutil
import sys
from functools import partial
from pathlib import Path

from diagrams import Cluster, Diagram
from diagrams.c4 import C4Node, Container, Database, Person, Relationship, System, SystemBoundary

# diagrams.c4 wraps text at 40 characters but draws 2.6in boxes, which cut lines off; 3.3in fits.
Container, Database, Person, System = (partial(f, width="3.3") for f in (Container, Database, Person, System))
Component = partial(C4Node, type="Component", width="3.3")
File = partial(C4Node, type="File", width="4.6")

OUT = Path(__file__).parent
ROOT = OUT.parents[1]
SYSTEM = "Cap fork for demo films"
GRAPH = {"splines": "spline", "nodesep": "0.8", "ranksep": "1.1"}
# A `// shortcut:` comment and the `//` lines that continue it.
SHORTCUT = re.compile(r"//+[ \t]*shortcut:[ \t]*([^\n]*(?:\n[ \t]*//+(?![ \t]*shortcut:)[^\n]*)*)")
SHORTCUT_DIRS = ("apps/desktop-gpui/src", "apps/desktop-gpui/gcs-mirror/src", "apps/cli/src", "crates/verify-stimulus/src")
START, END = "<!-- fork:start (written by docs/architecture/c4.py) -->", "<!-- fork:end -->"
BUCKET = "Object storage"
# Open pull requests the diagrams already show: R-8 stop and go cues and natural zoom, R-9 pick-ups,
# R-10 the CLI recording headless for the agent recorder.
R8, R9, R10 = "PR #22", "PR #26", "PR #25"
PRS = {R8: "stop and go cues, natural auto zoom", R9: "pick-ups from the republished shot list",
       R10: "headless `cap record --detach` for the agent recorder"}


def built(path, marker=None):
    """Whether `path` is in this tree and, when a marker is given, contains it."""
    file = ROOT / path
    return file.is_file() and (marker is None or marker in file.read_text())


def pending(pr, path, marker=None):
    """' (PR #N)' until that PR's file, or its marker text, is in the tree."""
    return "" if built(path, marker) else f" ({pr})"

# Level 4: the code the fork adds or changes, grouped by where it runs, as (path, what it does,
# then any open PR's change to it as (pr, text that marks it in the file, what it adds)).
APP, CRATE, CLI, CI, UPSTREAM = (
    "Desktop app (apps/desktop-gpui/src)",
    "cap-gcs-mirror crate (apps/desktop-gpui/gcs-mirror)",
    "cap CLI (apps/cli)",
    "CI fixture and release",
    "Upstream, reused",
)
CODE = {
    APP: [
        ("apps/desktop-gpui/src/recording.rs", "Studio start: starts the mirror and the window log; finalize: mirror stop before the remux"),
        ("apps/desktop-gpui/src/session.rs", "Recording phases; shot_time(): take dir and (segment, ms) on the start-gate clock"),
        ("apps/desktop-gpui/src/teleprompter_window.rs", "Shows the shot on screen; load_shot_list GETs it from storage; mark_shot",
         (R8, "Cue::", "STOP cue, next shot's card, Go with a 3-2-1 countdown"),
         (R9, "fn reload_shot_list", "re-reads the shot list at each take start; pick-ups and what's left")),
        ("apps/desktop-gpui/src/shots.rs", "pick the stored or local list, parse (shotlist rules), ShotClock, mark -> shots.json",
         (R8, "pub enum Cue", "Cue, countdown, zoomless (shots marked zoom: none)"),
         (R9, "pub struct Reads", "Reads (latest read wins), Progress")),
        ("apps/desktop-gpui/src/hotkeys.rs", "Ctrl+Alt+N / Ctrl+Alt+R while a shot list shows -> Next shot / Retake"),
        ("apps/desktop-gpui/src/window_log.rs", "Windows: foreground window every 250 ms -> windows.jsonl, on change only"),
        ("apps/desktop-gpui/src/window_log/change.rs", "What counts as a change; time_ms from the segment's start-gate epoch"),
        ("apps/desktop-gpui/src/gcs_mirror.rs", "Reads mirror.json from app data; start, stop, resume_pending"),
        ("apps/desktop-gpui/src/theme.rs", "Light palette and type tokens for the restyle"),
        ("apps/desktop-gpui/src/editor_window.rs", "The editor; its generate button builds the zoom track",
         (R8, "generate_natural_zoom_for_timeline", "natural zoom over the take's cursor and keys, none in zoom: none shots")),
        ("apps/desktop-gpui/src/editor_edits.rs", "Timeline edits, zoom segments and springs",
         (R8, "generate_natural_zoom_for_timeline", "natural zoom track, NATURAL_ZOOM_SPRING (0.75 s)")),
        ("apps/desktop-gpui/src/natural_zoom.rs", "Zoom only where work dwells 1.5 s in under 40% of the frame; 6 s apart, under 35% zoomed",
         (R8, None, "new file")),
    ],
    CRATE: [
        ("apps/desktop-gpui/gcs-mirror/src/lib.rs", "Mirror: outbox hard links, scan, freeze -> stop.json, drain, manifest last"),
        ("apps/desktop-gpui/gcs-mirror/src/sigv4.rs", "SigV4 signing for the XML API with an HMAC key"),
        ("apps/desktop-gpui/gcs-mirror/src/bin/cap-driver.rs", "Test adapter, JSON on stdin: sign, scan, stop, resume, shots, windows",
         (R8, "fn zoom_op", "prompter and zoom ops")),
    ],
    CLI: [
        ("apps/cli/src/events.rs", "cap events <take> --json: cursor.json and keyboard.bin as one sorted list"),
        ("apps/cli/src/record.rs", "cap record: studio takes from the command line, foreground or --detach",
         (R10, "remux: bool", "a --detach take keeps Cap's fragments for uploading whole")),
    ],
    CI: [
        (".github/workflows/verify-recording.yml", "windows-2022: a real studio take of the stimulus with SendInput clicks"),
        ("crates/verify-stimulus/src/main.rs", "The stimulus window; each click flips a corner patch, logged to .clicks.jsonl"),
        ("crates/verify-stimulus/src/bin/window_log_probe.rs", "window_log.rs compiled in unchanged, run beside the fixture take"),
        (".github/workflows/fork-windows-release.yml", "Unsigned Windows NSIS installer; no updater artefacts"),
    ],
    UPSTREAM: [
        ("crates/recording/src/output_pipeline/start_gate.rs", "The start-gate epoch every take time is measured from"),
    ],
}
# Level 4 arrows as (from, the text that must be in `from`, to, label, and the open PR that adds it).
CALLS = [
    ("apps/desktop-gpui/src/recording.rs", "gcs_mirror::start", "apps/desktop-gpui/src/gcs_mirror.rs", "start; stop before remux"),
    ("apps/desktop-gpui/src/recording.rs", "window_log::start", "apps/desktop-gpui/src/window_log.rs", "start, stop"),
    ("apps/desktop-gpui/src/window_log.rs", "mod change", "apps/desktop-gpui/src/window_log/change.rs", "detects changes, times lines"),
    ("apps/desktop-gpui/src/session.rs", "window_log::set_paused", "apps/desktop-gpui/src/window_log.rs", "set_paused on pause and resume"),
    ("apps/desktop-gpui/src/hotkeys.rs", "view.mark_shot", "apps/desktop-gpui/src/teleprompter_window.rs", "mark_shot"),
    ("apps/desktop-gpui/src/teleprompter_window.rs", "shot_time()", "apps/desktop-gpui/src/session.rs", "shot_time"),
    ("apps/desktop-gpui/src/teleprompter_window.rs", "shots::pick", "apps/desktop-gpui/src/shots.rs", "pick, mark"),
    ("apps/desktop-gpui/src/teleprompter_window.rs", "cap_gcs_mirror::request", "apps/desktop-gpui/gcs-mirror/src/lib.rs", "GET the shot list"),
    ("apps/desktop-gpui/src/session.rs", "armed_at", "crates/recording/src/output_pipeline/start_gate.rs", "armed_at: the shot clock's epoch"),
    ("apps/desktop-gpui/src/recording.rs", "armed_at", "crates/recording/src/output_pipeline/start_gate.rs", "armed_at: the window log's epoch"),
    ("apps/desktop-gpui/src/gcs_mirror.rs", "Mirror::new", "apps/desktop-gpui/gcs-mirror/src/lib.rs", "Mirror, spawn, stop, drain"),
    ("apps/desktop-gpui/gcs-mirror/src/lib.rs", "sigv4::authorize", "apps/desktop-gpui/gcs-mirror/src/sigv4.rs", "authorize"),
    ("apps/desktop-gpui/gcs-mirror/src/bin/cap-driver.rs", "Mirror::new", "apps/desktop-gpui/gcs-mirror/src/lib.rs", "scan, stop, resume"),
    ("apps/desktop-gpui/gcs-mirror/src/bin/cap-driver.rs", "src/shots.rs", "apps/desktop-gpui/src/shots.rs", "compiled in by #[path]"),
    ("apps/desktop-gpui/gcs-mirror/src/bin/cap-driver.rs", "src/window_log/change.rs", "apps/desktop-gpui/src/window_log/change.rs", "compiled in by #[path]"),
    ("crates/verify-stimulus/src/bin/window_log_probe.rs", "src/window_log.rs", "apps/desktop-gpui/src/window_log.rs", "compiled in by #[path]"),
    (".github/workflows/verify-recording.yml", "cap-verify-stimulus", "crates/verify-stimulus/src/main.rs", "records the stimulus"),
    (".github/workflows/verify-recording.yml", "window-log-probe", "crates/verify-stimulus/src/bin/window_log_probe.rs", "runs"),
    (".github/workflows/verify-recording.yml", "op: \"shots\"", "apps/desktop-gpui/gcs-mirror/src/bin/cap-driver.rs", "shots, scan, stop"),
    (".github/workflows/verify-recording.yml", "events \"$take\" --json", "apps/cli/src/events.rs", "cap events --json"),
    ("apps/desktop-gpui/src/editor_window.rs", "generate_natural_zoom_for_timeline", "apps/desktop-gpui/src/editor_edits.rs", "generate the zoom track", R8),
    ("apps/desktop-gpui/src/editor_window.rs", "shots::zoomless", "apps/desktop-gpui/src/shots.rs", "zoomless", R8),
    ("apps/desktop-gpui/src/editor_edits.rs", "natural_zoom(", "apps/desktop-gpui/src/natural_zoom.rs", "natural_zoom, stock", R8),
    ("apps/desktop-gpui/gcs-mirror/src/bin/cap-driver.rs", "src/natural_zoom.rs", "apps/desktop-gpui/src/natural_zoom.rs", "compiled in by #[path]", R8),
]


def draw(level, title, direction="TB"):
    return Diagram(f"{SYSTEM} - {title}", filename=str(OUT / f"c4_{level}"), outformat="png",
                   show=False, direction=direction, graph_attr=GRAPH)


def context():
    """Level 1: who uses the fork and what it talks to."""
    with draw("context", "System Context"):
        lead = Person("Presenter", "Records a product demo on a Windows laptop")
        fork = System(SYSTEM, "Cap's desktop recorder plus a shot list, a window log, live upload and cap events"
                      + "; natural zoom" + pending(R8, "apps/desktop-gpui/src/natural_zoom.rs"))
        store = System(BUCKET, "An S3-compatible bucket, HMAC key from local config", external=True)
        pipeline = System("Film pipeline", "Turns an uploaded take into an edited demo film", external=True)
        ci = System("GitHub Actions", "Builds the unsigned Windows installer; records the Windows fixture take", external=True)
        lead >> Relationship("records; Next shot, Retake; Go" + pending(R8, "apps/desktop-gpui/src/shots.rs", "pub enum Cue")) >> fork
        fork >> Relationship("reads the shot list") >> store
        fork >> Relationship("uploads the take while recording, manifest last") >> store
        pipeline >> Relationship("publishes the shot list; pulls takes/") >> store
        pipeline >> Relationship("cap events --json") >> fork
        pipeline >> Relationship("records headless: cap record --detach" + pending(R10, "apps/cli/src/record.rs", "remux: bool")) >> fork
        ci >> Relationship("installer; fixture take") >> fork
        ci >> Relationship("fixture take under ci/, if its secret is set") >> store


def container():
    """Level 2: the separately running pieces."""
    with draw("container", "Containers"):
        lead = Person("Presenter", "")
        with SystemBoundary(SYSTEM):
            app = Container("Desktop app", "Rust, GPUI (apps/desktop-gpui)", "Studio recording, teleprompter shot list, window log, upload mirror;"
                            " editor natural zoom" + pending(R8, "apps/desktop-gpui/src/natural_zoom.rs"))
            cli = Container("cap CLI", "Rust (apps/cli), same commit", "cap events <take> --json; cap record --detach keeps the fragments"
                            + pending(R10, "apps/cli/src/record.rs", "remux: bool"))
            driver = Container("cap-driver", "Rust (gcs-mirror crate, test binary)", "The mirror, shot and window-change logic, one JSON request per run;"
                               " prompter and zoom ops" + pending(R8, "apps/desktop-gpui/gcs-mirror/src/bin/cap-driver.rs", "fn zoom_op"))
            fixture = Container("Recording fixture", "GitHub Actions, windows-2022", "Studio take of the stimulus, SendInput clicks, artefact")
            release = Container("Release workflow", "GitHub Actions, windows-2022", "Unsigned installer, no updater artefacts")
            vendor = Database("contracts/vendor/0.1.0", "JSON Schema from the film repo", "shotlist, shots, windows, cap-events, manifest")
        store = Database(BUCKET, "S3-compatible", "shotlist/current.json; takes/<take>/ with the manifest last; ci/")
        take = Database("Take folder", "<take>.cap on the laptop", "Cap's files, plus shots.json, windows.jsonl, outbox/")
        recorder = System("Agent recorder", "Films click-through shots headless, under Xvfb", external=True)
        lead >> Relationship("record; Next shot, Retake") >> app
        recorder >> Relationship("cap record --detach" + pending(R10, "apps/cli/src/record.rs", "remux: bool")) >> cli
        cli >> Relationship("writes") >> take
        app >> Relationship("writes") >> take
        app >> Relationship("GET the shot list at each take start"
                            + pending(R9, "apps/desktop-gpui/src/teleprompter_window.rs", "fn reload_shot_list")) >> store
        app >> Relationship("PUT the take, manifest last") >> store
        app >> Relationship("parses the shot list by the rules of") >> vendor
        cli >> Relationship("reads cursor.json, keyboard.bin") >> take
        fixture >> Relationship("shots, scan, stop") >> driver
        fixture >> Relationship("cap events --json") >> cli
        driver >> Relationship("PUT under ci/<run>-<attempt>/") >> store
        release >> Relationship("builds") >> app


def component():
    """Level 3: inside the desktop app, only the parts the fork adds or reuses."""
    with draw("component", "Components of the desktop app"):
        with SystemBoundary("Desktop app (apps/desktop-gpui)"):
            rec = Component("Recording", "recording.rs, session.rs", "Studio start and stop; pause and resume; shot_time()")
            gate = Component("Start gate", "crates/recording, reused", "The armed instant cursor.json, shots and windows count from")
            keys = Component("Shot hotkeys", "hotkeys.rs", "Ctrl+Alt+N, Ctrl+Alt+R while a shot list shows")
            tele = Component("Teleprompter", "teleprompter_window.rs", "Shows the shot on screen; Next shot, Retake")
            cues = Component("Stop and go cues" + pending(R8, "apps/desktop-gpui/src/shots.rs", "pub enum Cue"), "shots.rs, teleprompter_window.rs",
                             "STOP at a shot's end, the next shot's card, Go with 3-2-1; resumes as a new segment")
            pickups = Component("Pick-ups" + pending(R9, "apps/desktop-gpui/src/shots.rs", "pub struct Reads"), "shots.rs, teleprompter_window.rs",
                                "Re-reads the republished shot list at each take start; shows what's left")
            shots = Component("Shots", "shots.rs", "Stored list or local file; parse; marks -> shots.json")
            editor = Component("Editor zoom track", "editor_window.rs, editor_edits.rs", "Generate button: zoom segments from cursor.json and keys")
            zoom = Component("Natural zoom" + pending(R8, "apps/desktop-gpui/src/natural_zoom.rs"), "natural_zoom.rs",
                             "In where work dwells 1.5 s in a small region, out after 2 s idle; 6 s apart, under 35% zoomed")
            win = Component("Window log", "window_log.rs", "Windows only: foreground title and process on change")
            wiring = Component("Mirror wiring", "gcs_mirror.rs", "mirror.json from app data; start, stop, resume_pending")
            mirror = Component("Mirror", "gcs-mirror crate: lib.rs, sigv4.rs", "outbox/ hard links, uploaded.jsonl, stop.json, manifest last")
        store = Database(BUCKET, "S3-compatible", "")
        take = Database("Take folder", "<take>.cap", "")
        keys >> Relationship("mark_shot") >> tele
        tele >> Relationship("pick, mark") >> shots
        tele >> Relationship("shot_time") >> rec
        tele >> Relationship("signed GET") >> mirror
        tele >> Relationship("shows") >> cues
        tele >> Relationship("shows") >> pickups
        cues >> Relationship("Go starts a new segment") >> rec
        pickups >> Relationship("signed GET at take start") >> mirror
        editor >> Relationship("moves, clicks, keys; zoom: none shots") >> zoom
        editor >> Relationship("reads cursor.json, shots.json") >> take
        rec >> Relationship("armed instant") >> gate
        rec >> Relationship("start, pause, stop") >> win
        rec >> Relationship("start; stop before remux") >> wiring
        wiring >> Relationship("spawn, stop, drain") >> mirror
        shots >> Relationship("shots.json") >> take
        win >> Relationship("windows.jsonl") >> take
        mirror >> Relationship("hard-links fragments into outbox/") >> take
        mirror >> Relationship("GET shot list; PUT take, manifest last") >> store


def outline(path, text):
    """What a file defines: a workflow's step names, else its public functions."""
    if path.endswith(".yml"):
        return "steps: " + ", ".join(re.findall(r"^\s*- name:\s*(.+)$", text, re.M))
    names = list(dict.fromkeys(re.findall(r"pub(?:\([a-z]+\))?\s+fn\s+(\w+)", text)))  # source order
    return "fns: " + ", ".join(names[:6]) + (", ..." if len(names) > 6 else "") if names else ""


def code(level, title, groups, other):
    """Level 4: the fork's files in `groups` as they are in this tree, with what they define."""
    files = {path: entry for group in groups for path, *entry in CODE[group]}
    sources = {path: (ROOT / path).read_text() if (ROOT / path).is_file() else None
               for entries in CODE.values() for path, *_ in entries}
    # left to right: one column per call depth keeps the files narrow enough for a README
    with draw(level, title, "LR"):
        nodes = {}
        with SystemBoundary("Fork code"):
            for group in groups:
                with Cluster(group):
                    for path, *_ in CODE[group]:
                        what, *changes = files[path]
                        # an open PR's change leads, as the label keeps only three lines
                        waiting = [(pr, note) for pr, marker, note in changes if not built(path, marker)]
                        detail = "".join(f"{pr}: {note}. " for pr, note in waiting) + what
                        detail += ". " + outline(path, sources[path]) if sources[path] else ""
                        tag = " (" + ", ".join(pr for pr, _ in waiting) + ")" if waiting else ""
                        name = Path(path).name + (tag if sources[path] or waiting else " (not built yet)")
                        nodes[path] = File(name, str(Path(path).parent), detail)
        # calls into the other code diagram share one grey box, one arrow per (file, label)
        across = {}
        for start, call, end, label, *pr in CALLS:
            found = call in (sources[start] or "") and sources[end]
            if start not in files or not (found or pr):
                continue
            tag = "" if found else f" ({pr[0]})"
            if end in nodes:
                nodes[start] >> Relationship(label + tag) >> nodes[end]
            else:
                across.setdefault((start, label), []).append(Path(end).name + tag)
        if across:
            elsewhere = File(other, "", f"shown in the {other} diagram", fillcolor="gray55")
            for (start, label), names in across.items():
                nodes[start] >> Relationship(f"{label}: {', '.join(names)}") >> elsewhere


LEVELS = {
    "context": context, "container": container, "component": component,
    "code": partial(code, "code", "Code: the desktop app", (APP, UPSTREAM), "mirror-crate code"),
    "code_tools": partial(code, "code_tools", "Code: mirror crate, CLI and CI", (CRATE, CLI, CI), "desktop-app code"),
}


def shortcuts():
    """Every `// shortcut:` comment in the fork's code, as (file, text, the fn it sits on or None)."""
    found = []
    for folder in SHORTCUT_DIRS:
        for path in sorted((ROOT / folder).rglob("*.rs")):
            text = path.read_text()
            for match in SHORTCUT.finditer(text):
                words = " ".join(re.sub(r"^\s*//+\s?", "", line).strip() for line in match.group(1).split("\n"))
                after = text[match.end():].lstrip("\n").split("\n")[:3]
                item = next((m.group(1) for line in after if (m := re.search(r"\bfn\s+(\w+)", line))), None)
                found.append((path.relative_to(ROOT).as_posix(), words.strip(), item))
    return found


def readme():
    """Rewrite the fork section of README.md, placed below upstream's header on the first run."""
    limits = "\n".join(f"- `{item or Path(path).name}` (`{path}`): {text}" for path, text, item in shortcuts())
    # each links to itself, as the code levels are only readable full size
    images = "\n\n".join(f"[![C4 {level}](docs/architecture/c4_{level}.png)](docs/architecture/c4_{level}.png)" for level in LEVELS)
    waiting = [f"{pr} ({what})" for pr, what in PRS.items() if any(
        pr == change[0] and not built(path, change[1]) for files in CODE.values() for path, _, *rest in files for change in rest)]
    open_prs = " A box or arrow tagged (PR #N) is in that open pull request, not yet on main: " + "; ".join(waiting) + "." if waiting else ""
    section = f"""{START}
## This fork

A fork of Cap for recording product demos that a film pipeline turns into edited films. The desktop
app adds a shot list in the teleprompter, read from object storage (or a local file), with Next shot
on Ctrl+Alt+N and Retake on Ctrl+Alt+R (Control+Option on macOS) writing shot marks to the take's
`shots.json`. Those two global chords are registered only while the teleprompter shows a shot list,
and a chord you have already bound to something else stays yours. It also adds a foreground-window
log on Windows (`windows.jsonl`), an upload mirror that streams each studio take to S3-compatible
storage while it records, with `take-manifest.json` last, and a light restyle. The CLI adds
`cap events <take> --json`. GitHub Actions builds an unsigned Windows installer and records a real
Windows fixture take. Upload settings live in a local `mirror.json` in the app data folder, never in
this repo.

Clocks: `shots.json` and `windows.jsonl` write `time_ms` from the segment's start-gate epoch (the
gate's armed instant for segment 0, each resume for later segments), the clock `cursor.json` uses;
`cap events` reports cursor and key events on that same clock. `stop.json` writes
`stopped_at_unix_ms`, wall-clock Unix milliseconds (the gate's armed wall time plus the monotonic
time since). `take-manifest.json` holds no times.

### Known limitations

{limits or "- None recorded."}

## Architecture

C4 diagrams of the fork, drawn by `docs/architecture/c4.py` from the code in this tree: the system in
its context, the separately running pieces, the desktop app's components, and the fork's files with
the calls between them, in two parts. Open a diagram for full size.{open_prs}

{images}
{END}"""
    path = ROOT / "README.md"
    text = path.read_text()
    if START in text:
        text = text[:text.index(START)] + section + text[text.index(END) + len(END):]
    else:
        # below upstream's header (its preview image), or at the top if that ever moves
        anchor = text.find("landing-cover.png")
        cut = text.index("\n", anchor) + 1 if anchor >= 0 else 0
        text = text[:cut] + "\n" + section + "\n" + text[cut:]
    path.write_text(text)


if __name__ == "__main__":
    if not shutil.which("dot"):
        sys.exit("c4: Graphviz is not installed (no `dot` on PATH)")
    for name in sys.argv[1:] or LEVELS:
        LEVELS[name]()
        print(OUT / f"c4_{name}.png")
    readme()
