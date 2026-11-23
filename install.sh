#!/bin/sh
# Builds Softcore and installs it for everyone on this machine: the program in
# /usr/local/bin, the session's files, and the entry that puts Softcore in the login
# manager's list of sessions.
#
#   ./install.sh               build and install (asks for your password to install)
#   ./install.sh --uninstall   take it all away again
set -e
cd "$(dirname "$0")"

if [ "$1" = "--uninstall" ]; then
    program=/usr/local/bin/softcore
    [ -x "$program" ] || program=./target/release/softcore
    [ -x "$program" ] || { echo "install.sh: there is no Softcore here to ask; build one first (cargo build --release)" >&2; exit 1; }
    exec sudo "$program" uninstall --system
fi
[ -z "$1" ] || { sed -n '2,7p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }

command -v cargo >/dev/null || { echo "install.sh: cargo isn't installed (https://rustup.rs)" >&2; exit 1; }
# Built as you, so the build's files stay yours; only the copying is done as root.
cargo build --release
exec sudo ./target/release/softcore install --system
