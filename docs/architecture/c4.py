# /// script
# requires-python = ">=3.11"
# dependencies = ["diagrams"]
# ///
"""C4 diagrams of this fork, levels 1 to 4, and the README section that shows them.

    uv run docs/architecture/c4.py            # writes c4_context.png ... c4_code.png beside this file
    uv run docs/architecture/c4.py context    # just one level

Level 4 is read from the code on every run: each fork file that exists is drawn with its public
functions, and one not yet in the tree is drawn as "not built yet". The README section between the
fork markers is rewritten too, so its Known limitations list always matches the `// shortcut:`
comments in apps/desktop-gpui/src. This repo is public: no bucket, key, host or address appears here.
"""

import re
import shutil
import sys
from functools import partial
from pathlib import Path

from diagrams import Diagram
from diagrams.c4 import C4Node, Container, Database, Person, Relationship, System, SystemBoundary

# diagrams.c4 wraps text at 40 characters but draws 2.6in boxes, which cut lines off; 3.3in fits.
Container, Database, Person, System = (partial(f, width="3.3") for f in (Container, Database, Person, System))
Component = partial(C4Node, type="Component", width="3.3")
File = partial(C4Node, type="File", width="4.6")

OUT = Path(__file__).parent
ROOT = OUT.parents[1]
SYSTEM = "Cap fork for demo films"
GRAPH = {"splines": "spline", "nodesep": "0.8", "ranksep": "1.1"}
SHORTCUT = re.compile(r"//+\s*shortcut:\s*([^\n]+)")
START, END = "<!-- fork:start (written by docs/architecture/c4.py) -->", "<!-- fork:end -->"

# Level 4: the code the fork adds or changes, as (path, what it does).
CODE = [
    ("apps/desktop-gpui/src/window_log.rs", "Foreground window every 250 ms -> windows.jsonl, on the cursor clock"),
    ("apps/desktop-gpui/src/teleprompter_window.rs", "Shot list lines; Next shot appends {shot_id, segment, time_ms} to shots.json"),
    ("apps/desktop-gpui/src/gcs_mirror.rs", "Closed fragments hard-linked to outbox/, uploaded once; audio as deltas; manifest last"),
    ("apps/cli/src/events.rs", "cap events <take> --json: cursor.json and keyboard.bin as one sorted list"),
    ("crates/recording/src/output_pipeline/start_gate.rs", "The start-gate epoch every take time is measured from (upstream, reused)"),
    ("apps/desktop-gpui/src/theme.rs", "Colour and type tokens for the restyle"),
]


def draw(level, title):
    return Diagram(f"{SYSTEM} - {title}", filename=str(OUT / f"c4_{level}"), outformat="png",
                   show=False, direction="TB", graph_attr=GRAPH)


def context():
    """Level 1: who uses the fork and what it depends on."""
    with draw("context", "System Context"):
        lead = Person("Presenter", "Records a product demo on a Windows laptop")
        fork = System(SYSTEM, "Cap's desktop recorder plus shots, a window log and live upload")
        store = System("Object storage", "S3-compatible bucket, HMAC key from local config", external=True)
        pipeline = System("Film pipeline", "Turns an uploaded take into an edited demo film", external=True)
        ci = System("GitHub Actions", "Builds the unsigned Windows installer", external=True)
        lead >> Relationship("records, marks shots") >> fork
        fork >> Relationship("uploads the take while recording") >> store
        pipeline >> Relationship("pulls takes") >> store
        ci >> Relationship("installer") >> fork


def container():
    """Level 2: the separately running pieces."""
    with draw("container", "Containers"):
        lead = Person("Presenter", "")
        with SystemBoundary(SYSTEM):
            app = Container("Desktop app", "Rust, GPUI (apps/desktop-gpui)", "Studio recording, teleprompter, window log, upload")
            cli = Container("cap CLI", "Rust (apps/cli), same commit", "cap events --json; cap export")
            release = Container("Release workflow", "GitHub Actions, windows-2022", "Unsigned installer; updater and telemetry off")
            vendor = Database("contracts/vendor/", "JSON Schema, copied from the pipeline", "Shot list, shots, windows, manifest")
        store = Database("Object storage", "S3-compatible", "takes/<id>/: fragments, sidecars, manifest last")
        take = Database("Take folder", "<take>.cap on the laptop", "Never rewritten by the fork; uploads go via outbox/")
        lead >> Relationship("record, Next shot") >> app
        app >> Relationship("writes") >> take
        app >> Relationship("PUT closed fragments") >> store
        cli >> Relationship("reads the same project format") >> take
        app >> Relationship("validates outputs against") >> vendor
        release >> Relationship("builds") >> app


def component():
    """Level 3: inside the desktop app, only the parts the fork adds or reuses."""
    with draw("component", "Components of the desktop app"):
        with SystemBoundary("Desktop app (apps/desktop-gpui)"):
            gate = Component("Start gate", "crates/recording, reused", "The one clock for cursor.json, keyboard.bin, shots and windows")
            tele = Component("Teleprompter and shots", "teleprompter_window.rs", "Shows the shot list; Next shot writes shots.json")
            win = Component("Window log", "window_log.rs", "Foreground title and process on change -> windows.jsonl")
            mirror = Component("Upload mirror", "gcs_mirror.rs", "outbox/ hard links; resumable from uploaded.jsonl; 120 s drain at stop")
        store = Database("Object storage", "S3-compatible", "")
        tele >> Relationship("times from") >> gate
        win >> Relationship("times from") >> gate
        mirror >> Relationship("PUT") >> store


def code():
    """Level 4: the fork's files as they are in this tree, with their public functions."""
    with draw("code", "Code (read from the tree)"):
        nodes = {}
        with SystemBoundary("Fork code"):
            for path, what in CODE:
                file = ROOT / path
                built = file.is_file()
                names = sorted(set(re.findall(r"pub(?:\([a-z]+\))?\s+fn\s+(\w+)", file.read_text())))[:6] if built else []
                detail = what + (". fns: " + ", ".join(names) if names else "")
                nodes[path] = File(Path(path).name + ("" if built else " (not built yet)"), str(Path(path).parent), detail)
        gate = nodes["crates/recording/src/output_pipeline/start_gate.rs"]
        for path in ("apps/desktop-gpui/src/window_log.rs", "apps/desktop-gpui/src/teleprompter_window.rs"):
            nodes[path] >> Relationship("times from") >> gate

LEVELS = {"context": context, "container": container, "component": component, "code": code}


def shortcuts():
    """Every `// shortcut:` comment in the desktop app, as (file, text)."""
    found = []
    for path in sorted((ROOT / "apps/desktop-gpui/src").rglob("*.rs")):
        found += [(path.relative_to(ROOT).as_posix(), text.strip()) for text in SHORTCUT.findall(path.read_text())]
    return found


def readme():
    """Rewrite the fork section of README.md, placed below upstream's header on the first run."""
    limits = "\n".join(f"- {text} (`{path}`)" for path, text in shortcuts()) or "- None recorded."
    images = "\n".join(f"![C4 {level}](docs/architecture/c4_{level}.png)" for level in LEVELS)
    section = f"""{START}
## This fork

A fork of Cap for recording product demos that are turned into edited films. It adds a teleprompter
with shot marks (`shots.json`), a foreground-window log (`windows.jsonl`), live upload of each closed
fragment to S3-compatible storage, and `cap events --json`. Every time it writes is on the take's
start-gate clock, the one `cursor.json` uses. Upload settings live in a local config file, never in
this repo.

{images}

### Known limitations

{limits}
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
