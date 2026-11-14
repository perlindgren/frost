#!/usr/bin/env python3
"""Decide which of the gate's suites a change can reach.

Called by the `plan` job of `.github/workflows/gate.yml`, once per run, before
either runner starts. It writes one `key=true` or `key=false` line per suite to
`$GITHUB_OUTPUT` and a table to `$GITHUB_STEP_SUMMARY` saying which changed file
pulled which suite into the run, so the decision is readable from the run page
rather than reconstructed from which steps happened to be green.

Two rules, and the second is the reason the first is allowed to exist:

1. A suite runs when a changed file belongs to a package that suite names, or to
   a package that some suite-named package depends on, following dependency
   edges backwards and including dev-dependencies. The closure is read out of
   `cargo metadata`, never written by hand, because a path table rots the day a
   crate moves and a rotting path table skips tests silently. `include_str!` and
   `include_bytes!` count as dependency edges too — six plugins and the rack
   compile a font out of `nice-tuner`'s directory by path, and cargo has no idea
   — so those are read off the source as well. See `embeddings()`.
2. When this script cannot see far enough to decide, it says `full` and every
   suite runs: no base commit to diff against, a force-push, a push to the
   default branch, an event that is neither, a workspace member that no suite
   names, a metadata graph it does not recognise. The cost of guessing wrong in
   that direction is a run that spent three more minutes; in the other direction
   it is a merge that nobody tested.

The suites left running by a `false` are the test suites and the two claims about
the dependency graph. Formatting, clippy, the packaging check, the web check and
the scans over tracked files are not this script's to skip: they are the cheap
half of the gate, they all compile or inspect everything anyway, and a warning
travels further than a test.

One accident this design has, and it is deliberate: the engine's own package lives
at the root of the tree, so a changed file no deeper package claims belongs to
`frost` — and `frost` is depended on by every plugin, every game and the rack, so
"this file belongs to nobody in particular" and "this file reaches the whole gate"
are the same answer. That is the conservative direction, which is the only
direction worth choosing when the question is whether to run a test.
"""

import argparse
import json
import os
import posixpath
import re
import subprocess
import sys

# The suites, one key per step group in `.github/workflows/gate.yml`, with the
# packages named by that step's `-p` flags. This table and that file are the same
# decision written twice, and they are kept in step by the rule in this file's
# header: a package that no suite here names and that no bare step covers makes
# the whole gate run rather than making a suite get skipped.
SUITES = {
    "tuner": ["nice-tuner", "pitch"],
    "nam": ["nice-nam"],
    "common": ["nice-common"],
    "wah": ["nice-wah"],
    "phase": ["nice-phase"],
    "pi_face": ["nice-pi-face"],
    "echo": ["nice-echo"],
    "ecomplex": ["nice-ecomplex"],
    "rack": [
        "nice-rack",
        "nice-rack-engine",
        "nice-clap-host",
        "nice-rack-standalone",
        "rack-window-input",
    ],
}

# The bare `cargo test` and the bare `cargo test -p frost --lib --no-run`: no `-p`
# flags, so what they cover is the workspace's default members, which cargo itself
# knows and this script reads rather than copies.
ENGINE_SUITE = "engine"

# Not a suite: the two steps in the Windows job that read the dependency graph to
# assert what is and is not in it. Only a manifest can move a package into or out
# of a graph, so that is the only change that can invalidate their answer.
GRAPH_SUITE = "graph"

# What the summary calls each suite, in the words `gate.yml` uses in its step
# names, so the table and the run's step list read as the same thing.
LABEL = {
    "tuner": "the tuner and its detector",
    "nam": "the model plugin",
    "common": "the shared crate",
    "wah": "the wah",
    "phase": "the phaser",
    "pi_face": "the Pi-Fase",
    "echo": "the Space Echo",
    "ecomplex": "the Echoplex",
    "rack": "the rack and its host adapter",
    ENGINE_SUITE: "the engine and its games",
    GRAPH_SUITE: "the two graph claims",
}


class PlanError(Exception):
    """Something this script cannot decide. Carries the sentence for the summary."""


def run(argv, cwd):
    proc = subprocess.run(argv, cwd=cwd, capture_output=True, text=True, encoding="utf-8")
    return proc.returncode, proc.stdout, proc.stderr


