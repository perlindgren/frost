#!/usr/bin/env python3
"""Serve the web/ build with no caching whatsoever.

The stock `python -m http.server` sends no Cache-Control header, so
Firefox applies heuristic freshness and can keep replaying a stale
wasm — a build so old it still drew the blue frost rim — through
page reloads. This server says `no-store` on every response: every
reload fetches the actual files on disk, always.

    python3 web/serve.py 8081

`--tls` adds the shape a browser across the LAN needs: HTTPS on every
interface. WebGPU lives in secure contexts only — `navigator.gpu`
exists on `localhost` and over TLS, and a LAN address in plain http
offers neither, so another machine's browser answers any game with
`no suitable GPU adapter` while this Mac's tab runs it happily.

The certificates come from a tiny private CA minted once into
`web/.tls/` and kept on the Mac — its key is the one file that never
leaves. The leaf the server actually presents is signed by that CA,
valid for a couple of years, and re-minted silently whenever it
expires or the machine's LAN address has changed; devices never
notice, because they trust the issuer, not the address.

To make the warnings stop for good on a friendly device, hand it
`web/.tls/ca.pem` (or the DER twin `ca.cer`, for pickier importers)
by any channel — it is the public half, no secret in it; the trust
comes from the owner choosing to install it. Then: iOS installs the
profile and flips Settings → General → About → Certificate Trust
Settings on it; Windows installs it under Trusted Root Certification
Authorities; macOS drops it in the Keychain set to Always Trust.
Green locks for a decade, on every frost address this Mac serves.

    python3 web/serve.py 8443 --tls
"""
import functools
import http.server
import pathlib
import ssl
import subprocess
import sys

FLAGS = {a for a in sys.argv[1:] if a.startswith("--")}
ARGS = [a for a in sys.argv[1:] if not a.startswith("--")]
PORT = int(ARGS[0]) if ARGS else 8081
ROOT = pathlib.Path(__file__).resolve().parent


def lan_addresses() -> list[str]:
    """Every address this machine answers with, not just the one the
    default route would take: a laptop on Wi-Fi and a home subnet on
    the same bridge answers both, and visitors arrive on whichever
    segment they live in. `ifconfig` says; loopback is already a
    standing entry in the certificate."""
    out = subprocess.run(["ifconfig", "-a"], capture_output=True, text=True).stdout
    return sorted({
        word.split("%")[0]
        for line in out.splitlines() if line.startswith("\tinet ")
        for word in line.split()[1:2]
        if not word.startswith("127.")
    })


def openssl(*args: str) -> str:
    done = subprocess.run(["openssl", *args], check=True, capture_output=True, text=True)
    return done.stdout


def make_ca(folder: pathlib.Path):
    """The root, minted once: self-signed, marked a certificate
    authority the way iOS demands — a CA is the only shape Apple's
    full-trust switch lights up for. Key and cert alike stay here."""
    openssl(
        "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "3650",
        "-subj", "/CN=antifrost-ca",
        "-addext", "basicConstraints=critical,CA:TRUE",
        "-addext", "keyUsage=critical,keyCertSign,cRLSign",
        "-keyout", str(folder / "ca-key.pem"), "-out", str(folder / "ca.pem"),
    )
    # The DER twin: Windows' double-click importer only offers the
    # certificate dialog for the extensions it recognises.
    openssl("x509", "-in", str(folder / "ca.pem"), "-outform", "DER",
            "-out", str(folder / "ca.cer"))


def leaf_honest(crt: pathlib.Path, ips: list[str]) -> bool:
    """Whether the presentable leaf still speaks for this machine:
    it must exist, still have days left, and carry every address the
    machine answers with today — browsers match names, not CNs. An
    address that has since vanished may stay on the leaf; a new one
    may not be missing."""
    if not crt.exists():
        return False
    try:
        openssl("x509", "-in", str(crt), "-noout", "-checkend", "0")
        names = openssl("x509", "-in", str(crt), "-noout", "-ext", "subjectAltName")
    except subprocess.CalledProcessError:
        return False
    return all(f"IP Address: {ip}" in names for ip in ips)


def tls_pair(root: pathlib.Path):
    """The pair the server presents, minted as needed. A fresh CA
    forces a fresh leaf — the file on disk might be an older
    self-signed one no CA vouches for."""
    folder = root / ".tls"
    ca_crt, ca_key = folder / "ca.pem", folder / "ca-key.pem"
    crt, key = folder / "cert.pem", folder / "key.pem"
    folder.mkdir(parents=True, exist_ok=True)
    fresh_ca = not (ca_crt.exists() and ca_key.exists())
    if fresh_ca:
        make_ca(folder)
    ip_list = lan_addresses()
    if fresh_ca or not leaf_honest(crt, ip_list):
        openssl("req", "-new", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=antifrost",
                "-keyout", str(key), "-out", str(folder / "leaf.csr"))
        # Browsers match the name from subjectAltName only, and
        # `x509 -req` copies nothing without being told: an extension
        # file is the honest way to sign these in.
        (folder / "leaf.cnf").write_text(
            "subjectAltName=DNS:localhost,IP:127.0.0.1,IP:" + ",IP:".join(ip_list) + "\n"
            "basicConstraints=critical,CA:FALSE\n"
            "keyUsage=critical,digitalSignature,keyEncipherment\n"
            "extendedKeyUsage=serverAuth\n"
        )
        openssl("x509", "-req", "-in", str(folder / "leaf.csr"),
                "-CA", str(ca_crt), "-CAkey", str(ca_key), "-CAcreateserial",
                "-days", "825", "-sha256",
                "-extfile", str(folder / "leaf.cnf"), "-out", str(crt))
    return str(crt), str(key)


class NoStore(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cache-Control", "no-store, max-age=0")
        super().end_headers()


if __name__ == "__main__":
    handler = functools.partial(NoStore, directory=str(ROOT))
    if "--tls" in FLAGS:
        crt, key = tls_pair(ROOT)
        # Every interface, not just localhost: the whole point is the
        # browser on the other machine.
        server = http.server.ThreadingHTTPServer(("0.0.0.0", PORT), handler)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(crt, key)
        server.socket = context.wrap_socket(server.socket, server_side=True)
        where = ", ".join(f"https://{ip}:{PORT}" for ip in lan_addresses())
        print(f"serving {ROOT} at https://localhost:{PORT} and {where} "
              f"(no-store; devices trust web/.tls/ca.pem)")
    else:
        print(f"serving {ROOT} at http://localhost:{PORT} (no-store)")
        server = http.server.ThreadingHTTPServer(("localhost", PORT), handler)
    server.serve_forever()
