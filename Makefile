.PHONY: all build check test clean docker docker-dev

# Default target
all: build

# Full release build
build:
	cargo build --release

# Quick check
check:
	cargo check

# Run tests
test:
	cargo test

# Clean
clean:
	cargo clean

# Docker build
docker:
	docker build -t codasaurus:latest .

# Development Docker (with hot-reload — mounts source)
docker-dev:
	docker compose up --build