def changed_paths(base, head, cwd):
    if not base:
        raise PlanError("there is no base commit to diff against")
    code, _, _ = run(["git", "cat-file", "-e", base + "^{commit}"], cwd)
    if code != 0:
        raise PlanError("`%s` is not in this checkout, so the change cannot be read" % base)
    code, out, err = run(["git", "diff", "--name-only", "-z", base, head], cwd)
    if code != 0:
        raise PlanError("`git diff %s %s` failed: %s" % (base[:8], head[:8], err.strip()[:200]))
    return sorted(p for p in out.split("\0") if p)


def workspace_metadata(cwd):
    code, out, err = run(
        ["cargo", "metadata", "--no-deps", "--offline", "--format-version", "1"], cwd
    )
    if code != 0:
        raise PlanError("`cargo metadata` failed: %s" % err.strip()[-200:])
    try:
        meta = json.loads(out)
    except ValueError as exc:
        raise PlanError("`cargo metadata` printed something that is not JSON: %s" % exc)
    for key in ("packages", "workspace_root", "workspace_default_members"):
        if key not in meta:
            raise PlanError("`cargo metadata` output has no `%s`; this script does not know that format" % key)
    return meta


def graph_of(meta):
    """Return (package directories, default members, reverse dependency edges)."""
    root = meta["workspace_root"]
    directories = {}
    for package in meta["packages"]:
        relative = os.path.relpath(package["manifest_path"], root).replace(os.sep, "/")
        directories[package["name"]] = relative.rsplit("/", 1)[0] if "/" in relative else "."

    default_ids = set(meta["workspace_default_members"])
    defaults = {p["name"] for p in meta["packages"] if p["id"] in default_ids}
    if len(defaults) != len(default_ids):
        raise PlanError("the default members name packages that are not in this workspace")

    dependents = {name: set() for name in directories}
    for package in meta["packages"]:
        for dep in package["dependencies"]:
            if dep["name"] in dependents:
                dependents[dep["name"]].add(package["name"])
            elif dep.get("path"):
                # A path dependency that is not a member of this workspace: a
                # renamed one, or something this script's cargo and this file
                # disagree about. Either way the closure below would be missing an
                # edge, and a missing edge skips a suite.
                raise PlanError(
                    "`%s` depends on the path dependency `%s`, which is not a workspace member"
                    % (package["name"], dep["name"])
                )
    return directories, defaults, dependents


def reach(crate, dependents):
    """`crate` and everything that depends on it, directly or through anything."""
    seen = {crate}
    queue = [crate]
    while queue:
        for parent in dependents.get(queue.pop(), ()):
            if parent not in seen:
                seen.add(parent)
                queue.append(parent)
    return seen


def owner(path, directories):
    """The package whose directory deepest contains `path`; the root package else."""
    best, depth = None, -1
    for name, directory in directories.items():
        prefix = "" if directory == "." else directory + "/"
        if path.startswith(prefix) and len(prefix) > depth:
            best, depth = name, len(prefix)
    return best


# One `include_str!("…")` or `include_bytes!("…")` call, with the literal it names.
EMBEDS = re.compile(r'include_(?:str|bytes)!\(\s*"((?:[^"\\]|\\.)*)"')


def embeddings(cwd, directories):
    """`(path, package)` for each file one package embeds from another's directory.

    A dependency graph is not the only way one package ends up compiled out of
    another's files: `include_bytes!` does it by path, and cargo has no idea. This
    tree really does it — six plugins and the rack embed
    `nice-tuner/assets/fonts/FluentSystemIcons-Regular.ttf` from beside their own
    icons — so without this, a change to that one file would be planned as a tuner
    change and the seven suites that bake the font in would stand aside.
    `.githooks/pre-push` makes the same argument for the paths it skips, which is
    where the shape of this check came from.
    """
    code, out, err = run(
        ["git", "grep", "-I", "-e", "include_str!(", "-e", "include_bytes!(", "--", "*.rs"], cwd
    )
    if code not in (0, 1):  # 1 is git's "no matches", which is a fine answer
        raise PlanError("`git grep` for the embedded files failed: %s" % err.strip()[:200])

    pairs, unreadable = set(), 0
    for line in out.split("\n"):
        if not line.strip():
            continue
        source, _, text = line.partition(":")
        names = EMBEDS.findall(text)
        unreadable += text.count("include_str!(") + text.count("include_bytes!(") - len(names)
        for name in names:
            target = posixpath.normpath(
                posixpath.join(posixpath.dirname(source), name.replace("\\", "/"))
            )
            embedder, embedded_in = owner(source, directories), owner(target, directories)
            if inert(target):
                # The inert list below is only inert because nothing is compiled out
                # of it. That is a claim about this tree, so it is checked against the
                # tree rather than trusted, which is how `.githooks/pre-push` treats
                # its own skip list: the day a document is `include_str!`'d into a
                # build, editing it changes the build.
                raise PlanError(
                    "`%s` embeds `%s`, which this script would otherwise have called inert"
                    % (source, target)
                )
            if embedder != embedded_in:
                pairs.add((target, embedder))
    if unreadable:
        raise PlanError(
            "%d `include_bytes!` or `include_str!` calls do not name a plain path, so this script "
            "cannot tell who embeds what" % unreadable
        )
    return sorted(pairs)


