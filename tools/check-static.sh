#!/usr/bin/env bash
# Run the repository's lightweight static checks without installing anything.
set -euo pipefail

# CHECK_STATIC_ROOT is intentionally available to the integration test, which
# supplies a disposable tree containing a malformed shell script. Normal use
# always checks the repository that contains this script.
project_root=${CHECK_STATIC_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}
cd "$project_root"

shopt -s nullglob
shell_scripts=(tools/*.sh)
python_scripts=(tools/*.py)
qml_files=(packaging/plasma/org.jambalinux.soniccore/contents/ui/*.qml)

for script in "${shell_scripts[@]}"; do
    bash -n "$script"
done

if ((${#shell_scripts[@]})) && command -v shellcheck >/dev/null 2>&1; then
    shellcheck "${shell_scripts[@]}"
elif ! command -v shellcheck >/dev/null 2>&1; then
    printf 'warning: shellcheck is not installed; skipping shellcheck\n' >&2
fi

if ((${#python_scripts[@]})) && command -v python3 >/dev/null 2>&1; then
    pycache_dir=$(mktemp -d "${TMPDIR:-/tmp}/jambalinux-soniccore-pycache.XXXXXX")
    trap 'rm -rf "$pycache_dir"' EXIT
    PYTHONPYCACHEPREFIX=$pycache_dir python3 -m py_compile "${python_scripts[@]}"
elif ((${#python_scripts[@]})); then
    printf 'warning: python3 is not installed; skipping Python syntax checks\n' >&2
fi

if ((${#qml_files[@]})); then
    if command -v qmllint >/dev/null 2>&1; then
        # Qt 6 can treat unavailable Plasma/Kirigami imports as warnings and
        # turn those environment-specific warnings into a failing exit code.
        # Suppress only that category when this qmllint supports it; syntax and
        # all other enabled checks still fail this script.
        qml_help=$(qmllint --help-all 2>&1 || true)
        if [[ $qml_help == *--import* ]]; then
            qmllint --import disable "${qml_files[@]}"
        else
            # Older qmllint releases cannot distinguish unavailable Plasma
            # imports from source diagnostics and may fail without output.
            # Do not turn that host limitation into a false project failure.
            printf 'warning: qmllint lacks import-diagnostic controls; skipping QML lint\n' >&2
        fi
    else
        printf 'warning: qmllint is not installed; skipping QML lint\n' >&2
    fi
fi
