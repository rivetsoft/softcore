#!/bin/sh
# Installs Softcore for everyone on this machine: the program in /usr/local/bin, the session's
# files, and the entry that puts Softcore in the login manager's list of sessions. In a
# release download it installs the program beside it; in the source code it builds it first.
#
#   ./install.sh               install (asks for your password to install)
#   ./install.sh --uninstall   take it all away again
set -e
cd "$(dirname "$0")"

# Root needs no sudo, and a minimal system may not have it.
as_root() {
    if [ "$(id -u)" = 0 ]; then "$@"; else sudo "$@"; fi
}

if [ "$1" = "--uninstall" ]; then
    for program in /usr/local/bin/softcore ./softcore ./target/release/softcore; do
        if [ -x "$program" ]; then
            as_root "$program" uninstall --system
            exit
        fi
    done
    echo "install.sh: there is no Softcore here to ask; build one first (cargo build --release)" >&2
    exit 1
fi
[ -z "$1" ] || { sed -n '2,7p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }

# The release download: the program is already here.
if [ -x ./softcore ] && [ ! -f Cargo.toml ]; then
    as_root ./softcore install --system
    exit
fi

command -v cargo >/dev/null || { echo "install.sh: cargo isn't installed (https://rustup.rs)" >&2; exit 1; }
# Built as you, so the build's files stay yours; only the copying is done as root.
cargo build --release
as_root ./target/release/softcore install --system
