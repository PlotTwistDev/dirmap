#!/usr/bin/env bash
# Packs a release build of Filescape into the Linux downloads:
#
#   Filescape-linux.tar.gz    the program, icon and desktop entry, for any distribution
#   Filescape-linux.AppImage  one file that runs on any distribution, no install needed
#   Filescape-linux.deb       Ubuntu, Debian, Linux Mint, Pop!_OS and other apt systems
#   Filescape-linux.rpm       Fedora, RHEL, openSUSE and other rpm systems
#
#   packaging/linux/package.sh target/release/filescape dist
#
# Needs dpkg-deb, rpmbuild (the `rpm` package on Ubuntu) and curl. The binary is built on an
# older Ubuntu (22.04) so it runs on any distribution with glibc 2.35 or later; nothing else is
# bundled, because the graphics and window libraries it loads come with every desktop install.
set -euo pipefail

bin=$(realpath "$1")
out=$(mkdir -p "$2" && realpath "$2")
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
version=$(grep -m1 '^version' "$repo/Cargo.toml" | cut -d'"' -f2)
# The release date: the commit being built, like the app's own update check.
epoch=${SOURCE_DATE_EPOCH:-$(git -C "$repo" log -1 --format=%ct 2>/dev/null || date +%s)}
date=$(date -u -d "@$epoch" +%Y-%m-%d)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# appimagetool, pinned and checked.
APPIMAGETOOL_URL=https://github.com/AppImage/appimagetool/releases/download/1.9.0/appimagetool-x86_64.AppImage
APPIMAGETOOL_SHA256=46fdd785094c7f6e545b61afcfb0f3d98d8eab243f644b4b17698c01d06083d1

# ---------- The installed layout, shared by the .deb, the .rpm and the AppImage ----------
root=$work/root
install -Dm755 "$bin" "$root/usr/bin/filescape"
install -Dm644 "$repo/packaging/filescape.desktop" "$root/usr/share/applications/filescape.desktop"
install -Dm644 "$repo/packaging/filescape.svg" "$root/usr/share/icons/hicolor/scalable/apps/filescape.svg"
for s in 128 256 512; do
  install -Dm644 "$here/icons/filescape-$s.png" "$root/usr/share/icons/hicolor/${s}x$s/apps/filescape.png"
done
install -d "$root/usr/share/metainfo"
sed -e "s/@VERSION@/$version/" -e "s/@DATE@/$date/" "$here/app.filescape.Filescape.metainfo.xml" \
  > "$root/usr/share/metainfo/app.filescape.Filescape.metainfo.xml"
install -Dm644 "$repo/README.md" "$root/usr/share/doc/filescape/README.md"

# ---------- tar.gz (unchanged from before: a folder with the program in it) ----------
mkdir "$work/filescape"
cp "$bin" "$repo/README.md" "$repo/packaging/filescape.desktop" "$repo/packaging/filescape.png" "$work/filescape/"
tar -C "$work" --owner=0 --group=0 -czf "$out/Filescape-linux.tar.gz" filescape

# ---------- .deb ----------
# The oldest glibc the binary asks for, so apt refuses it on a system too old to run it.
glibc=$(objdump -T "$bin" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -1)
deb=$work/deb
cp -a "$root" "$deb"
mkdir "$deb/DEBIAN"
cat > "$deb/DEBIAN/control" <<CONTROL
Package: filescape
Version: $version
Architecture: amd64
Maintainer: Filescape <support@filescape.app>
Installed-Size: $(du -sk "$root" | cut -f1)
Depends: libc6 (>= $glibc), libgcc-s1, libxkbcommon0, libegl1 | libgl1
Recommends: libxkbcommon-x11-0, libx11-6, libxcursor1, libxi6, libxrandr2, libwayland-client0, libwayland-egl1, xdg-desktop-portal
Section: utils
Priority: optional
Homepage: https://filescape.app/
Description: See what is taking up space on your drives
 Filescape scans a drive or folder and draws everything on it as a map,
 where each file is a tile sized by how much space it takes. Filter the
 map, find duplicates, compare snapshots and manage files, including on
 remote servers over SFTP and FTP.
CONTROL
dpkg-deb --root-owner-group -Zxz --build "$deb" "$out/Filescape-linux.deb" >/dev/null

# ---------- .rpm ----------
# Requirements name libraries, not packages, so the same file installs on Fedora and openSUSE;
# rpmbuild adds the libc ones it finds in the binary itself.
mkdir -p "$work/rpm/SPECS"
cat > "$work/rpm/SPECS/filescape.spec" <<SPEC
Name:      filescape
Version:   $version
Release:   1
Summary:   See what is taking up space on your drives
License:   Proprietary
URL:       https://filescape.app/
BuildArch: x86_64
Requires:  libxkbcommon.so.0()(64bit)
Requires:  (libEGL.so.1()(64bit) or libGL.so.1()(64bit))
Recommends: libxkbcommon-x11.so.0()(64bit)
Recommends: libX11.so.6()(64bit)
Recommends: libXcursor.so.1()(64bit)
Recommends: libXi.so.6()(64bit)
Recommends: libXrandr.so.2()(64bit)
Recommends: libwayland-client.so.0()(64bit)
Recommends: libwayland-egl.so.1()(64bit)

# Prebuilt: no debug package, no stripping (the line tables give crash.log its file names).
%global debug_package %{nil}
%global __os_install_post %{nil}
%global _build_id_links none
%global _binary_payload w6.xzdio

%description
Filescape scans a drive or folder and draws everything on it as a map, where each file is a
tile sized by how much space it takes. Filter the map, find duplicates, compare snapshots and
manage files, including on remote servers over SFTP and FTP.

%install
cp -a $root/. %{buildroot}/

%files
/usr/bin/filescape
/usr/share/applications/filescape.desktop
/usr/share/icons/hicolor/*/apps/filescape.*
/usr/share/metainfo/app.filescape.Filescape.metainfo.xml
%doc /usr/share/doc/filescape/README.md
SPEC
rpmbuild -bb --quiet --target x86_64 \
  --define "_topdir $work/rpm" \
  --define "_rpmfilename filescape.rpm" \
  --define "use_source_date_epoch_as_buildtime 1" \
  "$work/rpm/SPECS/filescape.spec"
cp "$work/rpm/RPMS/filescape.rpm" "$out/Filescape-linux.rpm"

# ---------- AppImage ----------
app=$work/Filescape.AppDir
cp -a "$root" "$app"
rm -rf "$app/usr/share/doc"
cp "$repo/packaging/filescape.desktop" "$app/"
cp "$here/icons/filescape-256.png" "$app/filescape.png"
ln -s filescape.png "$app/.DirIcon"
cat > "$app/AppRun" <<'APPRUN'
#!/bin/sh
here=$(dirname "$(readlink -f "$0")")
exec "$here/usr/bin/filescape" "$@"
APPRUN
chmod 755 "$app/AppRun"
tool=$work/appimagetool
curl -fsSL -o "$tool" "$APPIMAGETOOL_URL"
echo "$APPIMAGETOOL_SHA256  $tool" | sha256sum -c --quiet
chmod 755 "$tool"
# No FUSE on build machines: let the tool unpack itself instead of mounting.
ARCH=x86_64 VERSION=$version APPIMAGE_EXTRACT_AND_RUN=1 "$tool" --no-appstream "$app" "$out/Filescape-linux.AppImage" >/dev/null
chmod 755 "$out/Filescape-linux.AppImage"

ls -l "$out"/Filescape-linux.*
