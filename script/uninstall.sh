#!/usr/bin/env sh
set -eu

# Uninstalls Flint that was installed using the install.sh script.
# Flint is only built for macOS.

app_name="Flint.app"
bundle_identifier="io.github.feodosiishus.Flint"
install_dir="${FLINT_INSTALL_DIR:-/Applications}"

platform="$(uname -s)"
if [ "$platform" != "Darwin" ]; then
    echo "Flint is only built for macOS; this system is $platform" >&2
    exit 1
fi

if [ -d "$install_dir/$app_name" ]; then
    rm -rf "$install_dir/$app_name"
fi

# The in-app Install CLI action links the helper here.
rm -f /usr/local/bin/flint

rm -rf "$HOME/Library/Application Support/com.apple.sharedfilelist/com.apple.LSSharedFileList.ApplicationRecentDocuments/$bundle_identifier.sfl"*
rm -rf "$HOME/Library/Caches/$bundle_identifier"
rm -rf "$HOME/Library/HTTPStorages/$bundle_identifier"
rm -rf "$HOME/Library/Preferences/$bundle_identifier.plist"
rm -rf "$HOME/Library/Saved Application State/$bundle_identifier.savedState"

printf "Do you want to keep your Flint preferences and logs? [Y/n] "
read -r response
case "$response" in
    [nN]|[nN][oO])
        rm -rf "$HOME/Library/Application Support/Flint"
        rm -rf "$HOME/Library/Logs/Flint"
        rm -rf "$HOME/.config/flint"
        echo "Preferences removed."
        ;;
    *)
        echo "Preferences kept."
        ;;
esac

echo "Flint has been uninstalled"
