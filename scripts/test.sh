#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."
compiler_directory="$(dirname "$(xcrun --find swiftc)")"
testing_plugin="$compiler_directory/../lib/swift/host/plugins/testing/libTestingMacros.dylib"
if [ -f "$testing_plugin" ]; then
    swift test -Xswiftc -load-plugin-library -Xswiftc "$testing_plugin" "$@"
else
    swift test "$@"
fi