def inert(path):
    """A change no test in this gate could be the witness for.

    Prose, licences, and `.gitignore`. The last is the one worth a sentence: a
    `.gitignore` cannot change a compiled unit or a test result, and the two steps
    that would notice one — the droppings scan and the packaging check, which asks
    cargo what it would put in the crate — run on every change regardless.

    The prose is not compiled either, and that is not taken on faith: `embeddings()`
    refuses the plan the day an `include_str!` names one of these paths. Measured
    across the tracked tree while this file was written, every embed names an asset,
    a font, a shader or a level, and none names a document.
    """
    return (
        path.endswith(".md")
        or path.startswith("docs/")
        or "/docs/" in path
        or path == ".gitignore"
        or path.endswith("/.gitignore")
        or path == "LICENSE"
        or path.startswith("LICENSE-")
    )


def touches_graph(path):
    # A manifest is the only thing that can move a package into or out of a graph,
    # and the gate's own file is here because the two claims are written in it: a
    # step whose script just changed is a step that has to run to be tested.
    return (
        path == "Cargo.lock"
        or path.endswith("/Cargo.lock")
        or path == "Cargo.toml"
        or path.endswith("/Cargo.toml")
        or path.startswith(".cargo/")
        or path.startswith("rust-toolchain")
        or path == ".github/workflows/gate.yml"
    )


def decide(options):
    """Return (mode, reason, results, notes) where results maps every output key to a bool."""
    keys = list(SUITES) + [ENGINE_SUITE, GRAPH_SUITE]
    directories, defaults, dependents = graph_of(workspace_metadata(options.root))
    suites = dict(SUITES)
    suites[ENGINE_SUITE] = sorted(defaults)

    paths = changed_paths(options.base, options.head, options.root)
    embedded = embeddings(options.root, directories)
    seeds, untouched, graph = {}, [], False
    for path in paths:
        if inert(path):
            untouched.append(path)
            continue
        if touches_graph(path):
            graph = True
        named = {owner(path, directories)}
        named.update(embedder for target, embedder in embedded if target == path)
        for crate in named:
            seeds.setdefault(crate, set()).add(path)

    named = set(defaults)
    for listed in SUITES.values():
        named.update(listed)
    strangers = sorted(set(directories) - named)
    if strangers and set(seeds) & set(strangers):
        # Not this script's business which suite a member belongs to — that is a
        # decision in `gate.yml` — but a change to a package no suite names is not
        # evidence that its own tests already ran somewhere else either.
        raise PlanError(
            "`%s` is a workspace member no suite names, so nothing here can say its tests already ran"
            % "`, `".join(sorted(set(seeds) & set(strangers)))
        )

    def named_by(crate):
        """How the summary describes a seed crate, telling its own files from a file
        it merely embeds."""
        home = directories[crate]
        prefix = "" if home == "." else home + "/"
        if any(path.startswith(prefix) for path in seeds[crate]):
            return "`%s`" % crate
        return "`%s`, which embeds a file that changed" % crate

    results = {key: False for key in keys}
    pulls = {key: [] for key in keys}
    for crate in sorted(seeds):
        affected = reach(crate, dependents)
        for suite, members in suites.items():
            if affected & set(members):
                results[suite] = True
                pulls[suite].append(named_by(crate))
    results[GRAPH_SUITE] = graph

    changed = {
        "paths": len(paths),
        "crates": sorted(seeds),
        "inert": untouched,
        "strangers": strangers,
        "pulls": pulls,
    }
    return "plan", "planned from %d changed %s" % (
        len(paths),
        "file" if len(paths) == 1 else "files",
    ), {key: results[key] for key in keys}, changed


