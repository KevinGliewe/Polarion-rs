#!/bin/sh
# Entry point of the polarion-cli and polarion-mcp images: apply the profile POLARION_PROFILE
# names, then run the command (`polarion`, in the polarion-mcp image `polarion mcp`, followed by
# the container's arguments). A profile sees those arguments as "$@".
#
# POLARION_PROFILE is the name of a file in /etc/polarion/profiles (without `.sh`), a path to a
# profile of your own (mounted into the container), or empty for none. A profile is a shell
# fragment: it sets defaults for variables the container was not given (`${VAR-default}`: a
# variable given empty counts as given), and may unset variables to enforce its scenario
# (read-only unsets the write scope).
set -eu

profiles=/etc/polarion/profiles
available() {
    for f in "$profiles"/*.sh; do
        [ -e "$f" ] && basename "$f" .sh
    done | tr '\n' ' '
}

case "${POLARION_PROFILE:-}" in
"") ;;
*/*)
    if [ ! -f "$POLARION_PROFILE" ]; then
        echo "POLARION_PROFILE: no file $POLARION_PROFILE" >&2
        exit 64
    fi
    . "$POLARION_PROFILE"
    ;;
*)
    if [ ! -f "$profiles/$POLARION_PROFILE.sh" ]; then
        echo "POLARION_PROFILE: unknown profile '$POLARION_PROFILE'; built in: $(available)" >&2
        exit 64
    fi
    . "$profiles/$POLARION_PROFILE.sh"
    ;;
esac

exec "$@"
