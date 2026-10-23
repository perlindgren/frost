#!/usr/bin/env bash
# Re-apply the /dev/dri grant to the installed DeepSeek Harness sandbox.
#
# WHY THIS EXISTS
#
# On Linux every confined command runs under bwrap, and the profile is built
# from a literal argv list (dsh-sandbox-local/lib/index.js):
#
#     ["--ro-bind","/","/","--dev","/dev","--unshare-pid","--proc","/proc","--die-with-parent"]
#     + ("--tmpfs","/tmp") and ("--bind", workspace, workspace) for workspace-write
#
# `--dev /dev` mounts bwrap's own minimal device tree, so /dev/dri — the DRM
# render nodes every Vulkan and GL client enumerates through — is not
# permission-denied inside the sandbox, it is *absent*. `vulkaninfo` reports
# zero devices, wgpu's `request_adapter` resolves to null, and a headless
# WebGPU run quietly falls back to llvmpipe and draws something you then
# mistake for a real frame. The file-effect policy offers only read-only,
# workspace-write and danger-full-access: three modes that choose between no
# workspace bind and one workspace bind, with no way to say "this one device,
# read-write, nothing else". Escalation grants the whole machine for one
# command; the preset grants the whole machine all the time. Neither is the
# narrow thing anybody wants.
#
# WHAT THE PATCH DOES
#
# One opt-in environment variable, empty by default so an unmodified
# deployment behaves exactly as before:
#
#     DSH_SANDBOX_DEV_BINDS=/dev/dri          (colon-separated list)
#
# For each entry the profile gains `--dev-bind PATH PATH`, appended after
# `--dev /dev`. Entries are honoured only if they live under /dev/ and exist
# on the host, so this cannot be steered into a general filesystem-mount knob.
# A render node is still opened under the host's own group membership
# (video/render), so the confined process gains no privilege it would not
# have outside; it gains only the node it was missing.
#
# The probe that decides whether bwrap works at all is deliberately left
# WITHOUT the binds: if a host ever failed the probe over a device path, the
# provider would fall back a rung to Landlock, which cannot grant device
# nodes in the first place — a silent loss of enforcement to keep a
# convenience. Probing the isolation and granting the device stay separate
# questions.
#
# THE CAVEAT, STATED PLAINLY
#
# This edits a *built bundle inside an npx cache*
# (`~/.npm/_npx/<hash>/node_modules/@deepseek-ai/...`), not a source checkout.
# The next `npx` resolution at a new version overwrites it silently, and
# nothing will appear broken: the sandbox just goes back to hiding /dev/dri,
# and a WebGPU experiment starts producing llvmpipe frames again. Re-run this
# script after any harness update, and note that the running server has the
# old module in memory — it takes effect only for a newly started `dsh web`.
#
# VERIFY IT, DON'T ASSUME IT
#
#     ls /dev/dri           # from a confined command, after the restart
#     vulkaninfo --summary  # expect the real adapters, not zero devices
set -euo pipefail

python3 - "$@" <<'PY'
import glob, os, shutil, subprocess, sys, tempfile

marker = "DSH_SANDBOX_DEV_BINDS"

candidates = sorted(
    glob.glob(os.path.expanduser("~/.npm/_npx/*/node_modules/@deepseek-ai/dsh-sandbox-local/lib/index.js")),
    key=os.path.getmtime,
)
if not candidates:
    sys.exit("no installed dsh-sandbox-local bundle found under ~/.npm/_npx")

for path in candidates:
    src = open(path).read()
    if marker in src:
        # A patch that parsed once can still be a broken one — an earlier run
        # of this script inserted `, false` on the wrong side of a parenthesis
        # and left the bundle unparsable. Say so, and repair from the backup
        # rather than reporting success about a file node cannot load.
        if subprocess.run(["node", "--check", path], capture_output=True).returncode == 0:
            print(f"already patched: {path}")
            continue
        if not os.path.exists(path + ".orig"):
            sys.exit(f"{path} carries the patch but does not parse, and there is no .orig to restore")
        shutil.copy2(path + ".orig", path)
        print(f"previous patch did not parse; restored the backup and re-applying: {path}")
        src = open(path).read()

    # 1. The helper, inserted above the profile builder it feeds.
    anchor = "function bwrapProfileArgs(policy) {\n\tconst args = ["
    helper = '''/**
* Extra host device subtrees to reattach inside the sandbox, from
* `DSH_SANDBOX_DEV_BINDS` (colon-separated, e.g. "/dev/dri"); empty by
* default, so the profile is what it always was unless someone asks.
*
* `--dev /dev` replaces the device tree with bwrap's own, which leaves
* /dev/dri absent rather than denied — and a DRM-less sandbox cannot see a
* GPU at all, so every headless WebGPU or Vulkan run inside it silently
* becomes a CPU one. Only paths under /dev/ that exist on the host count, so
* this stays a device grant and cannot be widened into a mount knob; the
* node itself is still reached through the host's group permissions, so no
* privilege is added, only the missing path.
*/
function deviceBinds() {
	const raw = process.env.DSH_SANDBOX_DEV_BINDS ?? "";
	return raw.split(":").map((path) => path.trim()).filter((path) => path.startsWith("/dev/") && !path.includes("..") && existsSync(path));
}
/**
* @param withDevices - false for the capability probe, which must not depend
*   on a device being present: failing it would drop the provider to Landlock,
*   which cannot express device nodes at all.
*/
function bwrapProfileArgs(policy, withDevices = true) {
\tconst args = ['''
    if src.count(anchor) != 1:
        print(f"skip (anchor seen {src.count(anchor)} times, expected 1): {path}")
        continue
    src = src.replace(anchor, helper)

    # 2. The binds themselves, after `--dev /dev` has put its tmpfs in place.
    tail = ('\t\targs.push("--bind", policy.workspaceRoot, policy.workspaceRoot);\n'
            "\t}\n\treturn args;\n}")
    if src.count(tail) != 1:
        sys.exit(f"cannot find the end of bwrapProfileArgs in {path} — refusing a partial patch")
    src = src.replace(tail, ('\t\targs.push("--bind", policy.workspaceRoot, policy.workspaceRoot);\n'
                             "\t}\n"
                             "\tif (withDevices) for (const device of deviceBinds()) args.push(\"--dev-bind\", device, device);\n"
                             "\treturn args;\n}"))

    # 3. The probe keeps the bare profile, for the reason in the comment.
    #    `, false` belongs INSIDE the call's parentheses, before its `)` —
    #    appending it after them is a syntax error, which is what the syntax
    #    check at the end of this script is there to catch.
    probe = '...bwrapProfileArgs({\n\t\t\tmode: "read-only",\n\t\t\tworkspaceRoot: "/"\n\t\t}),'
    bare = probe[:-2] + ", false),"
    if src.count(probe) == 1:
        src = src.replace(probe, bare)
    else:
        print("note: probe call site not matched; the probe will include the device binds")

    # Parse the result on a scratch copy BEFORE putting it where the sandbox
    # lives: a bundle that does not load is worse than one that merely lacks
    # the grant, and the provider would not report it until a command failed
    # to start. The backup is kept (never overwritten) so a repair is one
    # copy away.
    fd, scratch = tempfile.mkstemp(suffix=".js")
    os.write(fd, src.encode())
    os.close(fd)
    try:
        subprocess.run(["node", "--check", scratch], check=True)
    finally:
        os.unlink(scratch)
    if not os.path.exists(path + ".orig"):
        shutil.copy2(path, path + ".orig")
    open(path, "w").write(src)
    print(f"patched: {path}\n  backup: {path}.orig")
PY
