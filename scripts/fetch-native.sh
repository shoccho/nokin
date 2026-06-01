#!/usr/bin/env sh
set -eu

SCINTILLA_VERSION=562
LEXILLA_VERSION=548
SCINTILLA_SHA256=7b8345a224d7473b60c23face71ca8efb649c3b970705588911b40b505a0b10d
LEXILLA_SHA256=742909e4f9c9d23ad2c4239185bf37977f35b0fb118daf52c1d0bcf7f8a79f29

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
VENDOR_DIR=${VENDOR_DIR:-"$ROOT/vendor"}

if [ -f "$VENDOR_DIR/scintilla/version.txt" ] &&
    [ "$(cat "$VENDOR_DIR/scintilla/version.txt")" = "$SCINTILLA_VERSION" ] &&
    [ -f "$VENDOR_DIR/lexilla/version.txt" ] &&
    [ "$(cat "$VENDOR_DIR/lexilla/version.txt")" = "$LEXILLA_VERSION" ]; then
    printf '%s\n' "Native sources already present in $VENDOR_DIR"
    exit 0
fi

for command in curl sha256sum tar mktemp; do
    if ! command -v "$command" >/dev/null 2>&1; then
        printf '%s\n' "Missing required command: $command" >&2
        exit 1
    fi
done

TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT HUP INT TERM

fetch() {
    name=$1
    version=$2
    expected=$3
    archive="$TEMP_DIR/$name$version.tgz"
    curl --fail --location --retry 3 \
        --output "$archive" "https://www.scintilla.org/$name$version.tgz"
    printf '%s  %s\n' "$expected" "$archive" | sha256sum --check -
    tar -xzf "$archive" -C "$TEMP_DIR"
}

fetch scintilla "$SCINTILLA_VERSION" "$SCINTILLA_SHA256"
fetch lexilla "$LEXILLA_VERSION" "$LEXILLA_SHA256"

mkdir -p "$VENDOR_DIR"
rm -rf "$VENDOR_DIR/scintilla" "$VENDOR_DIR/lexilla"
mv "$TEMP_DIR/scintilla" "$TEMP_DIR/lexilla" "$VENDOR_DIR/"
printf '%s\n' "Fetched Scintilla 5.6.2 and Lexilla 5.4.8 into $VENDOR_DIR"
