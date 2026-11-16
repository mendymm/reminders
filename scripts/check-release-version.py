#!/usr/bin/env python3
"""Check that a release tag matches the app's version.

Usage:
    scripts/check-release-version.py vX.Y.Z [--apk APK [--aapt2 AAPT2]]

The version lives in `[workspace.package] version` of rust/Cargo.toml. The Gradle
build reads versionName from it and derives versionCode from it, so the tag only
has to match that one file. With --apk the built artifact is checked as well:
`aapt2 dump badging` must report the tag's version as versionName and the derived
versionCode.

Needs Python 3.11+ (tomllib) and, for --apk, aapt2 on PATH or given by --aapt2
(it ships in the Android SDK build-tools).
"""

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

DEFAULT_MANIFEST = Path(__file__).resolve().parent.parent / "rust" / "Cargo.toml"
TAG_PATTERN = re.compile(r"v(\d+)\.(\d+)\.(\d+)")


class CheckError(Exception):
    pass


def parse_tag(tag: str) -> tuple[str, int]:
    """Return (version, versionCode) for a vMAJOR.MINOR.PATCH tag."""
    match = TAG_PATTERN.fullmatch(tag)
    if not match:
        raise CheckError(f"tag {tag!r} is not of the form vMAJOR.MINOR.PATCH")
    major, minor, patch = (int(part) for part in match.groups())
    # Keep in sync with appVersionCode in android/app/build.gradle.kts.
    if minor >= 100 or patch >= 100:
        raise CheckError(f"tag {tag!r}: minor and patch must be below 100 to derive versionCode")
    return tag[1:], major * 10_000 + minor * 100 + patch


def read_manifest_version(manifest: Path) -> str:
    try:
        with open(manifest, "rb") as handle:
            data = tomllib.load(handle)
    except OSError as error:
        raise CheckError(f"cannot read {manifest}: {error}") from error
    except tomllib.TOMLDecodeError as error:
        raise CheckError(f"{manifest} is not valid TOML: {error}") from error
    version = data.get("workspace", {}).get("package", {}).get("version")
    if not isinstance(version, str):
        raise CheckError(f"{manifest} has no [workspace.package] version")
    return version


def badging_package_line(aapt2: str, apk: Path) -> str:
    result = subprocess.run([aapt2, "dump", "badging", str(apk)], capture_output=True, text=True, check=False)
    if result.returncode != 0:
        raise CheckError(f"`aapt2 dump badging` failed for {apk}: {result.stderr.strip()}")
    for line in result.stdout.splitlines():
        if line.startswith("package:"):
            return line
    raise CheckError(f"`aapt2 dump badging` printed no package line for {apk}")


def check_apk(aapt2: str, apk: Path, version: str, version_code: int) -> None:
    line = badging_package_line(aapt2, apk)
    # The leading space keeps platformBuildVersionName from matching.
    name = re.search(r" versionName='([^']*)'", line)
    code = re.search(r" versionCode='(\d+)'", line)
    if not name or not code:
        raise CheckError(f"no versionName/versionCode in the package line of {apk}: {line}")
    if name.group(1) != version:
        raise CheckError(f"{apk} has versionName {name.group(1)!r}, expected {version!r}")
    if int(code.group(1)) != version_code:
        raise CheckError(f"{apk} has versionCode {code.group(1)}, expected {version_code}")


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Check that a release tag matches the app version.")
    parser.add_argument("tag", help="the tag, for example v0.1.0")
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST, help="Cargo workspace manifest")
    parser.add_argument("--apk", type=Path, help="also check this built APK with aapt2")
    parser.add_argument("--aapt2", default="aapt2", help="aapt2 binary for --apk (default: aapt2 from PATH)")
    return parser.parse_args()


def main() -> int:
    arguments = parse_arguments()
    try:
        version, version_code = parse_tag(arguments.tag)
        manifest_version = read_manifest_version(arguments.manifest)
        if manifest_version != version:
            raise CheckError(f"tag {arguments.tag} does not match version {manifest_version} in {arguments.manifest}")
        print(f"{arguments.manifest.name} version {manifest_version} matches tag {arguments.tag}")
        if arguments.apk:
            aapt2 = shutil.which(arguments.aapt2)
            if not aapt2:
                raise CheckError(f"`{arguments.aapt2}` was not found on PATH")
            check_apk(aapt2, arguments.apk, version, version_code)
            print(f"{arguments.apk} has versionName {version} and versionCode {version_code}")
    except CheckError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
