# Signed arm64 release APK: reproducible unsigned Nix build, then signed outside Nix.
# Key comes from 1Password via `op`, or from REMINDERS_RELEASE_KEYSTORE (+ _PASSWORD) if set.
build-release:
    #!/usr/bin/env fish
    nix shell --inputs-from . nixpkgs#nix-output-monitor --command nom build .#apk-release-arm64 -o result-release; or exit $status
    python3 scripts/sign-release.py result-release/reminders-release-UNSIGNED.zip reminders-release.apk
