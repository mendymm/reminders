build-release:
    #!/usr/bin/env fish
    if not set -q REMINDERS_RELEASE_KEYSTORE; or test -z "$REMINDERS_RELEASE_KEYSTORE"
        read -P "Absolute path to release .p12: " release_keystore
        set -lx REMINDERS_RELEASE_KEYSTORE "$release_keystore"
    end

    if not string match --quiet --regex '^/' -- "$REMINDERS_RELEASE_KEYSTORE"
        echo "Release keystore path must be absolute" >&2
        exit 2
    end
    if not test -f "$REMINDERS_RELEASE_KEYSTORE"
        echo "Release keystore was not found: $REMINDERS_RELEASE_KEYSTORE" >&2
        exit 2
    end
    if not test -r "$REMINDERS_RELEASE_KEYSTORE"
        echo "Release keystore is not readable: $REMINDERS_RELEASE_KEYSTORE" >&2
        exit 2
    end

    if not set -q REMINDERS_RELEASE_KEY_ALIAS; or test -z "$REMINDERS_RELEASE_KEY_ALIAS"
        set -lx REMINDERS_RELEASE_KEY_ALIAS reminders-release
    end

    if not set -q REMINDERS_RELEASE_KEYSTORE_PASSWORD; or test -z "$REMINDERS_RELEASE_KEYSTORE_PASSWORD"
        read -s -P "Keystore password: " release_keystore_password
        echo
        set -lx REMINDERS_RELEASE_KEYSTORE_PASSWORD "$release_keystore_password"
    end

    cd android
    exec ./gradlew --no-daemon --no-configuration-cache \
        :app:assembleRelease \
        -PrustAbis=arm64-v8a
