.PHONY: all build check test clean docker docker-dev

all: build

build:
	cargo build --release

check:
	cargo check

test:
	cargo test

clean:
	cargo clean

docker:
	docker build -t codasaurus:latest .

docker-dev:
	docker compose up --build
