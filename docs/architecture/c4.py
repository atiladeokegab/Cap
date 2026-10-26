# /// script
# requires-python = ">=3.11"
# dependencies = ["diagrams"]
# ///
"""C4 diagrams of this fork, levels 1 to 4, and the README section that shows them.

    uv run docs/architecture/c4.py            # writes c4_context.png ... c4_code.png beside this file
    uv run docs/architecture/c4.py context    # just one level

Level 4 is read from the code on every run: each fork file that exists is drawn with its public
functions (a workflow with its steps), and one not yet in the tree is drawn as "not built yet". A
level 4 arrow is drawn only when the call it names is in the source file it starts from. The README
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
BUCKET = "The takes bucket"

# Level 4: the code the fork adds or changes, grouped by where it runs, as (path, what it does).
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
        ("apps/desktop-gpui/src/teleprompter_window.rs", "Shows the shot on screen; load_shot_list GETs it from the bucket; mark_shot"),
        ("apps/desktop-gpui/src/shots.rs", "pick bucket or local list, parse (shotlist rules), ShotClock, mark -> shots.json"),
        ("apps/desktop-gpui/src/hotkeys.rs", "Ctrl+Alt+N / Ctrl+Alt+R while a shot list shows -> Next shot / Retake"),
        ("apps/desktop-gpui/src/window_log.rs", "Windows: foreground window every 250 ms -> windows.jsonl, on change only"),
        ("apps/desktop-gpui/src/gcs_mirror.rs", "Reads mirror.json from app data; start, stop, resume_pending"),
        ("apps/desktop-gpui/src/theme.rs", "Light palette and type tokens for the restyle"),
    ],
    CRATE: [
        ("apps/desktop-gpui/gcs-mirror/src/lib.rs", "Mirror: outbox hard links, scan, freeze -> stop.json, drain, manifest last"),
        ("apps/desktop-gpui/gcs-mirror/src/sigv4.rs", "SigV4 signing for the XML API with an HMAC key"),
        ("apps/desktop-gpui/gcs-mirror/src/bin/cap-driver.rs", "Test adapter, JSON on stdin: sign, scan, stop, resume, shots"),
    ],
    CLI: [
        ("apps/cli/src/events.rs", "cap events <take> --json: cursor.json and keyboard.bin as one sorted list"),
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
# Level 4 arrows as (from, the text that must be in `from`, to, label).
CALLS = [
    ("apps/desktop-gpui/src/recording.rs", "gcs_mirror::start", "apps/desktop-gpui/src/gcs_mirror.rs", "start; stop before remux"),
    ("apps/desktop-gpui/src/recording.rs", "window_log::start", "apps/desktop-gpui/src/window_log.rs", "start, stop"),
    ("apps/desktop-gpui/src/session.rs", "window_log::set_paused", "apps/desktop-gpui/src/window_log.rs", "set_paused"),
    ("apps/desktop-gpui/src/hotkeys.rs", "view.mark_shot", "apps/desktop-gpui/src/teleprompter_window.rs", "mark_shot"),
    ("apps/desktop-gpui/src/teleprompter_window.rs", "shot_time()", "apps/desktop-gpui/src/session.rs", "shot_time"),
    ("apps/desktop-gpui/src/teleprompter_window.rs", "shots::pick", "apps/desktop-gpui/src/shots.rs", "pick, mark"),
    ("apps/desktop-gpui/src/teleprompter_window.rs", "cap_gcs_mirror::request", "apps/desktop-gpui/gcs-mirror/src/lib.rs", "GET the shot list"),
    ("apps/desktop-gpui/src/session.rs", "armed_at", "crates/recording/src/output_pipeline/start_gate.rs", "armed instant"),
    ("apps/desktop-gpui/src/recording.rs", "armed_at", "crates/recording/src/output_pipeline/start_gate.rs", "window log epoch"),
    ("apps/desktop-gpui/src/gcs_mirror.rs", "Mirror::new", "apps/desktop-gpui/gcs-mirror/src/lib.rs", "Mirror, spawn, stop, drain"),
    ("apps/desktop-gpui/gcs-mirror/src/lib.rs", "sigv4::authorize", "apps/desktop-gpui/gcs-mirror/src/sigv4.rs", "authorize"),
    ("apps/desktop-gpui/gcs-mirror/src/bin/cap-driver.rs", "Mirror::new", "apps/desktop-gpui/gcs-mirror/src/lib.rs", "scan, stop, resume"),
    ("apps/desktop-gpui/gcs-mirror/src/bin/cap-driver.rs", "src/shots.rs", "apps/desktop-gpui/src/shots.rs", "includes by path"),
    ("crates/verify-stimulus/src/bin/window_log_probe.rs", "src/window_log.rs", "apps/desktop-gpui/src/window_log.rs", "includes by path"),
    (".github/workflows/verify-recording.yml", "cap-verify-stimulus", "crates/verify-stimulus/src/main.rs", "records the stimulus"),
    (".github/workflows/verify-recording.yml", "window-log-probe", "crates/verify-stimulus/src/bin/window_log_probe.rs", "runs"),
    (".github/workflows/verify-recording.yml", "op: \"shots\"", "apps/desktop-gpui/gcs-mirror/src/bin/cap-driver.rs", "shots, scan, stop"),
    (".github/workflows/verify-recording.yml", "events \"$take\" --json", "apps/cli/src/events.rs", "cap events --json"),
]


def draw(level, title, direction="TB"):
    return Diagram(f"{SYSTEM} - {title}", filename=str(OUT / f"c4_{level}"), outformat="png",
                   show=False, direction=direction, graph_attr=GRAPH)


def context():
    """Level 1: who uses the fork and what it talks to."""
    with draw("context", "System Context"):
        lead = Person("Presenter", "Records a product demo on a Windows laptop")
        fork = System(SYSTEM, "Cap's desktop recorder plus a shot list, a window log, live upload and cap events")
        store = System(BUCKET, "S3-compatible object storage, HMAC key from local config", external=True)
        pipeline = System("Film pipeline", "On the hub VM: turns an uploaded take into an edited demo film", external=True)
        ci = System("GitHub Actions", "Builds the unsigned Windows installer; records the Windows fixture take", external=True)
        lead >> Relationship("records; Next shot, Retake") >> fork
        fork >> Relationship("reads the shot list") >> store
        fork >> Relationship("uploads the take while recording, manifest last") >> store
        pipeline >> Relationship("publishes the shot list; pulls takes/") >> store
        pipeline >> Relationship("cap events --json") >> fork
        ci >> Relationship("installer; fixture take") >> fork
        ci >> Relationship("fixture take under ci/, if its secret is set") >> store


def container():
    """Level 2: the separately running pieces."""
    with draw("container", "Containers"):
        lead = Person("Presenter", "")
        with SystemBoundary(SYSTEM):
            app = Container("Desktop app", "Rust, GPUI (apps/desktop-gpui)", "Studio recording, teleprompter shot list, window log, upload mirror")
            cli = Container("cap CLI", "Rust (apps/cli), same commit", "cap events <take> --json")
            driver = Container("cap-driver", "Rust (gcs-mirror crate, test binary)", "The mirror and the shot logic, one JSON request per run")
            fixture = Container("Recording fixture", "GitHub Actions, windows-2022", "Studio take of the stimulus, SendInput clicks, artefact")
            release = Container("Release workflow", "GitHub Actions, windows-2022", "Unsigned installer, no updater artefacts")
            vendor = Database("contracts/vendor/0.1.0", "JSON Schema from the film repo", "shotlist, shots, windows, cap-events, manifest")
        store = Database(BUCKET, "S3-compatible", "shotlist/current.json; takes/<take>/ with the manifest last; ci/")
        take = Database("Take folder", "<take>.cap on the laptop", "Cap's files, plus shots.json, windows.jsonl, outbox/")
        lead >> Relationship("record; Next shot, Retake") >> app
        app >> Relationship("writes") >> take
        app >> Relationship("GET the shot list; PUT the take, manifest last") >> store
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
            shots = Component("Shots", "shots.rs", "Bucket list or local file; parse; marks -> shots.json")
            win = Component("Window log", "window_log.rs", "Windows only: foreground title and process on change")
            wiring = Component("Mirror wiring", "gcs_mirror.rs", "mirror.json from app data; start, stop, resume_pending")
            mirror = Component("Mirror", "gcs-mirror crate: lib.rs, sigv4.rs", "outbox/ hard links, uploaded.jsonl, stop.json, manifest last")
        store = Database(BUCKET, "S3-compatible", "")
        take = Database("Take folder", "<take>.cap", "")
        keys >> Relationship("mark_shot") >> tele
        tele >> Relationship("pick, mark") >> shots
        tele >> Relationship("shot_time") >> rec
        tele >> Relationship("signed GET") >> mirror
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
    names = sorted(set(re.findall(r"pub(?:\([a-z]+\))?\s+fn\s+(\w+)", text)))
    return "fns: " + ", ".join(names[:6]) + (", ..." if len(names) > 6 else "") if names else ""


def code():
    """Level 4: the fork's files as they are in this tree, with what they define."""
    # left to right: one column per call depth keeps 17 files narrow enough for a README
    with draw("code", "Code (read from the tree)", "LR"):
        nodes, sources = {}, {}
        with SystemBoundary("Fork code"):
            for group, files in CODE.items():
                with Cluster(group):
                    for path, what in files:
                        file = ROOT / path
                        sources[path] = file.read_text() if file.is_file() else None
                        detail = what + (". " + outline(path, sources[path]) if sources[path] else "")
                        name = Path(path).name + ("" if sources[path] else " (not built yet)")
                        nodes[path] = File(name, str(Path(path).parent), detail)
        for start, call, end, label in CALLS:
            if call in (sources[start] or "") and sources[end]:
                nodes[start] >> Relationship(label) >> nodes[end]

LEVELS = {"context": context, "container": container, "component": component, "code": code}


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
    limits = "\n".join(f"- {text} (`{path}`" + (f", `{item}`)" if item else ")") for path, text, item in shortcuts())
    images = "\n".join(f"![C4 {level}](docs/architecture/c4_{level}.png)" for level in LEVELS)
    section = f"""{START}
## This fork

A fork of Cap for recording product demos that a film pipeline turns into edited films. The desktop
app adds a shot list in the teleprompter, read from the bucket (or a local file), with Next shot and
Retake on global hotkeys that write shot marks to the take's `shots.json`; a foreground-window log on
Windows (`windows.jsonl`); an upload mirror that streams each studio take to S3-compatible storage
while it records, with `take-manifest.json` last; and a light restyle. The CLI adds
`cap events <take> --json`. Every time the fork writes is on the take's start-gate clock, the one
`cursor.json` uses. GitHub Actions builds an unsigned Windows installer and records a real Windows
fixture take. Upload settings live in a local `mirror.json` in the app data folder, never in this
repo.

{images}

### Known limitations

{limits or "- None recorded."}
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