def summary_text(mode, reason, results, facts, options):
    lines = ["## What this gate decided to run", ""]
    if mode == "full":
        lines.append(
            "**The whole gate runs**, because %s. Every suite below is on." % reason
        )
    else:
        lines.append(
            "**Planned run**: `%s`..`%s`, %d changed %s, %s."
            % (
                (options.base or "")[:8],
                options.head[:8],
                facts["paths"],
                "file" if facts["paths"] == 1 else "files",
                ("reaching " + ", ".join("`%s`" % c for c in facts["crates"]))
                if facts["crates"]
                else "no package",
            )
        )
    lines.append("")
    lines.append("| suite | runs | because |")
    lines.append("| --- | --- | --- |")
    for key in list(SUITES) + [ENGINE_SUITE, GRAPH_SUITE]:
        if mode == "full":
            why = "the whole gate is on"
        elif results[key]:
            if key == GRAPH_SUITE:
                why = "a manifest moved, which is the only thing that can change the graph"
            else:
                why = "reached by " + ", ".join(
                    "`%s`" % c for c in sorted(set(facts["pulls"][key]))
                )
        elif facts["inert"] and not facts["crates"]:
            why = "only prose, licences and ignore rules changed"
        else:
            why = "nothing changed that it or anything using it is built from"
        lines.append(
            "| %s (`%s`) | %s | %s |"
            % (LABEL[key], key, "yes" if results[key] else "no", why)
        )
    lines.append("")
    if facts["strangers"]:
        lines.append(
            "Members no suite names, so any change reaching one runs everything: %s."
            % ", ".join("`%s`" % s for s in facts["strangers"])
        )
        lines.append("")
    lines.append(
        "Formatting, clippy, the packaging check, the web check and the scans over tracked "
        "files are not decided here: they run every time."
    )
    if facts["inert"]:
        lines.append("")
        lines.append(
            "Changes no suite could be the witness for: %s." % ", ".join("`%s`" % d for d in facts["inert"][:8])
        )
    return "\n".join(lines) + "\n"


def emit(options, mode, reason, results, facts):
    keys = list(SUITES) + [ENGINE_SUITE, GRAPH_SUITE]
    if options.output:
        with open(options.output, "a", encoding="utf-8") as handle:
            handle.write("mode=%s\n" % mode)
            for key in keys:
                handle.write("%s=%s\n" % (key, "true" if results[key] else "false"))
    text = summary_text(mode, reason, results, facts, options)
    if options.summary:
        with open(options.summary, "a", encoding="utf-8") as handle:
            handle.write(text + "\n")
    print(text)
    print("outputs: mode=%s %s" % (mode, " ".join("%s=%s" % (k, results[k]) for k in keys)))


def full_mode(options, reason):
    results = {key: True for key in list(SUITES) + [ENGINE_SUITE, GRAPH_SUITE]}
    facts = {"paths": 0, "crates": [], "inert": [], "strangers": [], "pulls": {}}
    emit(options, "full", reason, results, facts)
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--base", default="", help="the commit the change is measured from")
    parser.add_argument("--head", default="HEAD", help="the commit being gated")
    parser.add_argument("--event", default="", help="github.event_name")
    parser.add_argument("--ref", default="", help="github.ref")
    parser.add_argument("--default-branch", default="main", help="the branch the gate runs everything on")
    parser.add_argument("--forced", default="false", help="github.event.forced")
    parser.add_argument("--root", default=os.getcwd(), help="the workspace to read")
    parser.add_argument("--output", default=os.environ.get("GITHUB_OUTPUT", ""), help="where to write outputs")
    parser.add_argument("--summary", default=os.environ.get("GITHUB_STEP_SUMMARY", ""), help="where to write the summary")
    options = parser.parse_args(argv)

    if options.event not in ("pull_request", "push"):
        return full_mode(options, "`%s` is not a change with a base to diff against" % (options.event or "an event"))
    if options.forced.lower() == "true":
        return full_mode(options, "the push was forced, so the previous head is not behind this one")
    if options.ref == "refs/heads/%s" % options.default_branch:
        return full_mode(options, "this is a push to `%s`, and the branch every later plan is measured from" % options.default_branch)
    if not options.base:
        return full_mode(options, "no base was named, which on a push means the branch did not exist a moment ago")

    try:
        mode, reason, results, facts = decide(options)
    except PlanError as exc:
        return full_mode(options, str(exc))
    except Exception as exc:  # noqa: BLE001 - the fallback is the point of this line
        return full_mode(options, "the planner itself failed: %s: %s" % (type(exc).__name__, exc))
    emit(options, mode, reason, results, facts)
    return 0


if __name__ == "__main__":
    sys.exit(main())
