#!/usr/bin/env python3
"""Sign an unsigned release APK with the release key stored in 1Password.

Usage:
    scripts/sign-release.py <unsigned.zip> <signed.apk> [--alias ALIAS] [--force]

The release keystore (.p12) and its password are read from 1Password with
`op read`. The keystore is written to a private temporary directory (mode 0700,
on /dev/shm when available) that is overwritten and removed when the script
finishes, including on errors, Ctrl-C, and SIGTERM/SIGHUP. The password is only
held in memory and is handed to apksigner through its environment, never on a
command line. Nothing secret is printed.

Run it where the `op` CLI is signed in and `apksigner` is available, either on
PATH or in $ANDROID_HOME/build-tools/*/ (as in `nix develop`). Only the Python
standard library is used.
"""

from __future__ import annotations

import argparse
import os
import shutil
import signal
import subprocess
import sys
import tempfile
from pathlib import Path

# Note the space before the slash: it is part of the item title in 1Password.
KEYSTORE_REFERENCE = "op://Personal/reminders app android release keystore /reminders-release.p12"
PASSWORD_REFERENCE = "op://Personal/reminders app android release keystore /p12 enc password"
DEFAULT_ALIAS = "reminders-release"
PASSWORD_ENV_VAR = "REMINDERS_KEYSTORE_PASSWORD"
# Keep in sync with minSdk in android/app/build.gradle.kts. It is passed to apksigner
# explicitly instead of letting it read the manifest.
MIN_SDK_VERSION = "37"
# apksigner aligns native libraries to this many bytes (16 KiB pages).
LIB_PAGE_ALIGNMENT = "16384"


class SigningError(Exception):
    pass


def find_tool(name: str, extra_candidates: list[Path] | None = None) -> str:
    found = shutil.which(name)
    if found:
        return found
    for candidate in extra_candidates or []:
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)
    raise SigningError(f"`{name}` was not found on PATH")


def apksigner_candidates() -> list[Path]:
    candidates: list[Path] = []
    for variable in ("ANDROID_HOME", "ANDROID_SDK_ROOT"):
        root = os.environ.get(variable)
        if root:
            build_tools = Path(root) / "build-tools"
            if build_tools.is_dir():
                versions = sorted(build_tools.iterdir(), reverse=True)
                candidates.extend(version / "apksigner" for version in versions)
    return candidates


def private_temp_root() -> str | None:
    """Prefer RAM-backed storage so key material never reaches a disk."""
    shm = Path("/dev/shm")
    if shm.is_dir() and os.access(shm, os.W_OK):
        return str(shm)
    return None


def overwrite_and_remove(path: Path) -> None:
    try:
        size = path.stat().st_size
        with open(path, "r+b", buffering=0) as handle:
            remaining = size
            chunk = b"\0" * 65536
            while remaining > 0:
                written = handle.write(chunk[: min(remaining, len(chunk))])
                remaining -= written
            os.fsync(handle.fileno())
    except OSError:
        pass
    try:
        path.unlink()
    except OSError:
        pass


def clean_up(workdir: Path) -> None:
    if workdir.exists():
        for secret in workdir.glob("*.p12"):
            overwrite_and_remove(secret)
        shutil.rmtree(workdir, ignore_errors=True)
    if workdir.exists():
        print(f"WARNING: could not remove temporary directory {workdir}", file=sys.stderr)
    else:
        print("Temporary files removed.", file=sys.stderr)


def run_op(op: str, arguments: list[str], description: str) -> subprocess.CompletedProcess[bytes]:
    result = subprocess.run([op, *arguments], capture_output=True, check=False)
    if result.returncode != 0:
        detail = result.stderr.decode(errors="replace").strip()
        raise SigningError(f"`op read` failed for the {description}: {detail}")
    return result


