#!/usr/bin/env sh
set -eu

app_name="Flint.app"
bundle_identifier="io.github.feodosiishus.Flint"
dmg_name="Flint-aarch64.dmg"
repository_url="https://github.com/FeodosiiShus/Flint"
quarantine_attribute="com.apple.quarantine"

temp_dir=""
mount_point=""
mounted=false
staging_path=""

log() {
    printf 'flint-install: %s\n' "$*"
}

fail() {
    printf 'flint-install: error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<EOF
Usage: install.sh [path/to/$dmg_name | path/to/flint-macos-aarch64.zip]

Installs $app_name into \${FLINT_INSTALL_DIR:-/Applications} and removes the
$quarantine_attribute attribute from it.

Without an argument the DMG is downloaded from the latest GitHub release, or from
the release tagged \$FLINT_VERSION when that variable is set.
EOF
}

cleanup() {
    exit_status=$?
    trap - EXIT INT TERM HUP
    if [ "$mounted" = true ]; then
        if hdiutil detach -quiet "$mount_point" || hdiutil detach -quiet -force "$mount_point"; then
            mounted=false
        else
            printf 'flint-install: error: could not detach %s; run: hdiutil detach -force "%s"\n' "$mount_point" "$mount_point" >&2
            exit_status=1
        fi
    fi
    if [ -n "$staging_path" ] && [ -e "$staging_path" ]; then
        rm -rf "$staging_path"
    fi
    if [ -n "$temp_dir" ] && [ -d "$temp_dir" ]; then
        if [ "$mounted" = true ]; then
            printf 'flint-install: error: left %s in place because a disk image is still mounted inside it\n' "$temp_dir" >&2
        else
            rm -rf "$temp_dir"
        fi
    fi
    exit "$exit_status"
}

check_platform() {
    platform="$(uname -s)"
    [ "$platform" = "Darwin" ] || fail "Flint is only built for macOS; this system is $platform"
    machine="$(uname -m)"
    if [ "$machine" != "arm64" ] && [ "$(sysctl -in hw.optional.arm64)" != "1" ]; then
        fail "Flint is only built for Apple silicon (arm64); this Mac is $machine"
    fi
    for tool in hdiutil ditto xattr find ps awk; do
        command -v "$tool" >/dev/null 2>&1 || fail "required tool '$tool' was not found in PATH"
    done
}

check_not_running() {
    executable_prefix="$1/Contents/MacOS/"
    running="$(ps -A -o pid=,command= | awk -v prefix="$executable_prefix" '{ pid = $1; sub(/^[ \t]*[0-9]+[ \t]+/, ""); if (index($0, prefix) == 1) print pid }')"
    if [ -n "$running" ]; then
        fail "Flint is running from $1 (pid $(printf '%s' "$running" | tr '\n' ' ' | sed 's/ $//')); quit Flint and run the installer again"
    fi
}

download_dmg() {
    command -v curl >/dev/null 2>&1 || fail "curl was not found in PATH"
    if [ -n "${FLINT_VERSION:-}" ]; then
        url="$repository_url/releases/download/$FLINT_VERSION/$dmg_name"
        log "Downloading Flint $FLINT_VERSION from $url"
    else
        url="$repository_url/releases/latest/download/$dmg_name"
        log "Downloading the latest Flint release from $url"
    fi
    curl --fail --location --show-error --progress-bar --proto '=https' --output "$1" "$url" \
        || fail "download failed: $url"
}

extract_zip() {
    extract_dir="$temp_dir/artifact"
    mkdir "$extract_dir"
    ditto -x -k "$1" "$extract_dir" || fail "could not extract zip archive $1"
    [ -f "$extract_dir/$dmg_name" ] || fail "zip archive $1 does not contain $dmg_name at its top level"
    extracted_dmg="$extract_dir/$dmg_name"
}

resolve_source() {
    source_path="$1"
    [ -e "$source_path" ] || fail "no such file: $source_path"
    [ -f "$source_path" ] || fail "not a regular file: $source_path"
    case "$source_path" in
        *.dmg | *.DMG)
            source_dmg="$source_path"
            ;;
        *.zip | *.ZIP)
            extract_zip "$source_path"
            source_dmg="$extracted_dmg"
            ;;
        *)
            fail "expected a .dmg or a .zip file, got: $source_path"
            ;;
    esac
}

remove_quarantine() {
    xattr -d -r -s "$quarantine_attribute" "$1" || fail "could not remove $quarantine_attribute from $1"
    remaining="$(find "$1" -xattrname "$quarantine_attribute")" || fail "could not scan $1 for $quarantine_attribute"
    if [ -n "$remaining" ]; then
        printf '%s\n' "$remaining" >&2
        fail "$quarantine_attribute is still set on the files above; run: xattr -d -r $quarantine_attribute \"$1\""
    fi
}

main() {
    case "${1:-}" in
        -h | --help)
            usage
            exit 0
            ;;
    esac
    [ "$#" -le 1 ] || {
        usage >&2
        exit 2
    }

    check_platform

    install_dir="${FLINT_INSTALL_DIR:-/Applications}"
    install_dir="${install_dir%/}"
    [ -n "$install_dir" ] || fail "FLINT_INSTALL_DIR must not be /"
    [ -d "$install_dir" ] || fail "install directory does not exist: $install_dir"
    [ -w "$install_dir" ] || fail "cannot write to $install_dir; check its permissions or set FLINT_INSTALL_DIR"
    installed_app="$install_dir/$app_name"

    check_not_running "$installed_app"

    temp_root="${TMPDIR:-/tmp}"
    temp_root="${temp_root%/}"
    trap cleanup EXIT
    trap 'exit 129' HUP
    trap 'exit 130' INT
    trap 'exit 143' TERM
    temp_dir="$(mktemp -d "$temp_root/flint-install.XXXXXX")" || fail "could not create a temporary directory in $temp_root"

    if [ "$#" -eq 1 ]; then
        resolve_source "$1"
    else
        source_dmg="$temp_dir/$dmg_name"
        download_dmg "$source_dmg"
    fi

    mount_point="$temp_dir/mount"
    mkdir "$mount_point"
    log "Mounting $source_dmg"
    hdiutil attach -quiet -nobrowse -readonly -noautoopen -mountpoint "$mount_point" "$source_dmg" \
        || fail "could not mount $source_dmg as a disk image"
    mounted=true

    mounted_app="$mount_point/$app_name"
    [ -d "$mounted_app" ] || fail "the disk image does not contain $app_name"
    plist="$mounted_app/Contents/Info.plist"
    [ -f "$plist" ] || fail "$app_name in the disk image has no Contents/Info.plist"
    found_identifier="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$plist")" \
        || fail "could not read CFBundleIdentifier from $plist"
    [ "$found_identifier" = "$bundle_identifier" ] \
        || fail "the disk image contains $found_identifier, expected $bundle_identifier"

    staging_path="$install_dir/.$app_name.installing.$$"
    log "Copying $app_name to $install_dir"
    ditto "$mounted_app" "$staging_path" || fail "could not copy $app_name to $install_dir"

    hdiutil detach -quiet "$mount_point" || fail "could not detach $mount_point"
    mounted=false

    check_not_running "$installed_app"
    if [ -e "$installed_app" ]; then
        log "Replacing existing $installed_app"
        rm -rf "$installed_app" || fail "could not remove the existing $installed_app"
    fi
    mv "$staging_path" "$installed_app" || fail "could not move the new $app_name into $install_dir"
    staging_path=""

    log "Removing $quarantine_attribute from $installed_app"
    remove_quarantine "$installed_app"

    log "Installed $installed_app"
}

main "$@"
