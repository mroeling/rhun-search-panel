#!/bin/sh
# Builds the extension for this machine, lays it out as rhun installs it, and runs tests/basic.txt
# headlessly against a rhun binary. usage: tests/run.sh [RHUN]; UPDATE=1 writes tests/basic.expected
set -e
cd "$(dirname "$0")/.."
RHUN=${1:-$(command -v rhun || echo "$HOME/.local/bin/rhun")}
name=$(sed -n 's/^name *= *//p' extension.ini | head -1)
owner=$(sed -n 's/^owner *= *//p' extension.ini | head -1)
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) target=linux-x86_64 ;;
    Darwin-arm64) target=macos-arm64 ;;
    *) echo "no target for $(uname -s)-$(uname -m)"; exit 77 ;;
esac
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
dir=$work/exts/$owner/$name
mkdir -p "$dir/bin" "$work/proj/src/ui" "$work/config/rhun" "$work/state"
sh tests/layout.sh "$dir"  # what the release ships: the manifest, the icon
cargo build --release --quiet
cp "target/release/$name" "$dir/bin/$name-$target"
# the project: a file at the top and two under src/, with "hello" in several places
printf 'hello in root\n' > "$work/proj/root.txt"
printf 'fn main() {\n    let Hello = 1; // hello\n}\n' > "$work/proj/src/main.rs"
printf 'Hello there\nhello again\nno match\n' > "$work/proj/src/ui/x.txt"
printf '[files]\nrestore_session = false\nrestore_project = false\n[updates]\ncheck = false\n[git]\nenabled = false\n[ui]\nsidebar = true\nagents_panel = true\n[editor]\ncursor_blink = false\n' > "$work/config/rhun/config"
run() { # SCRIPT: runs it in a fresh editor, the output on stdout
    HOME=$work XDG_CONFIG_HOME=$work/config XDG_STATE_HOME=$work/state \
        "$RHUN" "$work/proj" --headless 1000x600 --scale 1 --extensions "$work/exts" --script "$1" 2>&1
}
sed "s|proj/|$work/proj/|" tests/basic.txt > "$work/basic.txt"
# the rows and fields are drawn where the platform's font puts them: the output keeps what the
# search finds, not where it is drawn; $work is a fresh folder each run
run "$work/basic.txt" | sed -e 's/pid=[1-9][0-9]*/pid=PID/' -e 's/ at=[0-9,]*//g' -e 's/^at=[0-9,]*/at=X/' -e 's/ size=[0-9x]*//g' \
    -e 's/ y=[0-9]*//g' -e 's/ scroll=[0-9]*//g' -e 's/ rowh=[0-9]*//g' -e 's/ confined=2/ confined=1/' \
    -e "s|$work|WORK|g" > "$work/out.txt"
[ -z "$UPDATE" ] || cp "$work/out.txt" tests/basic.expected  # UPDATE=1 writes the expected output
if cmp -s "$work/out.txt" tests/basic.expected; then
    echo "ok   basic"
else
    echo "FAIL basic"; diff tests/basic.expected "$work/out.txt" || true; exit 1
fi
