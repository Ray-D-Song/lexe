#!/bin/sh
set -eu

repo=Ray-D-Song/lexe
case "$(uname -m)" in
  x86_64|amd64) arch=x64 ;; arm64|aarch64) arch=arm64 ;; *) echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;; esac
install_dir=${LEXE_INSTALL_DIR:-"$HOME/.local/bin"}
asset="lexe-macos-$arch"
base="https://github.com/$repo/releases/latest/download"
target="$install_dir/lexe"

if [ -L "$target" ] || { [ -e "$target" ] && ! "$target" --version 2>/dev/null | grep -q '^lexe v'; }; then
  echo "Refusing to replace non-Lexe file: $target" >&2; exit 1
fi
mkdir -p "$install_dir"
tmp=$(mktemp -d "$install_dir/.lexe.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
curl -fsSL "$base/$asset" -o "$tmp/$asset"
curl -fsSL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS"
(cd "$tmp" && grep "  $asset$" SHA256SUMS | shasum -a 256 -c -)
chmod 755 "$tmp/$asset"
mv -f "$tmp/$asset" "$target"

path_line="export PATH=\"$install_dir:\$PATH\" # Lexe"
case "${SHELL##*/}" in
  bash) profile="$HOME/.bashrc" ;;
  zsh) profile="$HOME/.zshrc" ;;
  fish) profile="$HOME/.config/fish/conf.d/lexe.fish"; path_line="set -gx PATH \"$install_dir\" \$PATH # Lexe" ;;
  *) profile="$HOME/.profile" ;;
esac
mkdir -p "$(dirname "$profile")"
touch "$profile"
grep -Fqx "$path_line" "$profile" || printf '%s\n' "$path_line" >> "$profile"
echo "Installed Lexe to $target. Reopen your terminal or run: exec \"\$SHELL\" -l"
