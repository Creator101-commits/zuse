#!/usr/bin/env bash
# Install Ubuntu packages for CI from a cached directory of .deb files, falling
# back to the mirrors (scripts/ci-ubuntu-dependencies.sh) only when the cache
# is empty. Mirror installs took 10-27 minutes on slow-mirror days; installing
# cached .debs takes seconds. The workflow restores and saves the directory
# with actions/cache, keyed on the package list.
#
# Usage: ci-apt-install.sh <deb-cache-dir> <package>...
set -euo pipefail
cd "$(dirname "$0")/.."

cache=$1
shift
mkdir -p "$cache/partial"

if compgen -G "$cache/*.deb" > /dev/null; then
  # The cache holds everything these packages need beyond the image. dpkg
  # installs in argument order and refuses a package whose Pre-Depends is not
  # yet configured, so repeat the set: each pass configures more of it.
  # (apt-get install of local archives trips an apt bug here.)
  for _ in 1 2 3 4 5; do
    if sudo dpkg -i "$cache"/*.deb > /dev/null 2>&1; then
      sudo dpkg --configure -a
      [ -z "$(dpkg --audit)" ] && exit 0
      break
    fi
  done
  printf '%s\n' 'Cached packages did not install cleanly; falling back to the mirrors.' >&2
  rm -f "$cache"/*.deb
fi

# Keep the downloaded archives in the cache directory for the next run.
APT_ARCHIVES="$cache" bash scripts/ci-ubuntu-dependencies.sh "$@"
sudo chown -R "$(id -u):$(id -g)" "$cache"
