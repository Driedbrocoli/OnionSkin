#!/bin/sh
# Install Onionskin on Linux or macOS, in one line:
#
#     curl -fsSL https://raw.githubusercontent.com/Driedbrocoli/OnionSkin/main/install.sh | sh
#
# It works out which machine this is, fetches the right archive from the
# newest release, unpacks it somewhere temporary, and runs `onionskin install`
# — which copies the program into your own account. Nothing here uses sudo,
# and nothing here needs it.
#
# If you would rather not pipe a script from the internet into a shell — a
# reasonable thing to prefer — the README has the two commands this does,
# written out. Read this file first if you like; it is fifty lines.

set -eu

REPO="Driedbrocoli/OnionSkin"
LATEST="https://github.com/$REPO/releases/latest/download"

say() { printf '%s\n' "$*"; }
stop() { printf '%s\n' "$*" >&2; exit 1; }

# Which archive is this machine's. The names have no version in them, which is
# what lets this URL be written down once and keep working.
system=$(uname -s)
chip=$(uname -m)
case "$system:$chip" in
    Linux:x86_64|Linux:amd64)   archive="onionskin-linux-x64.tar.gz" ;;
    Darwin:arm64|Darwin:aarch64) archive="onionskin-macos-arm64.tar.gz" ;;
    Darwin:x86_64)              archive="onionskin-macos-x64.tar.gz" ;;
    *)
        stop "There is no ready-made Onionskin for $system on $chip yet.
It builds from source in about five minutes:

    git clone https://github.com/$REPO
    cd OnionSkin && cargo build --release

Full instructions: https://github.com/$REPO#or-build-it-from-source"
        ;;
esac

command -v curl >/dev/null 2>&1 || stop "This needs curl, and there is none on this machine."
command -v tar  >/dev/null 2>&1 || stop "This needs tar, and there is none on this machine."

work=$(mktemp -d)
# Whatever happens next, do not leave a directory behind in /tmp.
trap 'rm -rf "$work"' EXIT INT TERM

say "Fetching $archive…"
curl -fL --progress-bar -o "$work/onionskin.tar.gz" "$LATEST/$archive" \
    || stop "Could not download it. Is there a release yet?
Look at https://github.com/$REPO/releases"

tar -xzf "$work/onionskin.tar.gz" -C "$work"

# The archive holds a folder with the programs in it; find whichever it is
# rather than assuming the name, which has the version in it.
program=$(find "$work" -type f -name onionskin -perm -u+x 2>/dev/null | head -n 1)
[ -n "$program" ] || stop "That archive does not look like Onionskin."

say ""
"$program" install
