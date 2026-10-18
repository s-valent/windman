build:
    bash scripts/build-app.sh

run: build
    open build/Windman.app

test:
    bash scripts/test.sh

fix:
    xcrun swift-format format --in-place --recursive Sources Tests Package.swift

check:
    xcrun swift-format lint --strict --recursive Sources Tests Package.swift