def sign(unsigned: Path, destination: Path, alias: str) -> None:
    op = find_tool("op")
    apksigner = find_tool("apksigner", apksigner_candidates())

    workdir = Path(tempfile.mkdtemp(prefix="reminders-sign-", dir=private_temp_root()))
    try:
        keystore = workdir / "release.p12"
        signed_tmp = workdir / "signed.apk"

        print("Fetching the keystore from 1Password...", file=sys.stderr)
        run_op(
            op,
            ["read", "--force", "--file-mode", "0600", "--out-file", str(keystore), KEYSTORE_REFERENCE],
            "keystore",
        )

        print("Fetching the keystore password from 1Password...", file=sys.stderr)
        password = run_op(op, ["read", "--no-newline", PASSWORD_REFERENCE], "password").stdout.decode()

        environment = {**os.environ, PASSWORD_ENV_VAR: password}
        print(f"Signing {unsigned}...", file=sys.stderr)
        signed = subprocess.run(
            [
                apksigner,
                "sign",
                "--verbose",
                # Signing key.
                "--ks", str(keystore),
                "--ks-type", "PKCS12",
                "--ks-key-alias", alias,
                "--ks-pass", f"env:{PASSWORD_ENV_VAR}",
                "--key-pass", f"env:{PASSWORD_ENV_VAR}",
                # Signature schemes: minSdk 37 needs neither v1 (JAR) nor v4 (incremental install).
                "--min-sdk-version", MIN_SDK_VERSION,
                "--v1-signing-enabled", "false",
                "--v2-signing-enabled", "true",
                "--v3-signing-enabled", "true",
                "--v4-signing-enabled", "false",
                "--verity-enabled", "false",
                # Alignment: always (re)align, with 16 KiB native-library pages.
                "--alignment-preserved", "false",
                "--lib-page-alignment", LIB_PAGE_ALIGNMENT,
                # Never sign a debuggable APK with the production key.
                "--debuggable-apk-permitted", "false",
                "--out", str(signed_tmp),
                str(unsigned),
            ],
            env=environment,
            check=False,
        )
        del password, environment
        if signed.returncode != 0:
            raise SigningError("apksigner sign failed")

        print("Verifying the signature...", file=sys.stderr)
        verified = subprocess.run(
            [
                apksigner,
                "verify",
                "--verbose",
                "--print-certs",
                "--min-sdk-version", MIN_SDK_VERSION,
                str(signed_tmp),
            ],
            check=False,
        )
        if verified.returncode != 0:
            raise SigningError("apksigner verify failed")

        shutil.move(str(signed_tmp), destination)
        print(f"Wrote {destination}", file=sys.stderr)
    finally:
        clean_up(workdir)


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Sign an unsigned release APK with the key in 1Password.")
    parser.add_argument("unsigned", type=Path, help="unsigned APK (for example reminders-release-UNSIGNED.zip)")
    parser.add_argument("signed", type=Path, help="where to write the signed .apk")
    parser.add_argument(
        "--alias",
        default=os.environ.get("REMINDERS_RELEASE_KEY_ALIAS", DEFAULT_ALIAS),
        help=f"key alias in the keystore (default: $REMINDERS_RELEASE_KEY_ALIAS or {DEFAULT_ALIAS})",
    )
    parser.add_argument("--force", action="store_true", help="overwrite the output file if it exists")
    return parser.parse_args()


def raise_system_exit(signum: int, _frame: object) -> None:
    # Turn termination signals into an exception so the cleanup in `finally` runs.
    raise SystemExit(128 + signum)


def main() -> int:
    os.umask(0o077)
    for signum in (signal.SIGTERM, signal.SIGHUP):
        signal.signal(signum, raise_system_exit)

    arguments = parse_arguments()
    try:
        if not arguments.unsigned.is_file():
            raise SigningError(f"input file not found: {arguments.unsigned}")
        if arguments.signed.resolve() == arguments.unsigned.resolve():
            raise SigningError("the output must be a different file than the input")
        if arguments.signed.exists() and not arguments.force:
            raise SigningError(f"{arguments.signed} already exists (use --force to overwrite)")
        if not arguments.signed.parent.is_dir():
            raise SigningError(f"output directory does not exist: {arguments.signed.parent}")
        sign(arguments.unsigned, arguments.signed, arguments.alias)
    except SigningError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
