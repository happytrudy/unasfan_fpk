.PHONY: all build verify clean

all: build

build:
	./scripts/build.sh

verify:
	./scripts/verify.sh

clean:
	rm -f dist/*.fpk dist/*.fpk.sha256
