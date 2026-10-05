SHELL := /bin/sh

BIN      := devdoctor
CARGO    ?= cargo

.PHONY: all build release test fmt lint check run install clean help

all: check

## help: Print available targets
help:
	@echo "Available targets:"
	@grep -E '^## ' Makefile | sed -E 's/## /  /'

## build: Compile in debug mode
build:
	$(CARGO) build

## release: Compile a release binary
release:
	$(CARGO) build --release

## test: Run all tests
test:
	$(CARGO) test

## fmt: Format the source code
fmt:
	$(CARGO) fmt

## lint: Run clippy with warnings as errors
lint:
	$(CARGO) clippy --all-targets --all-features -- -D warnings

## check: Run fmt-check, clippy, and tests
check:
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --all-targets --all-features -- -D warnings
	$(CARGO) test

## run: Run the CLI in debug mode
run:
	$(CARGO) run --

## install: Install the binary from current source
install:
	$(CARGO) install --path .

## clean: Remove build artifacts
clean:
	$(CARGO) clean
